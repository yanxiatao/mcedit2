//! 启动脚本的原生会话上下文；脚本线程只修改私有会话，不取得文件写入能力。
use super::*;
use crate::structure_context::CodecContext;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[path = "startup_structure.rs"]
mod native;
#[path = "startup_output.rs"]
mod output;
pub use output::AppliedOutput;
#[path = "startup_chunks.rs"]
mod chunks;
#[path = "startup_documents.rs"]
mod documents;
#[path = "startup_objects.rs"]
pub mod objects;
#[path = "startup_opened.rs"]
mod opened;
pub use documents::{LoadOptions, LoadedDocument, OpenedDocument, PreparedDocument};
pub use opened::{OpenedOutput, ResolvedOutput};

#[derive(Debug, Clone)]
pub struct Input {
    pub world: WorldSummary,
    pub dimension: String,
    pub selections: BTreeMap<String, Option<SelectionRegion>>,
    pub sessions: BTreeMap<String, Session>,
    pub readonly: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub world: usize,
    pub dimension: String,
    pub action: Action,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    ObjectOrder {
        tile: bool,
        sources: Vec<objects::Source>,
    },
    ObjectAdd {
        tile: bool,
        nbt: crate::nbt_document::Value,
    },
    Objects {
        region: SelectionRegion,
        tile: bool,
    },
    ObjectRead {
        source: objects::Source,
    },
    ObjectChange {
        source: objects::Source,
        expected: crate::nbt_document::Value,
        nbt: Option<crate::nbt_document::Value>,
    },
    ChunkPositions,
    ChunkInfo {
        position: [i32; 2],
    },
    Bounds,
    CreationProfile {
        biome: Value,
    },
    Read {
        position: [i32; 3],
    },
    Resolve {
        state: State,
    },
    Capture {
        region: SelectionRegion,
        entities: bool,
        biomes: bool,
    },
    CaptureMasked {
        region: SelectionRegion,
        mask: Option<Vec<bool>>,
        entities: bool,
        biomes: bool,
    },
    PasteSource {
        source: u64,
        input: clipboard::ScriptSource,
        destination: [i32; 3],
        create: bool,
        creation: Option<crate::chunk_creation::ChunkCreationProfile>,
    },
    Copy {
        source: u64,
        target: u64,
        input: clipboard::ScriptCopy,
    },
    Generate {
        source: u64,
        input: clipboard::Generated,
    },
    Mask {
        region: SelectionRegion,
    },
    Apply {
        snapshot: BlockClipboard,
        blocks: Vec<State>,
        content: snapshot::Content,
    },
    ApplyFrozen {
        source: u64,
        blocks: Vec<State>,
        content: snapshot::Content,
    },
    Fill {
        region: SelectionRegion,
        state: State,
        only_if: Option<Vec<State>>,
    },
    Delete {
        region: SelectionRegion,
        mode: String,
        keep_ids: BTreeSet<String>,
    },
    Selection {
        region: Option<SelectionRegion>,
    },
    UseDimension,
    Focus {
        position: [f64; 3],
        distance: f64,
    },
    Save,
}
#[derive(Debug, Clone)]
pub struct Focus {
    pub world: usize,
    pub dimension: String,
    pub position: [f64; 3],
    pub distance: f64,
}
#[derive(Debug, Clone)]
pub struct Change {
    pub world: usize,
    pub dimension: String,
    pub prepared: snapshot::Prepared,
    pub baseline: Session,
}
#[derive(Debug, Clone)]
pub struct Output {
    pub sources: Vec<(usize, String, Arc<()>)>,
    pub changes: Vec<Change>,
    pub dimensions: Vec<String>,
    pub selections: BTreeMap<(usize, String), Option<SelectionRegion>>,
    pub focus: Option<Focus>,
    pub save_worlds: Vec<usize>,
    pub save_sessions: Vec<(usize, String, Session)>,
    pub save_order: Vec<usize>,
    pub structures: Vec<crate::structure_document::startup::Output>,
    pub opened: Vec<OpenedDocument>,
}
struct Entry {
    baseline: Session,
    working: Session,
}
enum FrozenContent {
    World(Arc<BlockClipboard>),
    Structure(Arc<crate::structure_document::ScriptSnapshot>),
}
impl FrozenContent {
    fn clipboard(&self) -> &BlockClipboard {
        match self {
            Self::World(clip) => clip,
            Self::Structure(snapshot) => snapshot.clipboard(),
        }
    }
}
struct Frozen {
    world: usize,
    dimension: String,
    content: FrozenContent,
    bytes: usize,
}
struct StructureEntry {
    baseline: crate::structure_document::Document,
    working: crate::structure_document::Document,
}
pub struct Working {
    inputs: Vec<Input>,
    world_slots: BTreeMap<usize, usize>,
    entries: BTreeMap<(usize, String), Entry>,
    dimensions: Vec<String>,
    selections: BTreeMap<(usize, String), Option<SelectionRegion>>,
    focus: Option<Focus>,
    save_worlds: Vec<usize>,
    readonly: bool,
    budget: EditWorkBudget,
    cancelled: Arc<AtomicBool>,
    changed_selections: BTreeSet<(usize, String)>,
    observed: BTreeSet<(usize, String)>,
    snapshots: BTreeMap<u64, Frozen>,
    next_snapshot: u64,
    frozen_bytes: usize,
    structures: BTreeMap<usize, StructureEntry>,
    opened: Vec<OpenedDocument>,
    opened_bytes: usize,
    documents_revision: Arc<()>,
}
impl Working {
    pub fn new(
        inputs: Vec<Input>,
        readonly: bool,
        budget: EditWorkBudget,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, WorldError> {
        Self::new_documents(inputs, Vec::new(), readonly, budget, cancelled)
    }
    pub fn new_documents(
        inputs: Vec<Input>,
        structures: Vec<crate::structure_document::startup::Input>,
        readonly: bool,
        budget: EditWorkBudget,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, WorldError> {
        if inputs.is_empty() && structures.is_empty() {
            return Err(refuse("启动脚本需要实际世界或结构文档"));
        }
        let mut entries = BTreeMap::new();
        let mut selections = BTreeMap::new();
        for (index, input) in inputs.iter().enumerate() {
            if !input
                .world
                .dimensions
                .iter()
                .any(|dim| dim.name == input.dimension)
            {
                return Err(refuse("启动脚本初始维度不存在"));
            }
            for (dimension, selection) in &input.selections {
                if !input
                    .world
                    .dimensions
                    .iter()
                    .any(|dim| dim.name == *dimension)
                {
                    return Err(refuse("启动脚本选区所属维度不存在"));
                }
                selections.insert((index, dimension.clone()), *selection);
            }
            for (dimension, session) in &input.sessions {
                if session.original_world().root != input.world.root
                    || session.dimension() != dimension
                {
                    return Err(refuse("启动脚本输入会话归属不一致"));
                }
                entries.insert(
                    (index, dimension.clone()),
                    Entry {
                        baseline: session.clone(),
                        working: session.clone(),
                    },
                );
            }
        }
        let mut dimensions = inputs
            .iter()
            .map(|input| input.dimension.clone())
            .collect::<Vec<_>>();
        let mut documents = BTreeMap::new();
        for (offset, input) in structures.into_iter().enumerate() {
            let slot = inputs.len() + offset;
            let dimension = input.document.content().dimension.clone();
            let region = input
                .selection
                .map(|region| input.document.script_absolute_region(region))
                .transpose()?;
            selections.insert((slot, dimension.clone()), region);
            dimensions.push(dimension);
            documents.insert(
                slot,
                StructureEntry {
                    working: input.document.script_scratch(),
                    baseline: input.document,
                },
            );
        }
        let world_slots = (0..inputs.len()).map(|index| (index, index)).collect();
        Ok(Self {
            inputs,
            world_slots,
            entries,
            dimensions,
            selections,
            focus: None,
            save_worlds: Vec::new(),
            readonly,
            budget,
            cancelled,
            changed_selections: BTreeSet::new(),
            observed: BTreeSet::new(),
            snapshots: BTreeMap::new(),
            next_snapshot: 0,
            frozen_bytes: 0,
            structures: documents,
            opened: Vec::new(),
            opened_bytes: 0,
            documents_revision: Arc::new(()),
        })
    }
    fn stop(&self) -> Result<(), WorldError> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(refuse("启动脚本已取消"))
        } else {
            Ok(())
        }
    }
    /// Inspector 的外围查询只能读取；目标 NBT 的专用编辑由所属文档单独预检。
    pub fn restrict_to_reads(&mut self) {
        self.readonly = true;
    }
    fn read_only(&self, world: usize) -> bool {
        self.readonly
            || self
                .world_slots
                .get(&world)
                .is_some_and(|index| self.inputs[*index].readonly)
            || self
                .structures
                .get(&world)
                .is_some_and(|entry| entry.working.read_only())
    }
    /// 只比较本轮命令前后的完整领域摘要，既有未保存内容不计作命令新增修改。
    pub fn has_content_changes(&self) -> bool {
        self.entries
            .values()
            .any(|entry| entry.baseline.content_hashes != entry.working.content_hashes)
            || self
                .structures
                .values()
                .any(|entry| entry.baseline.content() != entry.working.content())
    }
    pub fn info(&self) -> Value {
        let documents = (0..self.document_count())
            .map(|slot| {
                self.document_info(slot)
                    .expect("每个已分配槽位都具有实际文档")
            })
            .collect::<Vec<_>>();
        json!({"readonly":self.readonly,"order":(0..documents.len()).collect::<Vec<_>>(),"worlds":documents})
    }
    fn entry(&mut self, world: usize, dimension: &str) -> Result<&mut Entry, WorldError> {
        let input = self.world_input(world)?;
        if !input
            .world
            .dimensions
            .iter()
            .any(|dim| dim.name == dimension)
        {
            return Err(refuse("启动脚本请求未知维度"));
        }
        let key = (world, dimension.to_owned());
        if !self.entries.contains_key(&key) {
            let session = Session::new(input.world.clone(), dimension)?;
            self.entries.insert(
                key.clone(),
                Entry {
                    baseline: session.clone(),
                    working: session,
                },
            );
        }
        Ok(self.entries.get_mut(&key).expect("已初始化脚本会话"))
    }
    /// 每次操作读取本轮脚本的最新实体来源，同时记录正式会话的发布依赖。
    fn bind_entity_context(&mut self, world: usize, dimension: &str) {
        let key = (world, dimension.to_owned());
        let root = self.entries[&key].working.original_world().root.clone();
        let mut peers = Vec::new();
        for (peer_key, entry) in &mut self.entries {
            if peer_key != &key
                && entry.working.original_world().root == root
                && entry.working.has_pending_entity_changes()
            {
                peers.push(entry.working.identity_peer_snapshot());
                self.observed.insert(peer_key.clone());
            }
        }
        self.entries
            .get_mut(&key)
            .expect("脚本目标会话已初始化")
            .working
            .set_identity_peers(peers);
    }
    fn resolve(
        &mut self,
        world: usize,
        dimension: &str,
        state: State,
    ) -> Result<State, WorldError> {
        let budget = self.transient_budget()?;
        let token = Arc::clone(&self.cancelled);
        let session = &self.entry(world, dimension)?.working;
        let chunks =
            session.terrain_chunks_cancellable(budget, || token.load(Ordering::Acquire))?;
        let first = chunks
            .into_iter()
            .next()
            .ok_or_else(|| refuse("空维度没有可核对的方块编码；不能推断数字状态"))?;
        let (chunk, _) = session.visible_chunk_base(first.0, first.1)?;
        let numeric = matches!(
            chunk.ok_or_else(|| refuse("源区块已消失"))?.layout,
            Layout::McRegion128 | Layout::NumericAnvil112
        );
        resolve_state(state, numeric, session.world.release())
    }
    fn frozen(
        &self,
        id: u64,
        world: usize,
        dimension: &str,
    ) -> Result<&BlockClipboard, WorldError> {
        let frozen = self
            .snapshots
            .get(&id)
            .ok_or_else(|| refuse("启动脚本冻结来源不存在"))?;
        if frozen.world != world || frozen.dimension != dimension {
            return Err(refuse("启动脚本冻结来源不属于当前真实维度"));
        }
        Ok(frozen.content.clipboard())
    }
}
fn resolve_state(state: State, numeric: bool, release: Option<&str>) -> Result<State, WorldError> {
    let state = parse_state(&render_state(&state))?;
    if !numeric {
        if state.name.starts_with("numeric:") {
            return Err(refuse("命名来源没有可推测的旧数字 ID 映射"));
        }
        return Ok(state);
    }
    if state.name.starts_with("numeric:") {
        return Ok(state);
    }
    let release = release.ok_or_else(|| refuse("数字来源缺少准确版本，不能解析方块名称"))?;
    let version = release.split('.').take(2).collect::<Vec<_>>().join(".");
    let registry: Value = serde_json::from_str(
        crate::legacy_assets::mapping(&version)
            .ok_or_else(|| refuse("没有该来源版本的数字方块注册表"))?,
    )
    .map_err(|error| refuse(error.to_string()))?;
    let entries = registry["entries"]
        .as_array()
        .ok_or_else(|| refuse("数字注册表缺少 entries"))?;
    let mut exact = BTreeSet::new();
    let mut default = BTreeSet::new();
    for row in entries {
        let row = row.as_array().ok_or_else(|| refuse("数字注册表条目无效"))?;
        let id = row[0].as_i64().ok_or_else(|| refuse("数字注册 ID 无效"))?;
        let meta = row[1]
            .as_i64()
            .ok_or_else(|| refuse("数字注册 Data 无效"))?;
        if row[5].as_str() == Some(render_state(&state).as_str()) {
            exact.insert((id, meta));
        }
        if state.properties.is_empty() && meta == 0 && row[4].as_str() == Some(state.name.as_str())
        {
            default.insert((id, meta));
        }
    }
    let matches = if exact.is_empty() { default } else { exact };
    if matches.len() != 1 {
        return Err(refuse("数字方块名称未知或具有多个准确编码"));
    }
    let (id, meta) = matches.into_iter().next().expect("有唯一注册项");
    parse_state(&format!("numeric:{id}[data={meta}]"))
}
impl Working {
    /// Python 冻结租约的最后一个引用释放时交还原始来源；取消同样允许资源释放。
    pub fn release_snapshot(&mut self, id: u64) -> Result<(), WorldError> {
        let frozen = self
            .snapshots
            .remove(&id)
            .ok_or_else(|| refuse("启动脚本冻结租约已释放或不存在"))?;
        self.frozen_bytes = self
            .frozen_bytes
            .checked_sub(frozen.bytes)
            .expect("冻结工作集与实际租约一致");
        Ok(())
    }
    /// 为仍持有的冻结来源预留工作集，再交给共享内核和 Python 数组分配入口。
    pub fn transient_budget(&self) -> Result<EditWorkBudget, WorldError> {
        let retained = self
            .frozen_bytes
            .checked_mul(4)
            .and_then(|bytes| {
                self.opened_bytes
                    .checked_mul(4)
                    .and_then(|opened| bytes.checked_add(opened))
            })
            .ok_or_else(|| refuse("文档与冻结来源工作集溢出"))?;
        let available = self
            .budget
            .bytes()
            .checked_sub(retained)
            .filter(|bytes| *bytes > 0)
            .ok_or_else(|| refuse("实际文档和仍被引用的冻结来源已占满脚本预算"))?;
        EditWorkBudget::from_bytes(available)
    }
    fn capture(
        &mut self,
        world: usize,
        dimension: String,
        region: SelectionRegion,
        entities: bool,
        biomes: bool,
        mask: Option<Vec<bool>>,
    ) -> Result<Value, WorldError> {
        let budget = self.transient_budget()?;
        let token = Arc::clone(&self.cancelled);
        let options = clipboard::CaptureOptions { entities, biomes };
        let frozen = if let Some(entry) = self.structures.get(&world) {
            FrozenContent::Structure(Arc::new(entry.working.capture_startup(
                region,
                mask,
                entities,
                biomes,
                budget,
                || token.load(Ordering::Acquire),
            )?))
        } else {
            let session = &self.entries[&(world, dimension.clone())].working;
            let clip = if let Some(mut mask) = mask {
                if mask.len() != region.bounds.volume()? {
                    return Err(refuse("启动脚本捕获掩码数量不匹配"));
                }
                if let Some(shape) = region.mask(budget, || token.load(Ordering::Acquire))? {
                    for (selected, shape) in mask.iter_mut().zip(shape) {
                        *selected &= shape;
                    }
                }
                session.capture_structure_masked(
                    region.bounds,
                    Some(mask),
                    options,
                    budget,
                    || token.load(Ordering::Acquire),
                )?
            } else {
                session.capture_structure_with_budget_cancellable(
                    region,
                    options,
                    budget,
                    || token.load(Ordering::Acquire),
                )?
            };
            FrozenContent::World(Arc::new(clip))
        };
        let clip = frozen.clipboard();
        let content = snapshot::Content::capture(clip)?;
        let retained = clip.work_bytes()?;
        let bytes = self
            .frozen_bytes
            .checked_add(retained)
            .ok_or_else(|| refuse("启动脚本保留的冻结来源工作集溢出"))?;
        self.budget.check(
            bytes
                .checked_mul(4)
                .ok_or_else(|| refuse("启动脚本冻结来源工作集溢出"))?,
        )?;
        self.stop()?;
        let context = self.next_snapshot;
        self.next_snapshot = context
            .checked_add(1)
            .ok_or_else(|| refuse("启动脚本冻结身份耗尽"))?;
        let value = json!({"snapshot":&clip,"content":content,"context":context});
        self.snapshots.insert(
            context,
            Frozen {
                world,
                dimension,
                content: frozen,
                bytes: retained,
            },
        );
        self.frozen_bytes = bytes;
        Ok(value)
    }
    pub fn invoke(&mut self, call: Call) -> Result<Value, WorldError> {
        match call {
            Call {
                world,
                dimension,
                action: Action::ObjectOrder { tile, sources },
            } => self.script_object_order(world, &dimension, tile, sources),
            Call {
                world,
                dimension,
                action: Action::ObjectAdd { tile, nbt },
            } => self.script_object_add(world, &dimension, tile, nbt),
            Call {
                world,
                dimension,
                action: Action::Objects { region, tile },
            } => self.script_objects(world, &dimension, region, tile),
            Call {
                world,
                dimension,
                action: Action::ObjectRead { source },
            } => self.script_object(world, &dimension, source),
            Call {
                world,
                dimension,
                action:
                    Action::ObjectChange {
                        source,
                        expected,
                        nbt,
                    },
            } => self.script_object_change(world, &dimension, source, expected, nbt),
            Call {
                world,
                dimension,
                action: Action::ChunkPositions,
            } => self.script_chunk_positions(world, &dimension),
            Call {
                world,
                dimension,
                action: Action::ChunkInfo { position },
            } => self.script_chunk_info(world, &dimension, position),
            Call {
                world,
                dimension,
                action:
                    Action::ApplyFrozen {
                        source,
                        blocks,
                        content,
                    },
            } => {
                let frozen = self
                    .snapshots
                    .get(&source)
                    .ok_or_else(|| refuse("脚本冻结来源已释放"))?;
                if frozen.world != world || frozen.dimension != dimension {
                    return Err(refuse("脚本冻结来源与调用文档不一致"));
                }
                self.apply_frozen_output(source, &blocks, &content, None, None)
                    .map(|output| json!(output.changed))
            }
            call => self.invoke_call(call),
        }
    }
    fn invoke_call(&mut self, call: Call) -> Result<Value, WorldError> {
        self.stop()?;
        if self.structures.contains_key(&call.world) {
            return self.invoke_structure(call);
        }
        self.entry(call.world, &call.dimension)?;
        self.bind_entity_context(call.world, &call.dimension);
        let key = (call.world, call.dimension.clone());
        self.observed.insert(key.clone());
        let token = Arc::clone(&self.cancelled);
        let validate = |region: SelectionRegion| -> Result<SelectionRegion, WorldError> {
            let bounds =
                Bounds::spatial(region.bounds.minimum.into(), region.bounds.maximum.into())?;
            if bounds != region.bounds {
                return Err(refuse("脚本选区边界颠倒"));
            }
            region.resolved(&self.world_input(call.world)?.world, &call.dimension)
        };
        match call.action {
            Action::CreationProfile { biome } => {
                use crate::chunk_creation::{BiomeSource, ChunkCreationProfile};
                let origin = "启动脚本明确指定的初始群系".to_owned();
                let (initial, source) = match biome {
                    Value::String(name) => {
                        let entries = BTreeSet::from([name.clone()]);
                        (name, BiomeSource::UserSpecified { origin, entries })
                    }
                    Value::Number(value) => {
                        let id = value
                            .as_u64()
                            .and_then(|value| u8::try_from(value).ok())
                            .ok_or_else(|| refuse("旧数字初始群系必须是 0..255 的实际整数 ID"))?;
                        (id.to_string(), BiomeSource::LegacyNumeric { origin, id })
                    }
                    _ => return Err(refuse("初始群系需要实际命名 ID 或旧数字 ID")),
                };
                let profile = ChunkCreationProfile::from_level(
                    &self.world_input(call.world)?.world,
                    &call.dimension,
                    initial,
                    source,
                )?;
                return serde_json::to_value(profile).map_err(|error| refuse(error.to_string()));
            }
            Action::Bounds => {
                let scan_budget = self.transient_budget()?;
                return serde_json::to_value(
                    self.entries[&key]
                        .working
                        .dimension_extent_cancellable(scan_budget, || {
                            token.load(Ordering::Acquire)
                        })?,
                )
                .map_err(|error| refuse(error.to_string()));
            }
            Action::Selection { region } => {
                let region = region.map(validate).transpose()?;
                self.changed_selections.insert(key.clone());
                self.selections.insert(key, region);
                return serde_json::to_value(region).map_err(|error| refuse(error.to_string()));
            }
            Action::UseDimension => {
                self.dimensions[call.world] = call.dimension;
                return Ok(Value::Null);
            }
            Action::Focus { position, distance } => {
                if !position.iter().all(|value| value.is_finite())
                    || !distance.is_finite()
                    || distance <= 0.
                {
                    return Err(refuse("定位需要有限坐标及正距离"));
                }
                self.focus = Some(Focus {
                    world: call.world,
                    dimension: call.dimension,
                    position,
                    distance,
                });
                return Ok(Value::Null);
            }
            Action::Save => {
                if self.read_only(call.world) {
                    return Err(refuse("只读模式禁止保存启动脚本更改"));
                }
                if !self.save_worlds.contains(&call.world) {
                    self.save_worlds.push(call.world);
                }
                return Ok(Value::Null);
            }
            Action::Resolve { state } => {
                return serde_json::to_value(self.resolve(call.world, &call.dimension, state)?)
                    .map_err(|error| refuse(error.to_string()))
            }
            Action::Read { position } => {
                let session = &self.entries[&key].working;
                return match session.visible_block(position.into())? {
                    Lookup::Present(value) => {
                        serde_json::to_value(value.state).map_err(|error| refuse(error.to_string()))
                    }
                    Lookup::NoChunk => Err(refuse("此位置没有实际区块")),
                    Lookup::NoSection => Err(refuse("此位置没有实际区段")),
                };
            }
            Action::Capture {
                region,
                entities,
                biomes,
            } => {
                let region = validate(region)?;
                return self.capture(call.world, call.dimension, region, entities, biomes, None);
            }
            Action::CaptureMasked {
                region,
                mask,
                entities,
                biomes,
            } => {
                let region = validate(region)?;
                return self.capture(call.world, call.dimension, region, entities, biomes, mask);
            }
            Action::PasteSource {
                source,
                input,
                destination,
                create,
                creation,
            } => {
                if self.read_only(call.world) {
                    return Err(refuse("只读模式禁止复制到启动脚本的世界会话"));
                }
                if !create && creation.is_some() {
                    return Err(refuse("提供区块创建合同时必须明确启用 create"));
                }
                let source = &self
                    .snapshots
                    .get(&source)
                    .ok_or_else(|| refuse("脚本复制的真实冻结来源已释放"))?
                    .content;
                let budget = self.transient_budget()?;
                let biomes = input.biomes;
                let clip = source
                    .clipboard()
                    .script_source_cancellable(input, budget, || token.load(Ordering::Acquire))?;
                let entry = &self.entries[&key];
                let mut work = entry.working.clone();
                let prepared = work.prepare_structure_paste_with_budget_cancellable(
                    &clip,
                    destination.into(),
                    clipboard::PasteOptions {
                        include_biomes: biomes,
                    },
                    creation,
                    budget,
                    || token.load(Ordering::Acquire),
                )?;
                let changed =
                    work.commit_structure_cancellable(prepared, || token.load(Ordering::Acquire))?;
                work.fold_script_history(&entry.baseline, budget)?;
                self.stop()?;
                self.entries
                    .get_mut(&key)
                    .expect("脚本目标会话存在")
                    .working = work;
                return Ok(json!(changed));
            }
            Action::Copy {
                source,
                target,
                input,
            } => {
                let source = self
                    .snapshots
                    .get(&source)
                    .ok_or_else(|| refuse("复制的实际冻结来源已经释放"))?
                    .content
                    .clipboard();
                let target = self.frozen(target, call.world, &call.dimension)?;
                let result = source.copy_script_dimension_cancellable(
                    &target,
                    input,
                    self.transient_budget()?,
                    || token.load(Ordering::Acquire),
                )?;
                self.stop()?;
                return serde_json::to_value(result).map_err(|error| refuse(error.to_string()));
            }
            Action::Generate { source, input } => {
                let source = self.frozen(source, call.world, &call.dimension)?;
                let result = source.materialize_generated(input, self.transient_budget()?)?;
                self.stop()?;
                return serde_json::to_value(result).map_err(|error| refuse(error.to_string()));
            }
            Action::Mask { region } => {
                let region = validate(region)?;
                return serde_json::to_value(
                    region.mask(self.transient_budget()?, || token.load(Ordering::Acquire))?,
                )
                .map_err(|error| refuse(error.to_string()));
            }
            action => {
                if self.read_only(call.world) {
                    if let Action::Apply {
                        snapshot,
                        blocks,
                        content,
                    } = &action
                    {
                        if snapshot.blocks == *blocks
                            && snapshot::Content::capture(snapshot)? == *content
                        {
                            return Ok(json!(0));
                        }
                    }
                    return Err(refuse("只读模式禁止修改启动脚本的世界会话"));
                }
                let budget = self.transient_budget()?;
                let entry = self.entries.get(&key).expect("已初始化脚本会话");
                let mut work = entry.working.clone();
                let changed = match action {
                    Action::Apply {
                        snapshot,
                        blocks,
                        content,
                    } => {
                        if let Some(prepared) = work.prepare_snapshot_difference_cancellable(
                            &snapshot,
                            &blocks,
                            &content,
                            budget,
                            || token.load(Ordering::Acquire),
                        )? {
                            work.commit_snapshot_difference_cancellable(
                                prepared,
                                || token.load(Ordering::Acquire),
                                |_| {},
                            )?
                        } else {
                            0
                        }
                    }
                    Action::Fill {
                        region,
                        state,
                        only_if,
                    } => {
                        let region = validate(region)?;
                        let snapshot = work.capture_block_states_with_budget_cancellable(
                            region,
                            budget,
                            || token.load(Ordering::Acquire),
                        )?;
                        let blocks = snapshot
                            .blocks
                            .iter()
                            .enumerate()
                            .map(|(index, before)| {
                                if snapshot.mask.as_ref().is_none_or(|mask| mask[index])
                                    && only_if
                                        .as_ref()
                                        .is_none_or(|filter| filter.contains(before))
                                {
                                    state.clone()
                                } else {
                                    before.clone()
                                }
                            })
                            .collect::<Vec<_>>();
                        if let Some(prepared) = work
                            .prepare_block_difference_with_budget_cancellable(
                                &snapshot,
                                &blocks,
                                budget,
                                || token.load(Ordering::Acquire),
                            )?
                        {
                            work.commit_structure_cancellable(prepared, || {
                                token.load(Ordering::Acquire)
                            })?
                        } else {
                            0
                        }
                    }
                    Action::Delete {
                        region,
                        mode,
                        keep_ids,
                    } => {
                        let region = validate(region)?;
                        let mode = match mode.as_str() {
                            "blocks" => deletion::Mode::Blocks,
                            "entities" => deletion::Mode::Entities,
                            "entities_without_items" => deletion::Mode::EntitiesWithoutItems,
                            "all" => deletion::Mode::All,
                            _ => return Err(refuse("未知脚本删除方式")),
                        };
                        let prepared = work.prepare_selection_deletion_cancellable(
                            deletion::Options {
                                region,
                                mode,
                                keep_ids,
                            },
                            budget,
                            || token.load(Ordering::Acquire),
                            |_| {},
                        )?;
                        work.commit_selection_deletion_cancellable(
                            prepared,
                            || token.load(Ordering::Acquire),
                            |_| {},
                        )?
                    }
                    _ => unreachable!("非修改操作已单独处理"),
                };
                work.fold_script_history(&entry.baseline, budget)?;
                if token.load(Ordering::Acquire) {
                    return Err(refuse("启动脚本已取消"));
                }
                self.entries
                    .get_mut(&key)
                    .expect("已初始化脚本会话")
                    .working = work;
                Ok(json!(changed))
            }
        }
    }
    pub fn finish(mut self) -> Result<Output, WorldError> {
        self.stop()?;
        self.snapshots.clear();
        let sources = self
            .observed
            .iter()
            .filter(|key| self.world_slots.contains_key(&key.0))
            .map(|key| (key.0, key.1.clone(), self.entries[key].baseline.revision()))
            .collect();
        let mut changes = Vec::new();
        let mut save_sessions = Vec::new();
        let mut structures = Vec::new();
        for (slot, entry) in self.structures {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(refuse("启动脚本结构结果整理已取消"));
            }
            if self.observed.iter().any(|key| key.0 == slot) {
                let document = entry.baseline.finish_startup(&entry.working, self.budget)?;
                structures.push(crate::structure_document::startup::Output {
                    slot,
                    baseline: entry.baseline,
                    document,
                    save: self.save_worlds.contains(&slot),
                });
            }
        }
        for ((world, dimension), entry) in self.entries {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(refuse("启动脚本世界结果整理已取消"));
            }
            if self.save_worlds.contains(&world) && entry.working.has_pending_changes() {
                save_sessions.push((world, dimension.clone(), entry.working.clone()));
            }
            if let Some(prepared) = entry
                .baseline
                .finish_script_candidate(entry.working, self.budget)?
            {
                changes.push(Change {
                    world,
                    dimension,
                    prepared,
                    baseline: entry.baseline,
                });
            }
        }
        Ok(Output {
            sources,
            changes,
            dimensions: self.dimensions,
            selections: self
                .selections
                .into_iter()
                .filter(|(key, _)| self.changed_selections.contains(key))
                .collect(),
            focus: self.focus,
            save_order: self.save_worlds.clone(),
            save_worlds: self
                .save_worlds
                .into_iter()
                .filter(|slot| self.world_slots.contains_key(slot))
                .collect(),
            save_sessions,
            structures,
            opened: self.opened,
        })
    }
}
