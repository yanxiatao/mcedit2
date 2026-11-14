//! 保存历史的只读来源：引用事务真实备份，不复制世界目录或把缺失文件生成为空文件。
use crate::{
    entity_editing::{DiskIdentitySource, IdentitySource},
    WorldError,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
#[path = "history_region.rs"]
mod region;
pub(crate) use region::RegionSources;

fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn reaches(
    start: &Option<String>,
    target: &Option<String>,
    edges: impl Iterator<Item = (Option<String>, Option<String>)>,
) -> bool {
    let mut graph = BTreeMap::<Option<String>, BTreeSet<Option<String>>>::new();
    for (before, after) in edges {
        graph.entry(before).or_default().insert(after);
    }
    let mut pending = vec![start.clone()];
    let mut seen = BTreeSet::new();
    while let Some(value) = pending.pop() {
        if &value == target {
            return true;
        }
        if seen.insert(value.clone()) {
            if let Some(next) = graph.get(&value) {
                pending.extend(next.iter().cloned());
            }
        }
    }
    false
}

/// 来源字节在分配前计入批次预算；文件大小变化是读取错误。
pub(crate) fn read_file(
    path: &Path,
    limit: usize,
    reserve: impl FnOnce(usize) -> Result<(), WorldError>,
) -> Result<Option<Vec<u8>>, WorldError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > limit as u64 {
        return Err(invalid(format!(
            "读取来源不是预算内普通文件：{}",
            path.display()
        )));
    }
    reserve(meta.len() as usize)?;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(meta.len() + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != meta.len() {
        return Err(invalid(format!(
            "来源文件在读取时大小改变：{}",
            path.display()
        )));
    }
    Ok(Some(bytes))
}

/// 借用同一目录语义，供活动事务视图与长期历史来源共同使用。
pub(crate) struct Layout<'a> {
    pub root: &'a Path,
    pub artifacts: &'a BTreeSet<PathBuf>,
    pub absent_dirs: &'a BTreeSet<PathBuf>,
}
impl Layout<'_> {
    pub fn checked(&self, path: &Path) -> Result<PathBuf, WorldError> {
        if !path.is_absolute()
            || !path.starts_with(self.root)
            || path.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            return Err(invalid("历史来源路径越界"));
        }
        let mut parent = path
            .parent()
            .ok_or_else(|| invalid("历史来源路径没有父目录"))?;
        loop {
            if self.absent_dirs.contains(parent) {
                break;
            }
            match fs::canonicalize(parent) {
                Ok(real) if real == parent => break,
                Ok(_) => return Err(invalid("历史来源目录包含链接或路径变化")),
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && parent != self.root =>
                {
                    parent = parent
                        .parent()
                        .ok_or_else(|| invalid("历史来源父目录越界"))?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(path.to_owned())
    }
    pub fn hidden(&self, path: &Path) -> bool {
        self.artifacts.contains(path)
            || self
                .absent_dirs
                .iter()
                .any(|directory| path.starts_with(directory))
    }
    pub fn is_directory<T>(
        &self,
        path: &Path,
        files: &BTreeMap<PathBuf, Option<T>>,
    ) -> Result<bool, WorldError> {
        let path = self.checked(path)?;
        if self.hidden(&path) || files.contains_key(&path) {
            return Ok(false);
        }
        DiskIdentitySource.is_directory(&path)
    }
    pub fn directory_members<T>(
        &self,
        path: &Path,
        files: &BTreeMap<PathBuf, Option<T>>,
    ) -> Result<Option<Vec<String>>, WorldError> {
        let path = self.checked(path)?;
        if self.hidden(&path) {
            return Ok(None);
        }
        let Some(actual) = DiskIdentitySource.directory_members(&path)? else {
            return Ok(None);
        };
        let mut names = BTreeSet::new();
        for name in actual {
            let child = path.join(&name);
            if self.hidden(&child) || files.get(&child).is_some_and(Option::is_none) {
                continue;
            }
            names.insert(name);
        }
        for (child, original) in files {
            if original.is_some() && child.parent() == Some(path.as_path()) && !self.hidden(child) {
                names.insert(
                    child
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| invalid("历史来源文件名不是 UTF-8"))?
                        .to_owned(),
                );
            }
        }
        Ok(Some(names.into_iter().collect()))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Backup {
    pub path: PathBuf,
    pub sha: String,
}
#[derive(Debug, Clone)]
struct Contents {
    root: PathBuf,
    files: BTreeMap<PathBuf, Option<Backup>>,
    heads: BTreeMap<PathBuf, Option<String>>,
    artifacts: BTreeSet<PathBuf>,
    absent_dirs: BTreeSet<PathBuf>,
    transitions: BTreeSet<(PathBuf, Option<String>, Option<String>)>,
    versions: BTreeMap<(PathBuf, String), Backup>,
    deletions: BTreeMap<String, Arc<crate::lifecycle_history::Deletion>>,
    repopulations: BTreeMap<String, Arc<crate::chunk_lifecycle::RepopProof>>,
    region_changes: BTreeMap<String, Arc<crate::region_history::Change>>,
}
/// 已核实文件版本的只读引用。不存在的文件也有明确版本，不合成空文件。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileVersion {
    root: PathBuf,
    path: PathBuf,
    sha: Option<String>,
    live: bool,
    absent_directory: Option<PathBuf>,
}
impl FileVersion {
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.root.as_os_str().len()
            + self.path.as_os_str().len()
            + self.sha.as_ref().map_or(0, String::len)
            + self
                .absent_directory
                .as_ref()
                .map_or(0, |path| path.as_os_str().len())
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn sha256(&self) -> Option<&str> {
        self.sha.as_deref()
    }
}
/// 恢复来源绑定实际备份或未被本会话改写的世界文件；两类来源分别核对。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum FileLocation {
    Backup { path: PathBuf },
    World { length: usize },
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArchivedFile {
    target: PathBuf,
    location: FileLocation,
    sha: String,
}
impl ArchivedFile {
    pub(crate) fn backup_path(&self, root: &Path) -> Result<Option<PathBuf>, WorldError> {
        self.validate()?;
        let FileLocation::Backup { path } = &self.location else {
            return Ok(None);
        };
        let empty = BTreeSet::new();
        Layout {
            root,
            artifacts: &empty,
            absent_dirs: &empty,
        }
        .checked(&root.join(path))
        .map(Some)
    }
    pub(crate) fn validate(&self) -> Result<(), WorldError> {
        let relative = |path: &Path| {
            !path.as_os_str().is_empty()
                && path
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
        };
        let location = match &self.location {
            FileLocation::Backup { path } => relative(path) && path != &self.target,
            FileLocation::World { length } => *length <= 256 * 1024 * 1024,
        };
        if !relative(&self.target)
            || !location
            || self.sha.len() != 64
            || !self
                .sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid("历史文件证明的路径、大小或摘要无效"));
        }
        Ok(())
    }
    pub(crate) fn read(
        &self,
        root: &Path,
        expected: &Path,
        limit: usize,
        source: &impl IdentitySource,
        reserve: impl FnOnce(usize) -> Result<(), WorldError>,
    ) -> Result<Vec<u8>, WorldError> {
        self.validate()?;
        let empty = BTreeSet::new();
        if (Layout {
            root,
            artifacts: &empty,
            absent_dirs: &empty,
        })
        .checked(&root.join(&self.target))?
            != expected
        {
            return Err(invalid("历史文件证明不属于声明的区块存储"));
        }
        let bytes = match &self.location {
            FileLocation::Backup { .. } => {
                let backup = self.backup_path(root)?.expect("已选择备份来源");
                read_file(&backup, limit, reserve)?
                    .ok_or_else(|| invalid(format!("恢复历史来源已丢失：{}", backup.display())))?
            }
            FileLocation::World { length } => {
                if *length > limit {
                    return Err(invalid("历史世界文件超过单文件预算"));
                }
                reserve(*length)?;
                let bytes = source
                    .read(expected)?
                    .ok_or_else(|| invalid("历史世界文件来源已缺失"))?;
                if bytes.len() != *length {
                    return Err(invalid("历史世界文件来源大小改变"));
                }
                bytes
            }
        };
        if hash(&bytes) != self.sha {
            return Err(invalid(format!(
                "恢复历史来源摘要改变：{}",
                expected.display()
            )));
        }
        Ok(bytes)
    }
}
/// 已验证事务的持久来源。克隆共享元数据；读取时重新核对不可变备份摘要。
/// 逻辑删除只隐藏已核实范围的槽和 MCC；物理文件版本及事务来源不随显示投影改变。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Masks {
    pub(crate) regions: BTreeMap<PathBuf, BTreeSet<(i32, i32)>>,
    pub(crate) external: BTreeSet<PathBuf>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Projection {
    masks: Masks,
    versions: Arc<Vec<FileVersion>>,
    regions: Arc<RegionSources>,
}
#[derive(Debug, Clone)]
pub struct Archive(Arc<Contents>, Arc<Projection>);
/// 在同一归档中选择操作自己的文件版本；未选择的文件沿用最早历史来源。
/// 目录枚举也采用同一选择，不能把新建目录的当前磁盘内容混入历史扫描。
pub struct VersionView<'a> {
    archive: &'a Archive,
    files: BTreeMap<PathBuf, FileVersion>,
    directories: BTreeMap<PathBuf, bool>,
}
impl VersionView<'_> {
    fn absent(&self, path: &Path) -> bool {
        self.directories
            .iter()
            .any(|(directory, exists)| !exists && path.starts_with(directory))
    }
    fn checked(&self, path: &Path) -> Result<PathBuf, WorldError> {
        let empty = BTreeSet::new();
        Layout {
            root: self.archive.world_root(),
            artifacts: &self.archive.0.artifacts,
            absent_dirs: &empty,
        }
        .checked(path)
    }
    /// 在分配实际文件载荷前计入调用方预算，缺失版本不分配任何字节。
    pub(crate) fn read_reserved(
        &self,
        path: &Path,
        limit: usize,
        reserve: impl FnOnce(usize) -> Result<(), WorldError>,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        let path = self.checked(path)?;
        if self.absent(&path) {
            return Ok(None);
        }
        match self.files.get(&path) {
            Some(version) => self.archive.read_version_reserved(version, limit, reserve),
            None => self.archive.read_original_reserved(&path, limit, reserve),
        }
    }
}
impl IdentitySource for VersionView<'_> {
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
        self.read_reserved(path, 256 * 1024 * 1024, |_| Ok(()))
    }
    fn is_directory(&self, path: &Path) -> Result<bool, WorldError> {
        let path = self.checked(path)?;
        if self.files.contains_key(&path)
            || self.archive.0.artifacts.contains(&path)
            || self.absent(&path)
        {
            return Ok(false);
        }
        if let Some(exists) = self.directories.get(&path) {
            return Ok(*exists);
        }
        self.archive.original_is_directory(&path)
    }
    fn directory_members(&self, path: &Path) -> Result<Option<Vec<String>>, WorldError> {
        let path = self.checked(path)?;
        if !self.is_directory(&path)? {
            return Ok(None);
        }
        let mut names = match self.archive.original_directory_members(&path)? {
            Some(names) => names.into_iter().collect::<BTreeSet<_>>(),
            None if self.directories.get(&path) == Some(&true) => BTreeSet::new(),
            None => return Err(invalid("文件版本视图没有该目录的存在证明")),
        };
        // 目录存在性来自所选版本；删除文件后的空目录与原先没有目录分别表示。
        for (directory, exists) in &self.directories {
            if directory.parent() == Some(path.as_path()) {
                let name = directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| invalid("文件版本目录名不是 UTF-8"))?;
                if *exists {
                    names.insert(name.to_owned());
                } else {
                    names.remove(name);
                }
            }
        }
        for (file, version) in &self.files {
            if file.parent() == Some(path.as_path()) {
                let name = file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| invalid("文件版本名称不是 UTF-8"))?;
                if version.sha.is_some() {
                    names.insert(name.to_owned());
                } else {
                    names.remove(name);
                }
            }
        }
        Ok(Some(names.into_iter().collect()))
    }
}
/// 后台已核对的来源推进；仅绑定原归档身份，不冻结逻辑编辑修订。
#[derive(Debug, Clone)]
pub struct Observation {
    pub(crate) previous: Option<Archive>,
    pub(crate) next: Archive,
}
impl Observation {
    /// 从真实保存身份、不可变备份和当前磁盘头建立通知；不复制当前会话内容。
    pub fn nbt_saved(
        root: &Path,
        previous: Option<&Archive>,
        identities: &[crate::nbt_file::SaveIdentity],
    ) -> Result<Option<Self>, WorldError> {
        let root = fs::canonicalize(root)?;
        let mut files = BTreeMap::new();
        let mut heads = BTreeMap::new();
        let mut transitions = BTreeSet::new();
        let mut versions = BTreeMap::new();
        let mut artifacts = BTreeSet::new();
        let mut artifact_sets = Vec::new();
        for identity in identities
            .iter()
            .filter(|identity| identity.path.starts_with(&root))
        {
            let path = crate::nbt_file::safe_path(&root, &identity.path)?;
            crate::nbt_file::safe_path(&identity.root, &identity.path)?;
            let empty = BTreeSet::new();
            let layout = Layout {
                root: &root,
                artifacts: &empty,
                absent_dirs: &empty,
            };
            let before = match (&identity.source_sha, &identity.backup) {
                (Some(sha), Some(backup)) => {
                    let backup = layout.checked(backup)?;
                    let bytes = read_file(&backup, crate::MAX_NBT as usize, |_| Ok(()))?
                        .ok_or_else(|| invalid("NBT 保存来源的备份已丢失"))?;
                    if hash(&bytes) != *sha {
                        return Err(invalid("NBT 保存来源的备份摘要改变"));
                    }
                    Some(Backup {
                        path: backup,
                        sha: sha.clone(),
                    })
                }
                (None, None) => None,
                _ => return Err(invalid("NBT 保存身份的原始存在性与备份不一致")),
            };
            if !artifact_sets
                .iter()
                .any(|set| Arc::ptr_eq(set, &identity.artifacts))
            {
                artifacts.extend(
                    identity
                        .artifacts
                        .iter()
                        .filter(|path| path.starts_with(&root))
                        .cloned(),
                );
                artifact_sets.push(Arc::clone(&identity.artifacts));
            }
            let after = Some(identity.candidate_sha.clone());
            let transition = (path.clone(), identity.source_sha.clone(), after.clone());
            if let Some(expected) = heads.get(&path) {
                if expected != &identity.source_sha
                    && !(expected == &after && transitions.contains(&transition))
                {
                    return Err(invalid("独立 NBT 保存通知的文件版本不连续"));
                }
            }
            if let Some(backup) = &before {
                versions
                    .entry((path.clone(), backup.sha.clone()))
                    .or_insert_with(|| backup.clone());
            }
            files.entry(path.clone()).or_insert(before);
            heads.insert(path, after);
            transitions.insert(transition);
        }
        if files.is_empty() {
            return Ok(None);
        }
        if files.keys().any(|path| artifacts.contains(path)) {
            return Err(invalid("NBT 保存目标与其事务工件重叠"));
        }
        let incoming = Archive(
            Arc::new(Contents {
                root,
                files,
                heads,
                transitions,
                versions,
                deletions: BTreeMap::new(),
                repopulations: BTreeMap::new(),
                region_changes: BTreeMap::new(),
                artifacts,
                absent_dirs: BTreeSet::new(),
            }),
            Arc::new(Projection::default()),
        );
        let next = match previous {
            Some(previous) => previous.merged(&incoming)?,
            None => incoming,
        };
        next.verify_current()?;
        Ok(Some(Self {
            previous: previous.cloned(),
            next,
        }))
    }
}
impl Archive {
    /// 同一版本登记表的原始物理来源；不复制备份元数据，也不把逻辑删除当成磁盘事实。
    pub(crate) fn unprojected(&self) -> Self {
        Self(Arc::clone(&self.0), Arc::new(Projection::default()))
    }
    pub(crate) fn with_versions(
        &self,
        versions: &Arc<Vec<FileVersion>>,
    ) -> Result<Self, WorldError> {
        self.version_view(versions.iter().cloned())?;
        Ok(Self(
            Arc::clone(&self.0),
            Arc::new(Projection {
                masks: Masks::default(),
                versions: Arc::clone(versions),
                regions: Default::default(),
            }),
        ))
    }
    pub(crate) fn selected_versions(&self) -> Arc<Vec<FileVersion>> {
        Arc::clone(&self.1.versions)
    }
    pub(crate) fn matches_selection(&self, versions: &[FileVersion]) -> bool {
        self.1.versions.as_slice() == versions
            && self.1.masks.regions.is_empty()
            && self.1.masks.external.is_empty()
            && self.1.regions.is_empty()
    }
    pub(crate) fn with_masks(&self, masks: Masks) -> Self {
        Self(
            Arc::clone(&self.0),
            Arc::new(Projection {
                masks,
                versions: Arc::new(Vec::new()),
                regions: Arc::clone(&self.1.regions),
            }),
        )
    }
    pub(crate) fn has_file(&self, path: &Path) -> bool {
        self.0.files.contains_key(path)
    }
    pub(crate) fn original_has_file(&self, path: &Path) -> Result<bool, WorldError> {
        let path = self.layout().checked(path)?;
        if self.layout().hidden(&path) {
            return Ok(false);
        }
        if let Some(file) = self.0.files.get(&path) {
            return Ok(file.is_some());
        }
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Ok(true),
            Ok(_) => Err(invalid("原始来源不是普通文件")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    pub(crate) fn masked_slot(&self, path: &Path, chunk: (i32, i32)) -> bool {
        self.1
            .masks
            .regions
            .get(path)
            .is_some_and(|chunks| chunks.contains(&chunk))
    }
    /// 冻结既有版本的真实备份位置，供恢复事务独立重放；不把实时文件当作不可变来源。
    pub(crate) fn archived_file(&self, version: &FileVersion) -> Result<ArchivedFile, WorldError> {
        if version.root != self.0.root {
            return Err(invalid("历史文件证明属于另一个世界"));
        }
        let sha = version
            .sha
            .as_ref()
            .ok_or_else(|| invalid("缺失文件不能作为历史槽位载荷来源"))?;
        let backup = self
            .0
            .versions
            .get(&(version.path.clone(), sha.clone()))
            .ok_or_else(|| invalid("历史文件版本没有登记的不可变备份"))?;
        if !self.0.artifacts.contains(&backup.path) {
            return Err(invalid("历史文件备份不在实际登记的事务工件中"));
        }
        let relative = |path: &Path| {
            path.strip_prefix(&self.0.root)
                .map(Path::to_owned)
                .map_err(|_| invalid("历史文件证明越过世界目录"))
        };
        let file = ArchivedFile {
            target: relative(&version.path)?,
            location: FileLocation::Backup {
                path: relative(&backup.path)?,
            },
            sha: sha.clone(),
        };
        file.validate()?;
        Ok(file)
    }
    /// 取得逻辑原始文件及其持久证明。已登记文件只读相应备份，未登记文件显式绑定本次世界源。
    pub(crate) fn original_witness(
        &self,
        path: &Path,
        limit: usize,
        reserve: impl FnOnce(usize) -> Result<(), WorldError>,
    ) -> Result<(ArchivedFile, Vec<u8>), WorldError> {
        let path = self.layout().checked(path)?;
        if self.layout().hidden(&path) {
            return Err(invalid("历史原始文件不存在于该来源目录"));
        }
        if self.has_file(&path) {
            let version = self.original_version(&path)?;
            let proof = self.archived_file(&version)?;
            let bytes = self
                .read_version_reserved(&version, limit, reserve)?
                .ok_or_else(|| invalid("历史原始文件缺失"))?;
            return Ok((proof, bytes));
        }
        let bytes =
            read_file(&path, limit, reserve)?.ok_or_else(|| invalid("历史世界文件来源缺失"))?;
        let proof = ArchivedFile {
            target: path
                .strip_prefix(&self.0.root)
                .map_err(|_| invalid("历史世界文件路径越界"))?
                .to_owned(),
            location: FileLocation::World {
                length: bytes.len(),
            },
            sha: hash(&bytes),
        };
        proof.validate()?;
        Ok((proof, bytes))
    }
    /// 当前派生数据必须仍属于已提交的实际文件头，不能混入外部改写后的另一份几何。
    pub(crate) fn verify_head(&self, path: &Path, bytes: Option<&[u8]>) -> Result<(), WorldError> {
        if let Some(expected) = self.0.heads.get(path) {
            if bytes.map(hash).as_ref() != expected.as_ref() {
                return Err(invalid(format!(
                    "派生数据显示来源已独立改变：{}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
    /// 只枚举此归档实际持有且磁盘头内容不同的文件，不把缺席的旧记录当作删除。
    pub(crate) fn changed_paths_since<'a>(
        &'a self,
        previous: Option<&'a Archive>,
    ) -> impl Iterator<Item = &'a Path> {
        self.0.heads.iter().filter_map(move |(path, head)| {
            (previous.and_then(|previous| previous.0.heads.get(path)) != Some(head))
                .then_some(path.as_path())
        })
    }
    pub(crate) fn same_source(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) && (Arc::ptr_eq(&self.1, &other.1) || self.1 == other.1)
    }
    pub(crate) fn same_registry(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    /// 独立文件通知推进版本登记时，保留接收方当前的逻辑删除 / 版本选择。
    pub(crate) fn with_projection_from(&self, current: &Self) -> Self {
        Self(Arc::clone(&self.0), Arc::clone(&current.1))
    }
    /// 只由核对过实际目标、备份与完整事务证明的保存入口构造。
    pub(crate) fn from_verified(
        root: PathBuf,
        files: BTreeMap<PathBuf, Option<Backup>>,
        heads: BTreeMap<PathBuf, Option<String>>,
        artifacts: BTreeSet<PathBuf>,
        absent_dirs: BTreeSet<PathBuf>,
    ) -> Self {
        let transitions = files
            .iter()
            .filter_map(|(path, before)| {
                heads.get(path).map(|after| {
                    (
                        path.clone(),
                        before.as_ref().map(|backup| backup.sha.clone()),
                        after.clone(),
                    )
                })
            })
            .collect();
        let versions = files
            .iter()
            .filter_map(|(path, backup)| {
                backup
                    .as_ref()
                    .map(|backup| ((path.clone(), backup.sha.clone()), backup.clone()))
            })
            .collect();
        Self(
            Arc::new(Contents {
                root,
                files,
                heads,
                artifacts,
                absent_dirs,
                transitions,
                versions,
                deletions: BTreeMap::new(),
                repopulations: BTreeMap::new(),
                region_changes: BTreeMap::new(),
            }),
            Arc::new(Projection::default()),
        )
    }
    fn layout(&self) -> Layout<'_> {
        Layout {
            root: &self.0.root,
            artifacts: &self.0.artifacts,
            absent_dirs: &self.0.absent_dirs,
        }
    }
    pub fn world_root(&self) -> &Path {
        &self.0.root
    }
    /// 精确选择已登记的版本，不扫描备份目录，不按命名猜测中间镜像。
    pub fn version_view(
        &self,
        versions: impl IntoIterator<Item = FileVersion>,
    ) -> Result<VersionView<'_>, WorldError> {
        let mut view = VersionView {
            archive: self,
            files: BTreeMap::new(),
            directories: BTreeMap::new(),
        };
        for version in versions {
            if version.root != self.0.root {
                return Err(invalid("文件版本视图属于另一个世界"));
            }
            let path = view.checked(&version.path)?;
            if self.0.artifacts.contains(&path) || !self.0.files.contains_key(&path) {
                return Err(invalid("文件版本视图包含未登记的世界来源"));
            }
            let known = match &version.sha {
                Some(sha) => {
                    self.0.versions.contains_key(&(path.clone(), sha.clone()))
                        || (version.live && self.0.heads.get(&path) == Some(&version.sha))
                }
                None => self.0.transitions.iter().any(|(file, before, after)| {
                    file == &path && (before.is_none() || after.is_none())
                }),
            };
            if !known {
                return Err(invalid("文件版本视图引用的中间来源未登记"));
            }
            if let Some(previous) = view.files.get(&path) {
                if previous.sha != version.sha
                    || previous.absent_directory != version.absent_directory
                {
                    return Err(invalid("文件版本视图对同一路径选择了不同版本"));
                }
                continue;
            }
            let mut parent = path.parent();
            while let Some(directory) = parent {
                if !directory.starts_with(&self.0.root) {
                    break;
                }
                let exists = !version
                    .absent_directory
                    .as_ref()
                    .is_some_and(|absent| directory.starts_with(absent));
                if view
                    .directories
                    .insert(directory.to_owned(), exists)
                    .is_some_and(|previous| previous != exists)
                {
                    return Err(invalid("文件版本视图选择了互相矛盾的目录状态"));
                }
                if directory == self.0.root {
                    break;
                }
                parent = directory.parent();
            }
            view.files.insert(path, version);
        }
        for directory in view.directories.keys() {
            if view.files.contains_key(directory)
                || self.0.artifacts.contains(directory)
                || self.0.files.get(directory).is_some_and(Option::is_some)
            {
                return Err(invalid("文件版本视图的文件与目录重叠"));
            }
            match fs::symlink_metadata(directory) {
                Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
                    return Err(invalid("文件版本视图的目录类型改变"))
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(view)
    }
    pub fn deletions(&self) -> impl Iterator<Item = &Arc<crate::lifecycle_history::Deletion>> {
        self.0.deletions.values()
    }
    /// 仅供实际事务入口登记已经保存并核对的中间版本，不改变当前磁盘头或逻辑投影。
    pub(crate) fn retain_verified_versions(mut self, versions: Vec<(PathBuf, Backup)>) -> Self {
        let contents = Arc::make_mut(&mut self.0);
        for (target, backup) in versions {
            contents.artifacts.insert(backup.path.clone());
            contents
                .versions
                .entry((target, backup.sha.clone()))
                .or_insert(backup);
        }
        self
    }
    pub(crate) fn retain_verified_transitions(
        mut self,
        transitions: Vec<(PathBuf, Option<String>, Option<String>)>,
    ) -> Self {
        Arc::make_mut(&mut self.0).transitions.extend(transitions);
        self
    }
    /// 获取明确保留的中间文件版本；不根据摘要寻找未登记文件或改走当前磁盘。
    pub fn retained_version(&self, path: &Path, sha: &str) -> Result<FileVersion, WorldError> {
        let path = self.layout().checked(path)?;
        if !self
            .0
            .versions
            .contains_key(&(path.clone(), sha.to_owned()))
        {
            return Err(invalid("所需中间文件版本没有登记的真实备份"));
        }
        Ok(FileVersion {
            root: self.0.root.clone(),
            path,
            sha: Some(sha.to_owned()),
            live: false,
            absent_directory: None,
        })
    }
    pub fn repopulations(&self) -> impl Iterator<Item = &Arc<crate::chunk_lifecycle::RepopProof>> {
        self.0.repopulations.values()
    }
    pub fn region_changes(&self) -> impl Iterator<Item = &Arc<crate::region_history::Change>> {
        self.0.region_changes.values()
    }
    pub(crate) fn register_region_changes(
        mut self,
        changes: Vec<crate::region_history::Change>,
    ) -> Self {
        let contents = Arc::make_mut(&mut self.0);
        for change in changes {
            contents
                .region_changes
                .entry(change.identity().to_owned())
                .or_insert_with(|| Arc::new(change));
        }
        self
    }
    pub(crate) fn register_repopulations(
        mut self,
        proofs: BTreeMap<String, Arc<crate::chunk_lifecycle::RepopProof>>,
    ) -> Self {
        Arc::make_mut(&mut self.0).repopulations.extend(proofs);
        self
    }
    pub(crate) fn register_deletions(
        mut self,
        deletions: Vec<crate::lifecycle_history::Deletion>,
    ) -> Self {
        let contents = Arc::make_mut(&mut self.0);
        for deletion in deletions {
            contents
                .deletions
                .entry(deletion.identity().to_owned())
                .or_insert_with(|| Arc::new(deletion));
        }
        self
    }
    /// 返回该事务归档最早的真实来源版本；调用方不能凭路径猜测不存在的来源。
    pub fn original_version(&self, path: &Path) -> Result<FileVersion, WorldError> {
        let path = self.layout().checked(path)?;
        let original = self
            .0
            .files
            .get(&path)
            .ok_or_else(|| invalid("文件不在已核实的事务来源中"))?;
        let absent_directory = if original.is_none() {
            self.0
                .absent_dirs
                .iter()
                .find(|directory| path.starts_with(directory))
                .cloned()
        } else {
            None
        };
        Ok(FileVersion {
            root: self.0.root.clone(),
            path,
            sha: original.as_ref().map(|backup| backup.sha.clone()),
            live: false,
            absent_directory,
        })
    }
    /// 记录已提交的文件头，后续合并归档可通过真实备份读取这个中间版本。
    pub fn committed_version(&self, path: &Path) -> Result<FileVersion, WorldError> {
        let path = self.layout().checked(path)?;
        let sha = self
            .0
            .heads
            .get(&path)
            .ok_or_else(|| invalid("文件没有已核实的提交版本"))?
            .clone();
        Ok(FileVersion {
            root: self.0.root.clone(),
            path,
            sha,
            live: true,
            absent_directory: None,
        })
    }
    /// 读取明确版本，优先使用该版本登记的不可变备份；备份缺失或摘要变化直接报错。
    pub fn read_version(
        &self,
        version: &FileVersion,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        self.read_version_reserved(version, limit, |_| Ok(()))
    }
    pub(crate) fn read_version_reserved(
        &self,
        version: &FileVersion,
        limit: usize,
        reserve: impl FnOnce(usize) -> Result<(), WorldError>,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        if self.0.root != version.root {
            return Err(invalid("文件版本属于另一个世界"));
        }
        let empty = BTreeSet::new();
        let layout = Layout {
            root: &self.0.root,
            artifacts: &self.0.artifacts,
            absent_dirs: &empty,
        };
        let path = layout.checked(&version.path)?;
        if self.0.artifacts.contains(&path) {
            return Err(invalid("事务工件不能冒充世界文件版本"));
        }
        let Some(sha) = &version.sha else {
            if !self
                .0
                .transitions
                .iter()
                .any(|(file, before, after)| file == &path && (before.is_none() || after.is_none()))
            {
                return Err(invalid("归档没有该文件不存在的版本证明"));
            }
            return Ok(None);
        };
        let source = match self.0.versions.get(&(path.clone(), sha.clone())) {
            Some(backup) => &backup.path,
            None if version.live && self.0.heads.get(&path) == Some(&Some(sha.clone())) => &path,
            None => {
                return Err(invalid(format!(
                    "文件版本没有登记的真实备份：{}",
                    path.display()
                )))
            }
        };
        layout.checked(source)?;
        let bytes = read_file(source, limit, reserve)?
            .ok_or_else(|| invalid(format!("文件版本的来源已丢失：{}", source.display())))?;
        if hash(&bytes) != *sha {
            return Err(invalid(format!(
                "文件版本的来源摘要改变：{}",
                source.display()
            )));
        }
        Ok(Some(bytes))
    }
    pub(crate) fn read_reserved(
        &self,
        path: &Path,
        limit: usize,
        mut reserve: impl FnMut(usize) -> Result<(), WorldError>,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        let path = self.layout().checked(path)?;
        if self.1.masks.external.contains(&path) {
            return Ok(None);
        }
        let bytes = if self.1.versions.is_empty() {
            self.read_original_reserved(&path, limit, &mut reserve)?
        } else {
            self.version_view(self.1.versions.iter().cloned())?
                .read_reserved(&path, limit, &mut reserve)?
        };
        let Some(mut bytes) = self.project_region(&path, bytes, limit, &mut reserve)? else {
            return Ok(None);
        };
        if let Some(chunks) = self.1.masks.regions.get(&path) {
            if !bytes.is_empty() {
                crate::region_plan::parse_slots(&bytes)?;
                for chunk in chunks {
                    let index = (chunk.1.rem_euclid(32) * 32 + chunk.0.rem_euclid(32)) as usize;
                    bytes[index * 4..index * 4 + 4].fill(0);
                    bytes[4096 + index * 4..4096 + index * 4 + 4].fill(0);
                }
            }
        }
        Ok(Some(bytes))
    }
    fn read_original_reserved(
        &self,
        path: &Path,
        limit: usize,
        reserve: impl FnOnce(usize) -> Result<(), WorldError>,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        let layout = self.layout();
        let path = layout.checked(path)?;
        if layout.hidden(&path) {
            return Ok(None);
        }
        match self.0.files.get(&path) {
            Some(Some(_)) => {
                self.read_version_reserved(&self.original_version(&path)?, limit, reserve)
            }
            Some(None) => Ok(None),
            None => read_file(&path, limit, reserve),
        }
    }
    /// 后续保存必须接在已知磁盘版本之后；历史读取仍保留最早的实际来源。
    pub fn merged(&self, later: &Self) -> Result<Self, WorldError> {
        if self.0.root != later.0.root {
            return Err(invalid("不能合并不同世界的保存历史来源"));
        }
        let mut combined = (*self.0).clone();
        for (path, original) in &later.0.files {
            if let Some(expected) = combined.heads.get(path) {
                let before = original.as_ref().map(|backup| backup.sha.clone());
                let shared = reaches(
                    &before,
                    expected,
                    later
                        .0
                        .transitions
                        .iter()
                        .filter(|transition| {
                            &transition.0 == path && combined.transitions.contains(*transition)
                        })
                        .map(|(_, before, after)| (before.clone(), after.clone())),
                );
                let continued = later.0.heads.get(path).is_some_and(|after| {
                    reaches(
                        expected,
                        after,
                        later
                            .0
                            .transitions
                            .iter()
                            .filter(|(file, _, _)| file == path)
                            .map(|(_, before, after)| (before.clone(), after.clone())),
                    )
                });
                let intermediate = expected
                    .as_ref()
                    .is_some_and(|sha| later.0.versions.contains_key(&(path.clone(), sha.clone())))
                    && reaches(
                        &before,
                        expected,
                        later
                            .0
                            .transitions
                            .iter()
                            .filter(|(file, _, _)| file == path)
                            .map(|(_, before, after)| (before.clone(), after.clone())),
                    );
                if before != *expected && !((shared || intermediate) && continued) {
                    return Err(invalid(format!(
                        "后续保存与历史磁盘基线不连续：{}",
                        path.display()
                    )));
                }
            }
            combined
                .files
                .entry(path.clone())
                .or_insert_with(|| original.clone());
        }
        combined.heads.extend(later.0.heads.clone());
        for (version, backup) in &later.0.versions {
            combined
                .versions
                .entry(version.clone())
                .or_insert_with(|| backup.clone());
        }
        combined.deletions.extend(
            later
                .0
                .deletions
                .iter()
                .map(|(identity, deletion)| (identity.clone(), Arc::clone(deletion))),
        );
        combined.repopulations.extend(
            later
                .0
                .repopulations
                .iter()
                .map(|(identity, proof)| (identity.clone(), Arc::clone(proof))),
        );
        combined.region_changes.extend(
            later
                .0
                .region_changes
                .iter()
                .map(|(identity, change)| (identity.clone(), Arc::clone(change))),
        );
        combined
            .transitions
            .extend(later.0.transitions.iter().cloned());
        combined.artifacts.extend(later.0.artifacts.iter().cloned());
        combined
            .absent_dirs
            .extend(later.0.absent_dirs.iter().cloned());
        Ok(Self(Arc::new(combined), Arc::clone(&self.1)))
    }
    /// 规划下一次写入时核对真实磁盘头；历史来源从不冒充当前文件的写入依据。
    pub fn verify_current(&self) -> Result<(), WorldError> {
        for (path, expected) in &self.0.heads {
            let actual = read_file(path, 256 * 1024 * 1024, |_| Ok(()))?;
            if actual.as_deref().map(hash).as_ref() != expected.as_ref() {
                return Err(invalid(format!(
                    "保存后的世界文件已独立改变：{}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

/// 会话领域扫描共用同一来源选择；没有保存历史时读取真实磁盘。
pub(crate) struct View<'a>(pub Option<&'a Archive>);
impl IdentitySource for View<'_> {
    fn is_directory(&self, path: &Path) -> Result<bool, WorldError> {
        match self.0 {
            Some(origin) => origin.is_directory(path),
            None => DiskIdentitySource.is_directory(path),
        }
    }
    fn directory_members(&self, path: &Path) -> Result<Option<Vec<String>>, WorldError> {
        match self.0 {
            Some(origin) => origin.directory_members(path),
            None => DiskIdentitySource.directory_members(path),
        }
    }
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
        match self.0 {
            Some(origin) => origin.read(path),
            None => read_file(path, 256 * 1024 * 1024, |_| Ok(())),
        }
    }
}
impl IdentitySource for Archive {
    fn is_directory(&self, path: &Path) -> Result<bool, WorldError> {
        if self.1.masks.external.contains(path) {
            return Ok(false);
        }
        if !self.1.regions.is_empty() {
            self.region_directory_view()?.is_directory(path)
        } else if self.1.versions.is_empty() {
            self.original_is_directory(path)
        } else {
            self.version_view(self.1.versions.iter().cloned())?
                .is_directory(path)
        }
    }
    fn directory_members(&self, path: &Path) -> Result<Option<Vec<String>>, WorldError> {
        let names = if !self.1.regions.is_empty() {
            self.region_directory_view()?.directory_members(path)?
        } else if self.1.versions.is_empty() {
            self.original_directory_members(path)?
        } else {
            self.version_view(self.1.versions.iter().cloned())?
                .directory_members(path)?
        };
        Ok(names.map(|names| {
            names
                .into_iter()
                .filter(|name| !self.1.masks.external.contains(&path.join(name)))
                .collect()
        }))
    }
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
        self.read_reserved(path, 256 * 1024 * 1024, |_| Ok(()))
    }
}
impl Archive {
    fn original_is_directory(&self, path: &Path) -> Result<bool, WorldError> {
        self.layout().is_directory(path, &self.0.files)
    }
    fn original_directory_members(&self, path: &Path) -> Result<Option<Vec<String>>, WorldError> {
        self.layout().directory_members(path, &self.0.files)
    }
}
