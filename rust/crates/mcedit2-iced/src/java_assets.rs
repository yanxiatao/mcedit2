//! Java 资源先建立覆盖索引，再解码方块实际使用的图片；动画只保留静态首帧。
use super::*;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

#[derive(Clone)]
pub(crate) struct LoadControl {
    pub budget: Option<crate::resource_budget::ResourceBudget>,
    pub all_png: bool,
    pub block_definitions: Arc<Vec<crate::block_definitions::Binding>>,
    pub world_data: Arc<mcedit2_world::world_data::Snapshot>,
    pub cancelled: Arc<AtomicBool>,
    pub progress: Arc<dyn Fn(String) + Send + Sync>,
}
impl Default for LoadControl {
    fn default() -> Self {
        Self {
            budget: None,
            all_png: true,
            block_definitions: Default::default(),
            world_data: Default::default(),
            cancelled: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(|_| {}),
        }
    }
}
impl LoadControl {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) {
            Err("材质加载已取消".into())
        } else {
            Ok(())
        }
    }
    fn report(&self, text: String) -> Result<(), String> {
        self.check()?;
        (self.progress)(text);
        self.check()
    }
}

#[derive(Debug, Clone)]
struct Source {
    file: PathBuf,
    pack: Arc<PathBuf>,
    member: Option<String>,
    asset: String,
    colormap: Option<&'static str>,
    length: u64,
}
impl Source {
    fn description(&self) -> String {
        match &self.member {
            Some(member) => format!("{}!/{member}", self.file.display()),
            None => format!("{} ({})", self.file.display(), self.asset),
        }
    }
    /// 元数据在索引阶段保留原文；来源条目和原文字节都进入常驻工作集。
    fn metadata_bytes(&self, id: &str) -> Result<usize, String> {
        self.index_bytes(id)
            .checked_add(usize::try_from(self.length).map_err(|_| "动画元数据超过地址范围")?)
            .ok_or_else(|| "动画元数据常驻计量溢出".into())
    }
    fn canonical_block(&self) -> bool {
        self.asset.contains("/textures/block/") || self.asset.contains("/textures/blocks/")
    }
    /// 索引常驻开销估算；包含键、来源路径、ZIP 成员名及树节点余量。
    fn index_bytes(&self, id: &str) -> usize {
        std::mem::size_of::<(String, Self)>()
            + 64
            + id.len()
            + self.file.as_os_str().as_encoded_bytes().len()
            + self.pack.as_os_str().as_encoded_bytes().len()
            + std::mem::size_of::<PathBuf>()
            + 16
            + self.asset.len()
            + self.member.as_ref().map_or(0, String::len)
    }
}
#[path = "palette_textures.rs"]
mod palette_textures;
struct Index<'a> {
    palettes: palette_textures::Definitions,
    bank: TextureBank,
    sources: BTreeMap<String, Source>,
    metadata_sources: BTreeMap<String, Source>,
    control: &'a LoadControl,
    budget: crate::resource_budget::ResourceBudget,
    source_bytes: usize,
}
fn read_bounded(reader: impl Read, maximum: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > maximum {
        return Err("资源在读取期间超过单项预算".into());
    }
    Ok(bytes)
}
impl Index<'_> {
    fn palette(&mut self, reader: impl Read, size: u64) -> Result<(), String> {
        self.budget.check(
            self.bank.models.source_bytes(),
            self.source_bytes
                .checked_add(size as usize * 8)
                .ok_or("atlas 解析工作集溢出")?,
            0,
        )?;
        let bytes = read_bounded(reader, self.budget.bytes / 8)?;
        let other = self
            .source_bytes
            .checked_sub(self.palettes.bytes())
            .ok_or("atlas 来源计量失配")?;
        let budget = self.budget;
        let models = self.bank.models.source_bytes();
        self.palettes.insert(&bytes, |size| {
            budget.check(
                models,
                other.checked_add(size).ok_or("atlas 来源计量溢出")?,
                0,
            )
        })?;
        self.source_bytes = other
            .checked_add(self.palettes.bytes())
            .ok_or("atlas 来源计量溢出")?;
        self.budget
            .check(self.bank.models.source_bytes(), self.source_bytes, 0)
    }
    fn font(
        &mut self,
        id: String,
        definition: bool,
        reader: impl Read,
        size: u64,
    ) -> Result<(), String> {
        self.control.check()?;
        self.budget.check(
            self.bank.models.source_bytes(),
            self.source_bytes
                .checked_add(size as usize)
                .ok_or("字体读取工作集溢出")?,
            0,
        )?;
        let bytes = read_bounded(reader, self.budget.bytes)?;
        let before = self.bank.fonts.bytes();
        Arc::make_mut(&mut self.bank.fonts).insert(id, definition, bytes)?;
        self.source_bytes = self
            .source_bytes
            .checked_sub(before)
            .and_then(|n| n.checked_add(self.bank.fonts.bytes()))
            .ok_or("字体来源计数溢出")?;
        self.budget
            .check(self.bank.models.source_bytes(), self.source_bytes, 0)
    }
    fn begin_client(&mut self, bytes: &[u8]) -> Result<bool, String> {
        match biome_colors::client_version(bytes) {
            Ok(Some(version)) => {
                self.bank.item_version = Some(i32::try_from(version).map_err(|e| e.to_string())?);
                let before = self.bank.tints.definition_bytes();
                Arc::make_mut(&mut self.bank.tints).begin_client(version);
                self.source_bytes = self
                    .source_bytes
                    .checked_sub(before)
                    .and_then(|n| n.checked_add(self.bank.tints.definition_bytes()))
                    .ok_or("生物群系来源计数失配")?;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(error) => {
                Arc::make_mut(&mut self.bank.warnings).push(error);
                Ok(false)
            }
        }
    }
    fn native_biome(
        &mut self,
        name: &str,
        id: String,
        reader: impl Read,
        size: u64,
    ) -> Result<(), String> {
        self.control.check()?;
        let estimate = usize::try_from(size)
            .map_err(|_| "生物群系来源超过地址范围")?
            .checked_mul(8)
            .and_then(|n| n.checked_add(self.source_bytes))
            .ok_or("生物群系解析工作集溢出")?;
        self.budget
            .check(self.bank.models.source_bytes(), estimate, 0)?;
        let bytes = read_bounded(reader, self.budget.bytes / 8)?;
        let before = self.bank.tints.definition_bytes();
        if let Err(error) = Arc::make_mut(&mut self.bank.tints).insert(id, &bytes) {
            Arc::make_mut(&mut self.bank.warnings)
                .push(format!("客户端生物群系 {name} 无效：{error}"));
        }
        self.source_bytes = self
            .source_bytes
            .checked_sub(before)
            .and_then(|n| n.checked_add(self.bank.tints.definition_bytes()))
            .ok_or("生物群系来源计数溢出")?;
        self.budget
            .check(self.bank.models.source_bytes(), self.source_bytes, 0)
    }
    fn language(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let estimate = bytes
            .len()
            .checked_mul(8)
            .and_then(|n| n.checked_add(self.source_bytes))
            .ok_or("语言表工作集溢出")?;
        self.budget
            .check(self.bank.models.source_bytes(), estimate, 0)?;
        let before = self.bank.languages.bytes();
        match Arc::make_mut(&mut self.bank.languages).insert(name, bytes) {
            Ok(diagnostics) => {
                Arc::make_mut(&mut self.bank.warnings).extend(
                    diagnostics
                        .into_iter()
                        .map(|error| format!("语言资源 {name}：{error}")),
                );
                self.source_bytes = self
                    .source_bytes
                    .checked_sub(before)
                    .and_then(|n| n.checked_add(self.bank.languages.bytes()))
                    .ok_or("语言表内存计数溢出")?;
            }
            Err(error) => Arc::make_mut(&mut self.bank.warnings)
                .push(format!("语言资源 {name} 无效：{error}")),
        }
        self.budget
            .check(self.bank.models.source_bytes(), self.source_bytes, 0)
    }
    fn insert_source(&mut self, id: String, source: Source) -> Result<(), String> {
        let previous = self.sources.get(&id).map_or(0, |old| old.index_bytes(&id));
        let bytes = self
            .source_bytes
            .checked_sub(previous)
            .and_then(|n| n.checked_add(source.index_bytes(&id)))
            .ok_or("资源索引大小溢出")?;
        self.budget
            .check(self.bank.models.source_bytes(), bytes, 0)
            .map_err(|error| format!("建立资源索引失败：{error}"))?;
        self.source_bytes = bytes;
        self.sources.insert(id, source);
        Ok(())
    }
    fn record(
        &mut self,
        name: &str,
        asset: PackAsset,
        source: Source,
        reader: impl Read,
        size: u64,
    ) -> Result<(), String> {
        let origin = source.description();
        let first = self.bank.warnings.len();
        let result = self.record_inner(name, asset, source, reader, size);
        for diagnostic in &mut Arc::make_mut(&mut self.bank.warnings)[first..] {
            diagnostic.push_str("；来源 ");
            diagnostic.push_str(&origin);
        }
        result.map_err(|error| format!("{origin}：{error}"))
    }
    fn record_inner(
        &mut self,
        name: &str,
        asset: PackAsset,
        mut source: Source,
        reader: impl Read,
        size: u64,
    ) -> Result<(), String> {
        source.length = size;
        let maximum = asset.max_bytes(self.budget);
        if size > maximum {
            return Err(format!(
                "资源 {name} 大小 {size} 字节，超过当前读取预算 {maximum} 字节"
            ));
        }
        match asset {
            PackAsset::NumericBiomes => {
                let bytes = read_bounded(reader, maximum)?;
                let before = self.bank.tints.definition_bytes();
                Arc::make_mut(&mut self.bank.tints).insert_numeric(&bytes)?;
                self.source_bytes = self
                    .source_bytes
                    .checked_sub(before)
                    .and_then(|n| n.checked_add(self.bank.tints.definition_bytes()))
                    .ok_or("数字 biome 颜色表来源计数失配")?;
            }
            PackAsset::Language => {
                let bytes = read_bounded(reader, maximum)?;
                self.language(name, &bytes)?;
            }
            PackAsset::Texture(id) => {
                if !self.bank.accept_texture_path(&id, name) {
                    return Ok(());
                }
                if name.contains("/textures/block/") {
                    self.bank.modern_textures.insert(id.clone());
                }
                self.bank.layer_png.insert(id.clone());
                self.insert_source(id, source)?;
            }
            PackAsset::Colormap(which) => {
                self.insert_source(format!("@colormap/{which}"), source)?;
            }
            PackAsset::Obj | PackAsset::Mtl => {
                let bytes = read_bounded(reader, maximum)?;
                let result = if matches!(asset, PackAsset::Obj) {
                    Arc::make_mut(&mut self.bank.models).insert_obj(name, &bytes)
                } else {
                    Arc::make_mut(&mut self.bank.models).insert_java_mtl(name, &bytes)
                };
                if let Err(error) = result {
                    Arc::make_mut(&mut self.bank.statistics).failed_models += 1;
                    Arc::make_mut(&mut self.bank.warnings)
                        .push(format!("静态模型资源 {name} 无效：{error}"));
                }
            }
            PackAsset::Metadata(id) => {
                let bytes = read_bounded(reader, maximum)?;
                if self.bank.accept_metadata_path(&id, name) {
                    source.length = bytes.len() as u64;
                    let previous = self
                        .metadata_sources
                        .get(&id)
                        .map(|s| s.metadata_bytes(&id))
                        .transpose()?
                        .unwrap_or(0);
                    let current = source.metadata_bytes(&id)?;
                    self.source_bytes = self
                        .source_bytes
                        .checked_sub(previous)
                        .and_then(|n| n.checked_add(current))
                        .ok_or("动画来源索引计量溢出")?;
                    self.bank.layer_metadata.insert(id.clone());
                    self.bank.metadata.insert(id.clone(), bytes);
                    self.metadata_sources.insert(id, source);
                }
            }
            PackAsset::Model => {
                let bytes = read_bounded(reader, maximum)?;
                self.bank.models.check_source_budget(name, bytes.len())?;
                match Arc::make_mut(&mut self.bank.models).insert_with_diagnostics(name, &bytes) {
                    Ok((_, diagnostics)) => Arc::make_mut(&mut self.bank.warnings)
                        .extend(diagnostics.into_iter().map(|d| format!("模型 {name}：{d}"))),
                    Err(error) => {
                        Arc::make_mut(&mut self.bank.statistics).failed_models += 1;
                        Arc::make_mut(&mut self.bank.warnings)
                            .push(format!("模型 {name} 无效：{error}"));
                    }
                }
            }
            _ => return Err("Java 索引收到非 Java 资源类型".into()),
        }
        self.budget
            .check(self.bank.models.source_bytes(), self.source_bytes, 0)
    }
    fn archive(&mut self, path: &Path) -> Result<(), String> {
        let pack = Arc::new(path.to_path_buf());
        let opened = crate::pack_sources::open(
            path,
            |extra| {
                self.budget.check(
                    self.bank.models.source_bytes(),
                    self.source_bytes
                        .checked_add(extra)
                        .ok_or("嵌套归档索引工作集溢出")?,
                    0,
                )
            },
            || self.control.check(),
        )?;
        let resident = opened.resident;
        self.source_bytes = self
            .source_bytes
            .checked_add(resident)
            .ok_or("嵌套归档索引工作集溢出")?;
        let prefix = opened.prefix;
        let mut archive = opened.archive;
        let has_client_main = if crate::pack_sources::is_nested(path) {
            false
        } else {
            match archive.by_name("net/minecraft/client/main/Main.class") {
                Ok(_) => true,
                Err(zip::result::ZipError::FileNotFound) => false,
                Err(error) => return Err(format!("客户端入口索引失败：{error}")),
            }
        };
        let mut version_id = None;
        let client = if has_client_main {
            match archive.by_name("version.json") {
                Ok(member) => {
                    let bytes = read_bounded(member, 1 << 20)?;
                    let client = self.begin_client(&bytes)?;
                    if client {
                        version_id = serde_json::from_slice::<serde_json::Value>(&bytes)
                            .map_err(|e| e.to_string())?["id"]
                            .as_str()
                            .map(str::to_owned);
                    }
                    client
                }
                Err(zip::result::ZipError::FileNotFound) => false,
                Err(error) => return Err(format!("客户端版本声明读取失败：{error}")),
            }
        } else {
            false
        };
        let external = if has_client_main {
            crate::client_assets::discover(path, version_id.as_deref())?
        } else {
            None
        };
        let external_bytes = external
            .as_ref()
            .map_or(0, crate::client_assets::Index::memory_bytes);
        self.source_bytes = self
            .source_bytes
            .checked_add(external_bytes)
            .ok_or("资产索引工作集溢出")?;
        self.budget
            .check(self.bank.models.source_bytes(), self.source_bytes, 0)?;
        for i in 0..archive.len() {
            if i % 64 == 0 {
                self.control.check()?;
            }
            let member = archive.by_index(i).map_err(|e| e.to_string())?;
            let actual_name = member.name().to_owned();
            let Some(name) = actual_name.strip_prefix(&prefix).map(str::to_owned) else {
                continue;
            };
            // 外部索引和 JAR 属于同一个原版资源包，同路径仅采用外部对象。
            if external
                .as_ref()
                .is_some_and(|index| index.objects.contains_key(&name))
            {
                continue;
            }
            if palette_textures::asset(&name) {
                let size = member.size();
                self.palette(member, size)?;
                continue;
            }
            if let Some((id, definition)) = crate::font_resources::asset(&name) {
                let size = member.size();
                self.font(id, definition, member, size)?;
                continue;
            }
            if client {
                if let Some(id) = biome_colors::definition_asset(&name) {
                    let size = member.size();
                    self.native_biome(&name, id, member, size)?;
                    continue;
                }
            }
            let Some(asset) = PackAsset::classify(&name) else {
                continue;
            };
            if !asset.is_java_resource(&name) {
                continue;
            }
            let colormap = if let PackAsset::Colormap(which) = &asset {
                Some(*which)
            } else {
                None
            };
            let source = Source {
                file: path.into(),
                pack: Arc::clone(&pack),
                member: Some(actual_name),
                asset: name.clone(),
                colormap,
                length: 0,
            };
            let size = member.size();
            self.record(&name, asset, source, member, size)?;
        }
        if let Some(external) = external {
            for (name, object) in external.objects {
                self.control.check()?;
                let font = crate::font_resources::asset(&name);
                let asset =
                    PackAsset::classify(&name).filter(|asset| asset.is_java_resource(&name));
                let palette = palette_textures::asset(&name);
                if font.is_none() && asset.is_none() && !palette {
                    continue;
                }
                let result = (|| {
                    let reader = object.open()?;
                    if palette {
                        return self.palette(reader, object.size);
                    }
                    if let Some((id, definition)) = font {
                        return self.font(id, definition, reader, object.size);
                    }
                    let asset = asset.expect("已筛选的资产缺少分类");
                    let colormap = if let PackAsset::Colormap(which) = &asset {
                        Some(*which)
                    } else {
                        None
                    };
                    let source = Source {
                        file: object.file,
                        pack: Arc::new(external.source.clone()),
                        member: None,
                        asset: name.clone(),
                        colormap,
                        length: object.size,
                    };
                    self.record(&name, asset, source, reader, object.size)
                })();
                result.map_err(|e: String| {
                    format!("资产索引 {} / {name}：{e}", external.source.display())
                })?;
            }
        }
        self.source_bytes = self
            .source_bytes
            .checked_sub(external_bytes)
            .and_then(|bytes| bytes.checked_sub(resident))
            .ok_or("资产索引工作集计量失配")?;
        Ok(())
    }
    fn directory(
        &mut self,
        root: &Path,
        current: &Path,
        scan_bytes: &mut usize,
    ) -> Result<(), String> {
        self.control.check()?;
        let meta = match fs::symlink_metadata(current) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        if meta.file_type().is_symlink() {
            return Err("资源目录不能使用符号链接".into());
        }
        if !meta.is_dir() {
            return Err("资源 assets 不是目录".into());
        }
        let mut items = Vec::new();
        for item in fs::read_dir(current).map_err(|e| e.to_string())? {
            self.control.check()?;
            let item = item.map_err(|e| e.to_string())?;
            // 目录排序的临时列表同样按工作预算检查，不按文件数量截断。
            let bytes = std::mem::size_of::<fs::DirEntry>()
                + item.file_name().as_encoded_bytes().len()
                + 64;
            *scan_bytes = scan_bytes
                .checked_add(bytes)
                .ok_or("资源目录索引大小溢出")?;
            self.budget.check(
                self.bank.models.source_bytes(),
                self.source_bytes
                    .checked_add(*scan_bytes)
                    .ok_or("资源索引大小溢出")?,
                0,
            )?;
            items.push((item, bytes));
        }
        items.sort_by_key(|(i, _)| i.file_name());
        for (item, bytes) in items {
            *scan_bytes -= bytes;
            self.control.check()?;
            let ty = item.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                continue;
            }
            let path = item.path();
            if ty.is_dir() {
                self.directory(root, &path, scan_bytes)?;
            } else if ty.is_file() {
                let name = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                if palette_textures::asset(&name) {
                    let file = File::open(&path).map_err(|e| e.to_string())?;
                    let size = file.metadata().map_err(|e| e.to_string())?.len();
                    self.palette(file, size)?;
                    continue;
                }
                if let Some((id, definition)) = crate::font_resources::asset(&name) {
                    let file = File::open(&path).map_err(|e| e.to_string())?;
                    let size = file.metadata().map_err(|e| e.to_string())?.len();
                    self.font(id, definition, file, size)?;
                    continue;
                }
                if let Some(id) = biome_colors::definition_asset(&name) {
                    let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
                    self.native_biome(
                        &name,
                        id,
                        File::open(&path).map_err(|e| e.to_string())?,
                        size,
                    )?;
                    continue;
                }
                let Some(asset) = PackAsset::classify(&name) else {
                    continue;
                };
                if !asset.is_java_resource(&name) {
                    continue;
                }
                let colormap = if let PackAsset::Colormap(which) = &asset {
                    Some(*which)
                } else {
                    None
                };
                let source = Source {
                    file: path.clone(),
                    pack: Arc::new(root.to_path_buf()),
                    member: None,
                    asset: name.clone(),
                    colormap,
                    length: 0,
                };
                let file = File::open(path).map_err(|e| e.to_string())?;
                let size = file.metadata().map_err(|e| e.to_string())?.len();
                self.record(&name, asset, source, file, size)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "client_asset_tests.rs"]
mod client_asset_tests;
#[path = "java_decode.rs"]
mod decode;
#[path = "java_png.rs"]
mod png;
#[cfg(test)]
fn freeze_png(data: &[u8], metadata: Option<&[u8]>) -> Result<(Tile, bool), String> {
    png::Plan::new(data, metadata, false, true)
        .decode(data, &|| false)
        .material
        .ok_or("PNG 材质合同没有结果")?
}

pub(super) fn open(paths: &[PathBuf], control: &LoadControl) -> Result<TextureBank, String> {
    if paths.is_empty() {
        return Err("请至少选择一个 Java 资源来源".into());
    }
    let started = Instant::now();
    control.check()?;
    let budget = control
        .budget
        .map(Ok)
        .unwrap_or_else(crate::resource_budget::ResourceBudget::automatic)?;
    let mut index = Index {
        bank: TextureBank::default(),
        sources: BTreeMap::new(),
        metadata_sources: BTreeMap::new(),
        palettes: palette_textures::Definitions::default(),
        control,
        budget,
        source_bytes: 0,
    };
    Arc::make_mut(&mut index.bank.models).budget = Some(budget);
    index.bank.definition_source_paths = Arc::new(paths.to_vec());
    index.bank.world_data = Arc::clone(&control.world_data);
    Arc::make_mut(&mut index.bank.statistics).budget_bytes = budget.bytes;
    for (i, path) in paths.iter().enumerate() {
        control.report(format!(
            "建立资源索引 {}/{}：{}",
            i + 1,
            paths.len(),
            path.file_name().unwrap_or_default().to_string_lossy()
        ))?;
        if let Some(version) = crate::legacy_render::Version::from_client_path(path) {
            Arc::make_mut(&mut index.bank.models).legacy = Some(version.registry());
            index.bank.item_version = path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| matches!(*s, "minecraft-1.12.2-client" | "1.12.2"))
                .map(|_| 1343);
            let before = index.bank.tints.definition_bytes();
            Arc::make_mut(&mut index.bank.tints).begin_legacy();
            index.source_bytes = index
                .source_bytes
                .checked_sub(before)
                .and_then(|n| n.checked_add(index.bank.tints.definition_bytes()))
                .ok_or("旧版气候表来源计数失配")?;
            Arc::make_mut(&mut index.bank.warnings).push(
                "旧版数字 biome 默认使用 MCEdit2 静态气候表，显式资源包颜色表可整份替换默认表。"
                    .into(),
            );
        }
        index.bank.modern_textures.clear();
        index.bank.modern_metadata.clear();
        index.bank.layer_png.clear();
        index.bank.layer_metadata.clear();
        let meta = if crate::pack_sources::is_nested(path) {
            None
        } else {
            Some(fs::symlink_metadata(path).map_err(|e| e.to_string())?)
        };
        if meta
            .as_ref()
            .is_some_and(|meta| meta.file_type().is_symlink())
        {
            return Err("资源来源不能使用符号链接".into());
        }
        if meta.as_ref().is_some_and(|meta| meta.is_dir()) {
            let root = path.canonicalize().map_err(|e| e.to_string())?;
            index.directory(&root, &root.join("assets"), &mut 0)?;
            let version_file = root.join("version.json");
            let client = match fs::symlink_metadata(&version_file) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err("客户端版本声明不能使用符号链接".into())
                }
                Ok(metadata) if metadata.is_file() => index.begin_client(&read_bounded(
                    File::open(&version_file).map_err(|e| e.to_string())?,
                    1 << 20,
                )?)?,
                Ok(_) => false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.to_string()),
            };
            if client {
                index.directory(&root, &root.join("data/minecraft/worldgen/biome"), &mut 0)?;
            }
        } else if meta.as_ref().is_none_or(|meta| meta.is_file()) {
            index.archive(path)?;
        } else {
            return Err(format!("不支持的资源来源：{}", path.display()));
        }
        for id in &index.bank.layer_png {
            if !index.bank.layer_metadata.contains(id) {
                index.bank.metadata.remove(id);
                if let Some(source) = index.metadata_sources.remove(id) {
                    index.source_bytes = index
                        .source_bytes
                        .checked_sub(source.metadata_bytes(id)?)
                        .ok_or("动画来源索引移除计量失配")?;
                }
            }
        }
    }
    if index
        .bank
        .models
        .legacy
        .as_ref()
        .is_some_and(|registry| registry.version() == crate::legacy_render::Version::V112)
    {
        Arc::make_mut(&mut index.bank.fonts).legacy_112 = true;
    }
    control.report("解析方块模型与纹理依赖…".into())?;
    Arc::make_mut(&mut index.bank.models)
        .load_native_block_models(index.bank.tints.client_version)?;
    if index.bank.tints.client_version == Some(3955) {
        Arc::make_mut(&mut index.bank.warnings).push(
            "附魔台书本显示离线闭合姿态；游戏中的旋转、翻页和玩家靠近动画不保存在存档中。".into(),
        );
    }
    let modern_chests = index
        .sources
        .contains_key("minecraft:entity/chest/normal_left")
        && index
            .sources
            .contains_key("minecraft:entity/chest/normal_right");
    let (special_count, special_warnings) =
        Arc::make_mut(&mut index.bank.models).install_vanilla_specials(modern_chests)?;
    Arc::make_mut(&mut index.bank.warnings).extend(special_warnings);
    if special_count > 0 {
        log::info!("[assets] 已建立 {special_count} 类原版特殊方块的静态实体模型");
    }
    Arc::make_mut(&mut index.bank.models).install_configured(&control.block_definitions)?;
    for binding in control.block_definitions.iter() {
        for texture in binding.textures.values() {
            let id =
                crate::resource_models::normalize_texture(texture).ok_or("配置纹理 ID 无效")?;
            if !index.sources.contains_key(&id) {
                return Err(format!(
                    "方块 {} 指定的纹理不存在：{id}",
                    binding.selector()
                ));
            }
        }
    }
    let catalog_bytes = index
        .sources
        .keys()
        .filter(|id| !id.starts_with('@'))
        .try_fold(0usize, |bytes, id| {
            bytes
                .checked_add(std::mem::size_of::<String>() + id.len())
                .and_then(|bytes| {
                    bytes.checked_add(std::mem::size_of::<(String, Arc<PathBuf>)>() + 64 + id.len())
                })
                .ok_or("纹理目录与来源工作集溢出")
        })?;
    index.source_bytes = index
        .source_bytes
        .checked_add(catalog_bytes)
        .ok_or("纹理目录工作集溢出")?;
    budget.check(index.bank.models.source_bytes(), index.source_bytes, 0)?;
    index.bank.available_textures = Arc::new(
        index
            .sources
            .keys()
            .filter(|id| !id.starts_with('@'))
            .cloned()
            .collect(),
    );
    index.bank.texture_origins = Arc::new(
        index
            .sources
            .iter()
            .filter(|(id, _)| !id.starts_with('@'))
            .map(|(id, source)| (id.clone(), Arc::clone(&source.pack)))
            .collect(),
    );
    Arc::make_mut(&mut index.bank.statistics).indexed_png = index
        .sources
        .values()
        .filter(|s| s.colormap.is_none())
        .count();
    Arc::make_mut(&mut index.bank.statistics).indexed_models = index.bank.model_count();
    let mut wanted = Arc::make_mut(&mut index.bank.models).retain_block_dependencies();
    if index.bank.item_data_version() == Some(3955) {
        wanted.extend(
            index
                .bank
                .wolf_variant_catalog()?
                .textures()
                .filter(|id| index.sources.contains_key(*id))
                .map(str::to_owned),
        );
    }
    wanted.extend(
        index
            .sources
            .keys()
            .filter(|id| {
                crate::item_specials::texture_dependency(id)
                    || crate::item_glint::texture_dependency(id)
            })
            .cloned(),
    );
    let font_images = index.bank.fonts.texture_dependencies()?;
    Arc::make_mut(&mut index.bank.warnings).extend(index.bank.models.validate_builtin_loaders());
    Arc::make_mut(&mut index.bank.warnings)
        .extend(index.bank.models.validate_block_visual_definitions());
    wanted.extend(index.bank.models.obj_texture_dependencies());
    wanted.extend(index.bank.models.native_block_texture_dependencies());
    wanted.extend(
        crate::entity_geometry::TEXTURES
            .iter()
            .map(|id| {
                crate::resource_models::normalize_texture(id).expect("内建依赖使用有效资源 ID")
            })
            .filter(|id| index.sources.contains_key(id)),
    );
    // 原版旗帜图集由 banner_base 单图和所有命名空间的 entity/banner 目录组成。
    // 只发现图像；图案注册身份由存档 Holder 或已核实的原版注册表提供。
    wanted.extend(
        index
            .sources
            .keys()
            .filter(|id| crate::banner_geometry::texture_dependency(id))
            .cloned(),
    );
    wanted.extend(
        index
            .sources
            .keys()
            .filter(|id| crate::skull_geometry::texture_dependency(id))
            .cloned(),
    );
    wanted.extend(
        index
            .sources
            .keys()
            .filter(|id| crate::sign_geometry::texture_dependency(id))
            .cloned(),
    );
    wanted.extend(
        index
            .sources
            .keys()
            .filter(|id| crate::entity_geometry::villager_texture_dependency(id))
            .cloned(),
    );
    wanted.extend(
        index
            .sources
            .keys()
            .filter(|id| crate::painting_geometry::texture_dependency(id))
            .cloned(),
    );
    wanted.extend(
        index
            .sources
            .keys()
            .filter(|id| crate::map_geometry::texture_dependency(id))
            .cloned(),
    );
    wanted.extend(
        index
            .sources
            .keys()
            .filter(|id| {
                crate::armor_geometry::texture_dependency(id)
                    || crate::elytra_geometry::texture_dependency(id)
                    || crate::entity_geometry::animals::texture_dependency(id)
            })
            .cloned(),
    );
    wanted.extend(index.palettes.equipment_outputs());
    let generated = index.palettes.outputs(&wanted);
    wanted.extend(index.palettes.dependencies(&wanted));
    Arc::make_mut(&mut index.bank.statistics).missing_references = wanted
        .union(&font_images)
        .filter(|id| {
            id.as_str() != MISSING_TEXTURE_ID
                && !index.sources.contains_key(*id)
                && !generated.contains(*id)
        })
        .cloned()
        .collect();
    for id in &index.bank.statistics.missing_references {
        Arc::make_mut(&mut index.bank.warnings).push(format!("模型引用的纹理不存在：{id}"));
    }
    for (loader, count) in index.bank.models.unsupported_loaders() {
        Arc::make_mut(&mut index.bank.warnings).push(format!(
            "{count} 个模型声明了尚未支持的渲染器 {loader}；对应模型需要渲染扩展。"
        ));
    }
    let mut atlas_ids: BTreeSet<_> = index
        .sources
        .iter()
        .filter(|(id, source)| {
            source.colormap.is_none() && (source.canonical_block() || wanted.contains(*id))
        })
        .map(|(id, _)| id.clone())
        .collect();
    atlas_ids.extend(generated);
    index.sources.retain(|id, source| {
        control.all_png
            || source.colormap.is_some()
            || source.canonical_block()
            || wanted.contains(id)
            || font_images.contains(id)
    });
    Arc::make_mut(&mut index.bank.statistics).requested_png = index
        .sources
        .values()
        .filter(|s| s.colormap.is_none())
        .count();
    let (mut bank, mut memory) = decode::decode(
        index.bank,
        index.sources,
        index.metadata_sources,
        &font_images,
        &atlas_ids,
        index.source_bytes,
        budget,
        control,
    )?;
    index
        .palettes
        .apply(&mut bank, &wanted, &mut memory, budget, control)?;
    if bank.tiles.is_empty() && bank.simple_tiles.is_empty() {
        return Err("资源来源没有可用于方块渲染的有效纹理".into());
    }
    bank.retain_villager_metadata();
    bank.metadata.clear();
    bank.modern_metadata.clear();
    bank.modern_textures.clear();
    bank.layer_png.clear();
    bank.layer_metadata.clear();
    Arc::make_mut(&mut bank.tiles).insert(MISSING_TEXTURE_ID.into(), missing_texture());
    control.report(format!(
        "生成图集：{} 个纹理，{} MiB",
        bank.statistics.decoded_png,
        memory / (1024 * 1024)
    ))?;
    let mut last = Instant::now();
    // 全部 PNG 可保留 CPU 缓存；界面/物品等未被方块引用的图像不占用 3D GPU 图集。
    // 移动所有权而不复制 RGBA；后续模型引用到的非 block/ 纹理同样进入图集。
    let tiles = std::mem::take(&mut bank.tiles);
    let tiles = Arc::try_unwrap(tiles).map_err(|_| "资源解码阶段不应共享纹理缓存")?;
    let (render_tiles, mut unused_tiles): (BTreeMap<_, _>, BTreeMap<_, _>) = tiles
        .into_iter()
        .partition(|(id, _)| id == MISSING_TEXTURE_ID || atlas_ids.contains(id));
    Arc::make_mut(&mut bank.statistics).atlas_png = render_tiles.len().saturating_sub(1);
    bank.tiles = Arc::new(render_tiles);
    let occupied = (bank.models.source_bytes() as u64)
        .checked_mul(8)
        .and_then(|n| n.checked_add(memory as u64))
        .and_then(|n| n.checked_add(bank.languages.bytes() as u64))
        .and_then(|n| n.checked_add(bank.fonts.bytes() as u64))
        .and_then(|n| n.checked_add(bank.tints.definition_bytes() as u64))
        .ok_or("资源工作集大小溢出")?;
    let atlas_budget = budget
        .bytes
        .checked_sub(occupied)
        .ok_or("资源工作集超过预算")?
        / 2;
    let mut atlas_tiles: BTreeMap<String, &Tile> = bank
        .tiles
        .iter()
        .map(|(id, tile)| (id.clone(), tile))
        .collect();
    for (id, sprite) in bank
        .live_animations
        .iter()
        .filter(|(id, _)| atlas_ids.contains(*id))
    {
        let tile = bank.tiles.get(id).ok_or("动画源缺少对应的图集首帧")?;
        if tile.width != sprite.source.width || tile.height != sprite.source.height {
            atlas_tiles.insert(super::simple_key(id), &sprite.source);
        }
    }
    for (id, tile) in bank
        .simple_tiles
        .iter()
        .filter(|(id, _)| atlas_ids.contains(*id))
    {
        atlas_tiles.insert(super::simple_key(id), tile);
    }
    // atlas 的透明 RGB 扩展不属于 SimpleTexture 上传；模糊整图必须保留原字节。
    for (id, sampler) in bank
        .simple_samplers
        .iter()
        .filter(|(id, _)| atlas_ids.contains(*id))
    {
        if sampler.as_ref().is_ok_and(|sampler| sampler.blur) {
            if let Some(tile) = bank
                .simple_image(id)
                .filter(|tile| tile.alpha_mode() == super::AlphaMode::Cutout)
            {
                atlas_tiles.insert(super::simple_key(id), tile);
            }
        }
    }
    let atlas = TextureAtlas::create_at_checked(
        &atlas_tiles,
        &bank.animations,
        0,
        atlas_budget,
        |done, total| {
            control.check()?;
            if last.elapsed().as_millis() >= 150 || done == total {
                control.report(format!("生成纹理图集 {done}/{total}"))?;
                last = Instant::now();
            }
            Ok(())
        },
    )?;
    drop(atlas_tiles);
    bank.atlas = Some(Arc::new(atlas));
    Arc::get_mut(bank.atlas.as_mut().unwrap())
        .expect("新建图集尚未共享")
        .configure_simple_samplers(&bank.simple_samplers)?;
    let atlas_bytes = bank.atlas.as_ref().unwrap().allocated_bytes();
    control.report(format!(
        "生成动画素材与 mip：{} 个动画源",
        bank.live_animations.len()
    ))?;
    super::atlas_animation::Set::install(
        Arc::get_mut(bank.atlas.as_mut().unwrap()).expect("新建图集尚未共享"),
        &bank.live_animations,
        &|| control.cancelled.load(std::sync::atomic::Ordering::Acquire),
    )?;
    Arc::make_mut(&mut bank.statistics).atlas_bytes = atlas_bytes;
    Arc::make_mut(&mut bank.statistics).animation_reserved_bytes =
        bank.atlas.as_ref().unwrap().allocated_bytes() - atlas_bytes;
    Arc::make_mut(&mut bank.tiles).append(&mut unused_tiles);
    control.check()?;
    log::info!(
        "[assets] Java 资源完成：{} 个纹理，{} 个模型，RGBA {} MiB / 图集 {} 页、{} MiB，{:?}；诊断 {} 条，动画缓存预算 {} MiB",
        bank.statistics.decoded_png,
        bank.model_count(),
        memory / (1024 * 1024),
        bank.atlas.as_ref().map_or(0, |atlas|atlas.page_count()),
        bank.statistics.atlas_bytes / (1024 * 1024),
        started.elapsed(),
        bank.warnings.len(),
        bank.statistics.animation_reserved_bytes.div_ceil(1024 * 1024)
    );
    control.report("材质与模型已就绪".into())?;
    Ok(bank)
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_scene_model_coverage() {
        let world = mcedit2_world::inspect_world(PathBuf::from(
            std::env::var_os("MCEDIT2_TEST_WORLD").expect("设置 MCEDIT2_TEST_WORLD"),
        ))
        .unwrap();
        let stack = crate::auto_assets::discover_stack(
            &world.root,
            world.version_name.as_deref().unwrap_or(""),
            &[],
        )
        .unwrap()
        .unwrap();
        let bank = open(
            &stack.paths,
            &LoadControl {
                all_png: false,
                ..Default::default()
            },
        )
        .unwrap();
        let session =
            mcedit2_world::editing::Session::new(world.clone(), world.dimensions[0].name.clone())
                .unwrap();
        let mut reader =
            mcedit2_world::block_lookup::ReadOnlyRegionCache::new(&world, session.dimension())
                .unwrap();
        let mut states = BTreeMap::new();
        for cx in -2..=6 {
            for cz in -9..=-1 {
                let patch = mcedit2_world::chunk_surface::inspect_full_chunk_cached(
                    &session,
                    &mut reader,
                    cx,
                    cz,
                )
                .unwrap();
                for voxel in patch.voxels {
                    let entry = states
                        .entry(format!("{}{:?}", voxel.state.name, voxel.state.properties))
                        .or_insert_with(|| {
                            (voxel.state.clone(), 0usize, (voxel.x, voxel.y, voxel.z))
                        });
                    entry.1 += 1;
                }
            }
        }
        let mut issues = Vec::new();
        for (id, (state, count, pos)) in &states {
            if matches!(state.name.as_str(), "minecraft:water" | "minecraft:lava") {
                continue;
            }
            let Some(parts) = bank.geometry_at(state, *pos) else {
                issues.push((
                    *count,
                    format!("无声明模型 {id}，{count} 暴露方块，样本 {pos:?}"),
                ));
                continue;
            };
            if parts.is_empty() {
                issues.push((
                    *count,
                    format!("空方块模型（需核对特殊渲染） {id}，{count} 暴露方块，样本 {pos:?}"),
                ));
                continue;
            }
            let mut missing = BTreeSet::new();
            for index in 0..parts.len() {
                for face in [
                    crate::isometric::Face::Top,
                    crate::isometric::Face::Bottom,
                    crate::isometric::Face::PosX,
                    crate::isometric::Face::NegX,
                    crate::isometric::Face::PosZ,
                    crate::isometric::Face::NegZ,
                ] {
                    if let Some(spec) = bank.models.element_face_at(state, face, index, *pos) {
                        if !bank.tiles.contains_key(&spec.texture) {
                            missing.insert(spec.texture);
                        }
                    }
                }
            }
            if !missing.is_empty() {
                issues.push((
                    *count,
                    format!("缺少纹理 {id}，{count} 暴露方块，样本 {pos:?}，{missing:?}"),
                ));
            }
        }
        issues.sort_by_key(|(count, _)| std::cmp::Reverse(*count));
        eprintln!(
            "真实观察区域 {} 个状态，{} 类缺口",
            states.len(),
            issues.len()
        );
        for (_, line) in issues.iter().take(30) {
            eprintln!("{line}");
        }
        std::fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/scene-model-coverage.txt"),
            issues
                .iter()
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert!(!states.is_empty());
    }
    fn png() -> Vec<u8> {
        let image = image::RgbaImage::from_raw(
            2,
            4,
            [[255, 0, 0, 255].repeat(4), [0, 255, 0, 255].repeat(4)].concat(),
        )
        .unwrap();
        let mut output = std::io::Cursor::new(Vec::new());
        image.write_to(&mut output, ImageFormat::Png).unwrap();
        output.into_inner()
    }

    fn static_decode_retains_full_image_and_only_allocates_selected_animation_frame() {
        let bytes = png();
        let mut oversized = bytes.clone();
        oversized[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        oversized[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(freeze_png(&oversized, None).is_err());
        let (tile, animated) = freeze_png(&bytes, None).unwrap();
        assert!(!animated);
        assert_eq!((tile.width, tile.height), (2, 4));
        let (tile, animated) =
            freeze_png(&bytes, Some(br#"{"animation":{"frames":[1,0]}}"#)).unwrap();
        assert!(animated);
        assert_eq!((tile.width, tile.height), (2, 2));
        assert_eq!(tile.rgba, [0, 255, 0, 255].repeat(4));
        for (metadata, expected, animated) in [
            (
                br#"{"animation":{"frames":[2]}}"#.as_slice(),
                [255, 0, 0, 255],
                false,
            ),
            (
                br#"{"animation":{"frames":[1]}}"#.as_slice(),
                [255, 0, 0, 255],
                false,
            ),
            (
                br#"{"animation":{"frames":[-1,1,0]}}"#.as_slice(),
                [0, 255, 0, 255],
                true,
            ),
            (
                br#"{"animation":{"frames":[1.9,0]}}"#.as_slice(),
                [0, 255, 0, 255],
                true,
            ),
            (
                br#"{"animation":{"frames":[4294967297,0]}}"#.as_slice(),
                [0, 255, 0, 255],
                true,
            ),
            (
                br#"{"animation":{"width":-1,"frames":[1,0]}}tail"#.as_slice(),
                [0, 255, 0, 255],
                true,
            ),
        ] {
            let (tile, actual) = freeze_png(&bytes, Some(metadata)).unwrap();
            assert_eq!(actual, animated);
            assert_eq!(tile.rgba, expected.repeat(4));
        }
        for metadata in [
            br#"{"animation":{"frames":[{"index":-1}]}}"#.as_slice(),
            br#"{"animation":{"interpolate":[]}}"#.as_slice(),
            br#"{animation:{frames:[1,0]}}"#.as_slice(),
        ] {
            assert!(freeze_png(&bytes, Some(metadata)).is_err());
        }
        let (tile, animated) = freeze_png(&bytes, Some(br#"{"texture":{"blur":true}}"#)).unwrap();
        assert!(!animated);
        assert_eq!(tile.height, 4);
        let (tile, animated) = freeze_png(
            &bytes,
            Some(br#"{"animation":{"frametime":1.5,"frames":[]}}"#),
        )
        .unwrap();
        assert_eq!((tile.width, tile.height), (2, 2));
        assert_eq!(tile.rgba, [255, 0, 0, 255].repeat(4));
        assert!(animated);
        let mut image = image::RgbaImage::from_pixel(1, 5000, image::Rgba([255, 0, 0, 255]));
        image.put_pixel(0, 4999, image::Rgba([0, 255, 0, 255]));
        let mut input = std::io::Cursor::new(Vec::new());
        image.write_to(&mut input, ImageFormat::Png).unwrap();
        let (tile, animated) = freeze_png(
            input.get_ref(),
            Some(br#"{"animation":{"frames":[4999,0]}}"#),
        )
        .unwrap();
        assert_eq!((tile.width, tile.height), (1, 1));
        assert_eq!(tile.rgba, [0, 255, 0, 255]);
        assert!(animated);
    }

    fn cancelled_load_never_reads_an_input_source() {
        let control = LoadControl {
            cancelled: Arc::new(AtomicBool::new(true)),
            ..Default::default()
        };
        assert!(open(&[PathBuf::from("不存在.zip")], &control)
            .unwrap_err()
            .contains("取消"));
    }

    fn java_pipeline_keeps_referenced_non_block_textures_and_skips_unrelated_gui() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "mcedit2-java-assets-{}-dependencies.zip",
            std::process::id()
        ));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name,data) in [
            ("assets/demo/blockstates/machine.json",br#"{"variants":{"":{"model":"demo:block/machine"}}}"#.as_slice()),
            ("assets/demo/models/block/machine.json",br##"{"parent":"minecraft:block/cube_all","textures":{"all":"demo:models/unusual"}}"##.as_slice()),
            ("assets/demo/textures/gui/unrelated.png",b"invalid PNG should never be decoded".as_slice()),
        ]{
            zip.start_file(name,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(data).unwrap();
        }
        zip.start_file(
            "assets/demo/textures/models/unusual.png",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&png()).unwrap();
        zip.finish().unwrap();
        let bank = open(
            std::slice::from_ref(&path),
            &LoadControl {
                all_png: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(bank.count(), 2);
        assert!(bank.warnings.is_empty());
        let state = mcedit2_world::editing::parse_state("demo:machine").unwrap();
        assert_eq!(
            bank.resolve_id_at(&state, crate::isometric::Face::Top, (0, 0, 0)),
            Some("demo:models/unusual".into())
        );
        assert!(bank.get("demo:gui/unrelated").is_none());
        assert!(bank.get("demo:models/unusual").is_some());
        std::fs::remove_file(path).unwrap();
    }

    fn atlas_cancellation_stops_before_publishing_any_partial_atlas() {
        let tile = Tile {
            width: 64,
            height: 64,
            rgba: [255, 0, 255, 0].repeat(64 * 64),
        };
        let tiles = BTreeMap::from([("demo:tile".into(), tile)]);
        let mut calls = 0;
        let result =
            TextureAtlas::create_at_checked(&tiles, &BTreeMap::new(), 0, u64::MAX, |_, _| {
                calls += 1;
                if calls >= 4 {
                    Err("测试取消".into())
                } else {
                    Ok(())
                }
            });
        assert!(result.unwrap_err().contains("取消"));
        assert_eq!(calls, 4);
    }

    fn all_png_mode_and_explicit_budget_report_actual_loading() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("mcedit2-all-png-{}.zip", std::process::id()));
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name, data) in [
            ("assets/test/textures/block/tile.png", png()),
            ("assets/test/textures/gui/large.png", png()),
            ("assets/test/textures/item/bad.png", b"invalid".to_vec()),
            (
                "assets/test/blockstates/test.json",
                br#"{"variants":{"":{"model":"test:block/test"}}}"#.to_vec(),
            ),
            (
                "assets/test/models/block/test.json",
                br##"{"textures":{"all":"test:missing"}}"##.to_vec(),
            ),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&data).unwrap();
        }
        zip.finish().unwrap();
        let control = LoadControl {
            budget: Some(crate::resource_budget::ResourceBudget { bytes: 100 }),
            ..Default::default()
        };
        assert!(open(std::slice::from_ref(&path), &control)
            .unwrap_err()
            .contains("预算"));
        let control = LoadControl {
            budget: Some(crate::resource_budget::ResourceBudget::from_input("2").unwrap()),
            ..Default::default()
        };
        let bank = open(std::slice::from_ref(&path), &control).unwrap();
        assert_eq!(
            (
                bank.statistics.indexed_png,
                bank.statistics.requested_png,
                bank.statistics.decoded_png,
                bank.statistics.failed_png
            ),
            (3, 3, 2, 1)
        );
        assert_eq!(bank.statistics.missing_references, ["test:missing"]);
        assert!(bank.get("test:gui/large").is_some());
        assert!(bank.statistics.atlas_bytes > 0);
        assert_eq!(bank.statistics.budget_bytes, 2 * 1024 * 1024);
        std::fs::remove_file(path).unwrap();
    }

    fn source_index_budget_counts_live_entries_and_releases_replaced_sources() {
        let id = "test:tile".to_owned();
        let source = Source {
            file: "first.zip".into(),
            pack: Arc::new("first.zip".into()),
            member: Some("assets/test/textures/block/tile.png".into()),
            asset: "assets/test/textures/block/tile.png".into(),
            colormap: None,
            length: 0,
        };
        let control = LoadControl::default();
        let mut index = Index {
            bank: TextureBank::default(),
            sources: BTreeMap::new(),
            metadata_sources: BTreeMap::new(),
            palettes: palette_textures::Definitions::default(),
            control: &control,
            budget: crate::resource_budget::ResourceBudget {
                bytes: source.index_bytes(&id) as u64,
            },
            source_bytes: 0,
        };
        for i in 0..1100 {
            let mut replacement = source.clone();
            replacement.file = format!("{i:05}.zip").into();
            replacement.pack = Arc::new(replacement.file.clone());
            index
                .insert_source(id.clone(), replacement.clone())
                .unwrap();
            assert_eq!(index.source_bytes, replacement.index_bytes(&id));
            assert_eq!(index.sources[&id].file, replacement.file);
        }
        let before = index.source_bytes;
        assert!(index
            .insert_source("test:other".into(), source)
            .unwrap_err()
            .contains("预算"));
        assert_eq!(index.sources.len(), 1);
        assert_eq!(index.source_bytes, before);
        assert_eq!(index.sources[&id].file, PathBuf::from("01099.zip"));
        let metadata = br#"{"animation":{}}"#;
        let id = "demo:meta".to_string();
        let name = "assets/demo/textures/block/meta.png.mcmeta";
        let source = Source {
            file: "00000.zip".into(),
            pack: Arc::new("00000.zip".into()),
            member: Some(name.into()),
            asset: name.into(),
            colormap: None,
            length: metadata.len() as u64,
        };
        let resident = source.metadata_bytes(&id).unwrap();
        let mut index = Index {
            bank: TextureBank::default(),
            sources: BTreeMap::new(),
            metadata_sources: BTreeMap::new(),
            palettes: palette_textures::Definitions::default(),
            control: &control,
            budget: crate::resource_budget::ResourceBudget {
                bytes: resident as u64,
            },
            source_bytes: 0,
        };
        for i in 0..40 {
            let mut replacement = source.clone();
            replacement.file = format!("{i:05}.zip").into();
            replacement.pack = Arc::new(replacement.file.clone());
            index
                .record(
                    name,
                    PackAsset::Metadata(id.clone()),
                    replacement,
                    std::io::Cursor::new(metadata),
                    metadata.len() as u64,
                )
                .unwrap();
            assert_eq!(index.source_bytes, resident);
            assert_eq!(index.bank.metadata[&id], metadata);
            assert_eq!(index.metadata_sources.len(), 1);
        }
        let name = "assets/demo/textures/block/other.png.mcmeta";
        let mut source = source;
        source.member = Some(name.into());
        source.asset = name.into();
        assert!(index
            .record(
                name,
                PackAsset::Metadata("demo:other".into()),
                source,
                std::io::Cursor::new(metadata),
                metadata.len() as u64
            )
            .unwrap_err()
            .contains("预算"));
    }

    fn many_java_layers_load_every_texture_and_apply_the_last_overlay() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!(
            "mcedit2-java-many-layers-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let first = png();
        let last = image::RgbaImage::from_pixel(2, 2, image::Rgba([7, 23, 81, 255]));
        let mut last_bytes = std::io::Cursor::new(Vec::new());
        last.write_to(&mut last_bytes, image::ImageFormat::Png)
            .unwrap();
        let mut paths = Vec::new();
        for i in 0..1100 {
            let path = root.join(format!("layer-{i:04}.zip"));
            let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
            zip.start_file(
                format!("assets/test/textures/block/tile_{i}.png"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&first).unwrap();
            if i == 0 || i == 1099 {
                zip.start_file(
                    "assets/test/textures/block/overlay.png",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                zip.write_all(if i == 0 { &first } else { last_bytes.get_ref() })
                    .unwrap();
            }
            zip.finish().unwrap();
            paths.push(path);
        }
        let bank = open(
            &paths,
            &LoadControl {
                budget: Some(crate::resource_budget::ResourceBudget::from_input("16").unwrap()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(bank.statistics.indexed_png, 1101);
        assert_eq!(bank.statistics.requested_png, 1101);
        assert_eq!(bank.statistics.decoded_png, 1101);
        assert_eq!(bank.statistics.failed_png, 0);
        assert!(bank.warnings.is_empty());
        let atlas = bank.atlas().unwrap();
        for i in 0..1100 {
            assert!(bank.get(&format!("test:tile_{i}")).is_some());
            assert!(atlas.entries.contains_key(&format!("test:tile_{i}")));
        }
        let uv = &atlas.entries["test:overlay"];
        assert_eq!(
            bank.get("test:overlay").unwrap().rgba,
            [7, 23, 81, 255].repeat(4)
        );
        assert_eq!(bank.atlas_pixel(uv.start, uv.page), Some([7, 23, 81, 255]));
        let root = root.canonicalize().unwrap();
        assert_eq!(
            root.parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn atlas_contains_more_than_u16_texture_ids_without_dropping_entries() {
        let tiles = (0..65_537)
            .map(|i| {
                (
                    format!("test:{i}"),
                    Tile {
                        width: 1,
                        height: 1,
                        rgba: vec![123, 45, 67, 255],
                    },
                )
            })
            .collect();
        let atlas = TextureAtlas::create_at_checked(
            &tiles,
            &BTreeMap::new(),
            0,
            // 此夹具验证 ID 容量；预算包含新增的独立 mip 及其工作区。
            256 * 1024 * 1024,
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(atlas.entries.len(), 65_537 * 2);
        assert_eq!(
            atlas
                .entries
                .keys()
                .filter(|id| !id.starts_with("@simple/"))
                .count(),
            65_537
        );
        for id in ["test:0", "test:65535", "test:65536"] {
            let uv = &atlas.entries[id];
            let page = atlas.page(uv.page).unwrap();
            let x = (uv.start[0] * page.width as f32) as usize;
            let y = (uv.start[1] * page.height as f32) as usize;
            assert_eq!(
                &page.rgba[(y * page.width as usize + x) * 4..][..4],
                &[123, 45, 67, 255]
            );
        }
    }

    fn asymmetric_texture_corners_match_java_world_face_coordinates() {
        use crate::isometric::Face;
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "mcedit2-java-assets-{}-face-uv.zip",
            std::process::id()
        ));
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let image = image::RgbaImage::from_fn(4, 4, |x, y| {
            image::Rgba(
                colors[match (x >= 2, y >= 2) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (true, true) => 2,
                    (false, true) => 3,
                }],
            )
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        for (name,data) in [
            ("assets/demo/blockstates/cube.json",br#"{"variants":{"":{"model":"demo:block/cube"}}}"#.as_slice()),
            ("assets/demo/models/block/cube.json",br##"{"parent":"minecraft:block/cube_all","textures":{"all":"demo:block/asymmetric"}}"##.as_slice()),
            ("assets/demo/textures/block/asymmetric.png",png.get_ref().as_slice()),
        ]{zip.start_file(name,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(data).unwrap();}
        zip.finish().unwrap();
        let bank = open(std::slice::from_ref(&path), &LoadControl::default()).unwrap();
        let state = mcedit2_world::editing::parse_state("demo:cube").unwrap();
        for face in [
            Face::Top,
            Face::Bottom,
            Face::PosX,
            Face::NegX,
            Face::PosZ,
            Face::NegZ,
        ] {
            let (uv, _, page) = bank
                .atlas_face_style_at(&state, face, 0, (0, 0, 0))
                .unwrap();
            for (i, [x, y, z]) in face.corners().into_iter().enumerate() {
                // 根据 Java 面的物理方向求 UV，不使用待验证的顶点排列表。
                let (u, v) = match face {
                    Face::Top => (x + 0.5, z + 0.5),
                    Face::Bottom => (x + 0.5, 0.5 - z),
                    Face::PosX => (0.5 - z, 0.5 - y),
                    Face::NegX => (z + 0.5, 0.5 - y),
                    Face::PosZ => (x + 0.5, 0.5 - y),
                    Face::NegZ => (0.5 - x, 0.5 - y),
                };
                let expected = colors[match (u > 0.5, v > 0.5) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (true, true) => 2,
                    (false, true) => 3,
                }];
                assert_eq!(
                    bank.atlas_pixel(uv[i], page),
                    Some(expected),
                    "{face:?} 的实际顶点 {:?}",
                    [x, y, z]
                );
            }
        }
        fs::remove_file(path).unwrap();
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Installed {
            real_scene_model_coverage();
        }

        if category == crate::test_categories::Category::Assets {
            super::client_asset_tests::complete();
            png::verify_reference();
            static_decode_retains_full_image_and_only_allocates_selected_animation_frame();
            cancelled_load_never_reads_an_input_source();
            java_pipeline_keeps_referenced_non_block_textures_and_skips_unrelated_gui();
            atlas_cancellation_stops_before_publishing_any_partial_atlas();
            all_png_mode_and_explicit_budget_report_actual_loading();
            source_index_budget_counts_live_entries_and_releases_replaced_sources();
            many_java_layers_load_every_texture_and_apply_the_last_overlay();
            atlas_contains_more_than_u16_texture_ids_without_dropping_entries();
            asymmetric_texture_corners_match_java_world_face_coordinates();
        }
    }
}
