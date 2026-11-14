//! 脚本上下文中的真实文档加载；追加槽位不移动已有来源、区块或冻结租约。
use super::*;
use crate::structure_document::{Document, SourceVersion};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    pub readonly: bool,
    /// 已核实的界面 / 来源首选维度；缺省采用读取器实际枚举的首个维度。
    pub dimension: Option<String>,
    /// 旧 schematic 必须由调用方交出明确的来源选择，不按其他文档推测。
    pub legacy_version: Option<SourceVersion>,
}
#[derive(Debug, Clone, Copy)]
pub struct LoadedDocument {
    pub slot: usize,
    pub created: bool,
}
enum PreparedInput {
    World(Input),
    Structure(Document),
}
/// 已由真实读取器完成的加载结果；安装之前不改变上下文的任何槽位。
pub struct PreparedDocument {
    slot: usize,
    registry_revision: Arc<()>,
    input: Option<PreparedInput>,
    bytes: usize,
}
impl PreparedDocument {
    pub fn slot(&self) -> usize {
        self.slot
    }
    pub fn created(&self) -> bool {
        self.input.is_some()
    }
}
#[derive(Debug, Clone)]
pub enum OpenedDocument {
    World { slot: usize, input: Input },
    Structure { slot: usize, document: Document },
}
impl OpenedDocument {
    pub fn slot(&self) -> usize {
        match self {
            Self::World { slot, .. } | Self::Structure { slot, .. } => *slot,
        }
    }
    pub fn path(&self) -> &Path {
        match self {
            Self::World { input, .. } => &input.world.root,
            Self::Structure { document, .. } => document.path(),
        }
    }
}
impl Working {
    pub fn document_count(&self) -> usize {
        self.dimensions.len()
    }
    pub fn document_info(&self, slot: usize) -> Result<Value, WorldError> {
        let dimension = self
            .dimensions
            .get(slot)
            .ok_or_else(|| refuse("实际文档槽位不存在"))?;
        let selections = self
            .selections
            .iter()
            .filter(|(key, _)| key.0 == slot)
            .map(|(key, region)| (key.1.clone(), *region))
            .collect();
        Ok(if let Some(index) = self.world_slots.get(&slot) {
            world_info(&self.inputs[*index], dimension, selections, self.readonly)
        } else {
            structure_info(
                &self.structures[&slot].baseline,
                dimension,
                selections,
                self.readonly,
            )
        })
    }
    /// 目录和传输预算可以在安装前核对完整的真实元数据，不预先暴露文档槽位。
    pub fn prepared_document_info(&self, document: &PreparedDocument) -> Result<Value, WorldError> {
        if !Arc::ptr_eq(&self.documents_revision, &document.registry_revision) {
            return Err(refuse("文档准备结果不属于当前目录来源"));
        }
        Ok(match &document.input {
            Some(PreparedInput::World(input)) => world_info(
                input,
                &input.dimension,
                input.selections.clone(),
                self.readonly,
            ),
            Some(PreparedInput::Structure(document)) => {
                let dimension = document.content().dimension.clone();
                structure_info(
                    document,
                    &dimension,
                    BTreeMap::from([(dimension.clone(), None)]),
                    self.readonly,
                )
            }
            None => return self.document_info(document.slot),
        })
    }
    /// 文档注册和应用目录登记可先共同预检，再在同一个可变上下文中安装。
    pub fn document_budget(
        &self,
        document: &PreparedDocument,
    ) -> Result<EditWorkBudget, WorldError> {
        let retained = self.reserve_document(document.bytes)?;
        let retained = retained
            .checked_add(self.frozen_bytes)
            .and_then(|bytes| bytes.checked_mul(4))
            .ok_or_else(|| refuse("文档与冻结来源合计工作集溢出"))?;
        let available = self
            .budget
            .bytes()
            .checked_sub(retained)
            .filter(|bytes| *bytes > 0)
            .ok_or_else(|| refuse("准备加载的文档已占满脚本工作预算"))?;
        EditWorkBudget::from_bytes(available)
    }
    fn prepared_existing(&self, document: LoadedDocument) -> PreparedDocument {
        PreparedDocument {
            slot: document.slot,
            registry_revision: Arc::clone(&self.documents_revision),
            input: None,
            bytes: 0,
        }
    }
    fn prepared_input(
        &self,
        input: PreparedInput,
        bytes: usize,
    ) -> Result<PreparedDocument, WorldError> {
        self.stop()?;
        let document = PreparedDocument {
            slot: self.dimensions.len(),
            registry_revision: Arc::clone(&self.documents_revision),
            input: Some(input),
            bytes,
        };
        self.document_budget(&document)?;
        Ok(document)
    }
    pub(super) fn world_input(&self, slot: usize) -> Result<&Input, WorldError> {
        let index = self
            .world_slots
            .get(&slot)
            .ok_or_else(|| refuse("脚本槽位不属于实际世界"))?;
        Ok(&self.inputs[*index])
    }
    /// 文档实际权限与 Inspector 等调用角色的外围只读权限分开。
    pub fn document_readonly(&self, slot: usize) -> Result<bool, WorldError> {
        if let Some(index) = self.world_slots.get(&slot) {
            return Ok(self.inputs[*index].readonly);
        }
        self.structures
            .get(&slot)
            .map(|entry| entry.baseline.read_only())
            .ok_or_else(|| refuse("实际文档槽位不存在"))
    }
    fn existing_document(&self, path: &Path) -> Result<Option<LoadedDocument>, WorldError> {
        let worlds = self
            .world_slots
            .iter()
            .filter_map(|(slot, index)| (self.inputs[*index].world.root == path).then_some(*slot));
        let structures = self
            .structures
            .iter()
            .filter_map(|(slot, entry)| (entry.baseline.path() == path).then_some(*slot));
        let mut matches = worlds.chain(structures);
        let first = matches.next();
        if matches.next().is_some() {
            return Err(refuse("路径对应多个输入会话，不能猜测要激活的文档"));
        }
        Ok(first.map(|slot| LoadedDocument {
            slot,
            created: false,
        }))
    }
    pub(super) fn reserve_document(&self, bytes: usize) -> Result<usize, WorldError> {
        let retained = self
            .opened_bytes
            .checked_add(bytes)
            .ok_or_else(|| refuse("新文档工作集溢出"))?;
        self.budget.check(
            retained
                .checked_add(self.frozen_bytes)
                .and_then(|bytes| bytes.checked_mul(4))
                .ok_or_else(|| refuse("文档与冻结来源合计工作集溢出"))?,
        )?;
        Ok(retained)
    }
    /// 只准备真实内容；读取、格式、版本或预算错误都不改变当前目录。
    pub fn prepare_document(
        &self,
        path: &Path,
        options: LoadOptions,
    ) -> Result<PreparedDocument, WorldError> {
        self.stop()?;
        if path.as_os_str().is_empty() {
            return Err(refuse("打开路径不能为空"));
        }
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        if let Some(existing) = self.existing_document(&absolute)? {
            return Ok(self.prepared_existing(existing));
        }
        let level_file = matches!(
            absolute.file_name().and_then(|name| name.to_str()),
            Some("level.dat" | "level.dat_old")
        );
        let metadata = std::fs::metadata(&absolute)?;
        if level_file && metadata.is_file() {
            let parent = absolute
                .parent()
                .ok_or_else(|| refuse("世界元数据缺少所属目录"))?;
            if let Some(existing) = self.existing_document(parent)? {
                return Ok(self.prepared_existing(existing));
            }
        }
        if metadata.is_dir() || level_file {
            let source_root = if metadata.is_dir() {
                absolute.as_path()
            } else {
                absolute
                    .parent()
                    .ok_or_else(|| refuse("世界元数据缺少所属目录"))?
            };
            let canonical = std::fs::canonicalize(source_root)?;
            if let Some(existing) = self.existing_document(&canonical)? {
                return Ok(self.prepared_existing(existing));
            }
            let world = crate::inspect_input(&absolute)?;
            self.stop()?;
            if let Some(existing) = self.existing_document(&world.root)? {
                return Ok(self.prepared_existing(existing));
            }
            if crate::mixed_transaction::pending_summary(&world)?.is_some() {
                return Err(refuse("新世界存在未完成保存事务，请先在原生保存恢复页处理"));
            }
            let dimension = match options.dimension {
                Some(dimension) => {
                    if !world.dimensions.iter().any(|entry| entry.name == dimension) {
                        return Err(refuse("新打开世界没有指定的真实维度"));
                    }
                    dimension
                }
                None => world
                    .dimensions
                    .first()
                    .ok_or_else(|| refuse("新打开世界没有实际维度"))?
                    .name
                    .clone(),
            };
            let dimensions = world.dimensions.len();
            let bytes = world
                .world_data
                .allocated_bytes()
                .checked_add(
                    dimensions
                        .checked_mul(std::mem::size_of::<Session>())
                        .ok_or_else(|| refuse("新世界会话工作集溢出"))?,
                )
                .and_then(|bytes| {
                    dimensions
                        .checked_mul(dimensions)
                        .and_then(|count| {
                            count.checked_mul(std::mem::size_of::<crate::DimensionSummary>())
                        })
                        .and_then(|metadata| bytes.checked_add(metadata))
                })
                .ok_or_else(|| refuse("新世界目录工作集溢出"))?;
            self.reserve_document(bytes)?;
            let mut sessions = BTreeMap::new();
            for target in &world.dimensions {
                self.stop()?;
                sessions.insert(
                    target.name.clone(),
                    Session::new(world.clone(), &target.name)?,
                );
            }
            let input = Input {
                world,
                dimension: dimension.clone(),
                selections: sessions.keys().map(|name| (name.clone(), None)).collect(),
                sessions,
                readonly: options.readonly,
            };
            return self.prepared_input(PreparedInput::World(input), bytes);
        }
        if std::fs::symlink_metadata(&absolute)?
            .file_type()
            .is_symlink()
        {
            return Err(refuse("结构文档必须来自实际普通文件"));
        }
        let canonical: PathBuf = std::fs::canonicalize(&absolute)?;
        if let Some(existing) = self.existing_document(&canonical)? {
            return Ok(self.prepared_existing(existing));
        }
        let format = crate::structure_exchange::Format::detect(&canonical)?
            .ok_or_else(|| refuse("该文件没有可建立世界 / 结构会话的实际格式"))?;
        let budget = self.transient_budget()?;
        if crate::structure_document::save::pending(&canonical, budget)?.is_some() {
            return Err(refuse("新结构存在未完成保存事务，请先在原生文件页处理"));
        }
        let document = if options.readonly {
            Document::open_read_only(&canonical, format, options.legacy_version, budget)?
        } else {
            Document::open(&canonical, format, options.legacy_version, budget)?
        };
        self.stop()?;
        let dimension = document.content().dimension.clone();
        if options
            .dimension
            .is_some_and(|requested| requested != dimension)
        {
            return Err(refuse("结构文件没有指定维度"));
        }
        let bytes = document.content().work_bytes()?;
        self.prepared_input(PreparedInput::Structure(document), bytes)
    }
    pub fn install_document(
        &mut self,
        document: PreparedDocument,
    ) -> Result<LoadedDocument, WorldError> {
        self.stop()?;
        if !Arc::ptr_eq(&self.documents_revision, &document.registry_revision) {
            return Err(refuse("文档准备之后目录已经改变，不能安装过期结果"));
        }
        if document.input.is_none() {
            return Ok(LoadedDocument {
                slot: document.slot,
                created: false,
            });
        }
        self.document_budget(&document)?;
        let retained = self.reserve_document(document.bytes)?;
        let slot = document.slot;
        let input = document.input.expect("新增文档的真实内容已在安装前确认");
        match input {
            PreparedInput::World(input) => {
                let index = self.inputs.len();
                for (name, session) in &input.sessions {
                    self.entries.insert(
                        (slot, name.clone()),
                        Entry {
                            baseline: session.clone(),
                            working: session.clone(),
                        },
                    );
                    self.selections.insert((slot, name.clone()), None);
                }
                self.world_slots.insert(slot, index);
                self.dimensions.push(input.dimension.clone());
                self.opened.push(OpenedDocument::World {
                    slot,
                    input: input.clone(),
                });
                self.inputs.push(input);
            }
            PreparedInput::Structure(document) => {
                let dimension = document.content().dimension.clone();
                self.structures.insert(
                    slot,
                    StructureEntry {
                        working: document.script_scratch(),
                        baseline: document.clone(),
                    },
                );
                self.selections.insert((slot, dimension.clone()), None);
                self.dimensions.push(dimension);
                self.opened
                    .push(OpenedDocument::Structure { slot, document });
            }
        }
        self.opened_bytes = retained;
        self.documents_revision = Arc::new(());
        Ok(LoadedDocument {
            slot,
            created: true,
        })
    }
    /// 不需要额外目录预检的调用方可以在一次调用中读取并安装真实文档。
    pub fn load_document(
        &mut self,
        path: &Path,
        options: LoadOptions,
    ) -> Result<LoadedDocument, WorldError> {
        let document = self.prepare_document(path, options)?;
        self.install_document(document)
    }
}
fn world_info(
    input: &Input,
    dimension: &str,
    selections: BTreeMap<String, Option<SelectionRegion>>,
    readonly: bool,
) -> Value {
    json!({"root":input.world.root,"data_version":input.world.data_version,"source_release":input.world.release(),
        "dimensions":input.world.dimensions.iter().map(|dim|dim.name.clone()).collect::<Vec<_>>(),
        "dimension":dimension,"selections":selections,"kind":"world","readonly":readonly || input.readonly})
}
fn structure_info(
    document: &Document,
    dimension: &str,
    selections: BTreeMap<String, Option<SelectionRegion>>,
    readonly: bool,
) -> Value {
    let content = document.content();
    json!({"root":document.path(),"data_version":content.data_version,"source_release":content.source_release,
        "dimensions":[dimension],"dimension":dimension,"selections":selections,"kind":"structure",
        "entity_dimension":content.entity_dimension,"readonly":readonly || document.read_only()})
}
