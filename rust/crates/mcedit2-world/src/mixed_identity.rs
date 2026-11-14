//! 对整组提交后的世界核对实体身份，包括其他维度、玩家及同时保存的元数据。
use super::*;
use crate::entity_editing::IdentitySource;

struct FinalView<'a, 'b> {
    source: &'b SourceView<'a>,
    files: BTreeMap<PathBuf, Option<&'b [u8]>>,
    directories: BTreeSet<PathBuf>,
}
impl IdentitySource for FinalView<'_, '_> {
    fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, WorldError> {
        let path = self.source.layout().checked(path)?;
        match self.files.get(&path) {
            Some(bytes) => Ok(bytes.map(<[u8]>::to_vec)),
            None => self.source.read(&path),
        }
    }
    fn is_directory(&self, path: &Path) -> Result<bool, WorldError> {
        let path = self.source.layout().checked(path)?;
        if self.files.contains_key(&path) {
            return Ok(false);
        }
        if self.directories.contains(&path) {
            return Ok(true);
        }
        self.source.is_directory(&path)
    }
    fn directory_members(&self, path: &Path) -> Result<Option<Vec<String>>, WorldError> {
        let path = self.source.layout().checked(path)?;
        if !self.is_directory(&path)? {
            return Ok(None);
        }
        let mut names = match self.source.directory_members(&path)? {
            Some(names) => names.into_iter().collect::<BTreeSet<_>>(),
            None => BTreeSet::new(), // 本次事务明确创建的目录尚未存在。
        };
        for directory in &self.directories {
            if directory.parent() == Some(path.as_path()) {
                names.insert(
                    directory
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| err("实体最终视图的目录名不是 UTF-8"))?
                        .to_owned(),
                );
            }
        }
        for (file, bytes) in &self.files {
            if file.parent() == Some(path.as_path()) {
                let name = file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| err("实体最终视图的文件名不是 UTF-8"))?;
                if bytes.is_some() {
                    names.insert(name.to_owned());
                } else {
                    names.remove(name);
                }
            }
        }
        Ok(Some(names.into_iter().collect()))
    }
}

pub(super) fn verify_final(
    world: &WorldSummary,
    root: &Path,
    journal: &Journal,
    images: &[&[u8]],
    source: &SourceView<'_>,
) -> Result<(), WorldError> {
    let mut files = BTreeMap::new();
    for (entry, image) in journal.entries.iter().zip(images) {
        files.insert(
            target(world, root, entry)?,
            (!deleting(&entry.proof)?).then_some(*image),
        );
    }
    let directories = journal
        .created_dirs
        .iter()
        .map(|directory| root.join(directory))
        .collect();
    let view = FinalView {
        source,
        files,
        directories,
    };
    let mut world = world.clone();
    for entry in &journal.entries {
        if let Proof::ChunkSnapshot { proof, .. } = &entry.proof {
            if matches!(proof.storage, crate::chunk_snapshot::Storage::Alpha { .. })
                && !world
                    .dimensions
                    .iter()
                    .any(|dimension| dimension.name == proof.dimension)
            {
                world.dimensions.push(crate::DimensionSummary {
                    name: proof.dimension.clone(),
                    region_dir: root.to_owned(),
                    regions: 0,
                    chunks: 0,
                    extension: "original c.x.z.dat (base36)".into(),
                });
            }
        }
    }
    // 所有候选都在文件视图中，不把某一个维度的独立暂存当成整组结果。
    crate::entity_editing::scan_identities_with(
        &view,
        &world,
        "",
        &BTreeMap::new(),
        512 * 1024 * 1024,
        || false,
    )?;
    Ok(())
}

pub(super) fn verify_maps_final(
    world: &WorldSummary,
    root: &Path,
    journal: &Journal,
    images: &[&[u8]],
    source: &SourceView<'_>,
) -> Result<(), WorldError> {
    let mut files = BTreeMap::new();
    for (entry, image) in journal.entries.iter().zip(images) {
        files.insert(
            target(world, root, entry)?,
            (!deleting(&entry.proof)?).then_some(*image),
        );
    }
    let view = FinalView {
        source,
        files,
        directories: journal
            .created_dirs
            .iter()
            .map(|directory| root.join(directory))
            .collect(),
    };
    crate::map_history::verify_final_counter(&view, world)
}
