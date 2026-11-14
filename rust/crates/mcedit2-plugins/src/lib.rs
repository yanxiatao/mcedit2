//! 插件运行只产生候选数据，由原生世界接口验证后转为可撤销的预览。
//! RustPython 在独立进程中执行，插件不取得 Session 或世界文件写入句柄。
pub mod catalog;
pub mod document;
mod process;
#[cfg(windows)]
mod process_owner;
pub mod resources;
mod tool_process;
pub use tool_process::{run_native, ToolRuntime};
mod work;
pub use work::work_budget;
pub mod transport;
pub use process::{run, RunLimits};

use mcedit2_world::{
    clipboard::BlockClipboard,
    editing::{Bounds, EditWorkBudget, Preview, Session},
    sections::State,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod object_receipt;
pub mod tool_input;
pub mod tool_pick;
pub mod tool_rpc;
pub mod tool_scene;

pub const PROTOCOL_VERSION: u32 = 4;
pub mod application;
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PYTHON_SOURCE_BYTES: usize = 4 * 1024 * 1024;

/// 与入口一起读取、校验并传入进程的纯 Python 模块快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PythonModule {
    pub source: String,
    pub filename: String,
    pub is_package: bool,
}

pub fn valid_module_component(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c == '_' || c.is_alphanumeric())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub protocol_version: u32,
    pub work_budget_bytes: u64,
    pub source: String,
    pub filename: String,
    pub package: bool,
    /// 键为相对入口包的点分模块名；不能替换解释器或宿主接口模块。
    pub modules: BTreeMap<String, PythonModule>,
    /// None 只发现命令与选项，不调用用户命令。
    pub command: Option<String>,
    pub options: BTreeMap<String, serde_json::Value>,
    /// 加载参数表单时的实际选区几何；不包含方块或世界读取权限。
    #[serde(default)]
    pub selection: Option<mcedit2_world::editing::SelectionRegion>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub form_action: Option<String>,
    #[serde(default)]
    pub form_initialize: bool,
    #[serde(default)]
    pub form_source: Option<InspectorSource>,
    #[serde(default)]
    pub tool_event: Option<ToolEvent>,
    #[serde(default)]
    pub gesture: Option<tool_input::Gesture>,
    #[serde(default)]
    pub tool_phase: Option<tool_input::ToolPhase>,
    #[serde(default)]
    pub inspector: Option<InspectorContext>,
    pub snapshot: Option<BlockClipboard>,
    #[serde(default)]
    pub object_receipt: Option<object_receipt::Receipt>,
    #[serde(default)]
    pub native_source: Option<tool_rpc::Source>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectorContext {
    pub position: [i32; 3],
    pub source_sha: String,
    pub path: String,
    pub nbt: mcedit2_world::nbt_document::Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectorSource {
    pub position: [i32; 3],
    pub source_sha: String,
    pub path: String,
    pub value_sha: String,
}
impl InspectorContext {
    pub fn binding(&self) -> Result<InspectorSource, String> {
        use sha2::{Digest, Sha256};
        let bytes = self.nbt.payload().map_err(|error| error.to_string())?;
        Ok(InspectorSource {
            position: self.position,
            source_sha: self.source_sha.clone(),
            path: self.path.clone(),
            value_sha: format!("{:x}", Sha256::digest(bytes)),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolEvent {
    pub hit: [i32; 3],
    pub state: State,
    pub view: String,
    pub camera: Option<[f64; 3]>,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan: [f32; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCommand {
    pub id: String,
    pub name: String,
    pub options: Vec<serde_json::Value>,
    /// 声明式控件变化后以同一候选流程重建生成预览，不直接提交编辑。
    #[serde(default)]
    pub preview_on_change: bool,
    /// 旧 Inspector 类精确匹配实际 NBT ID，空列表表示普通原生回调。
    #[serde(default)]
    pub inspector_ids: Vec<String>,
    #[serde(default)]
    pub inspector_source: Option<InspectorSource>,
    #[serde(default)]
    pub gesture_input: bool,
    /// 激活期间复用独立解释器和工具状态；每次输入仍读取新的真实来源。
    #[serde(default)]
    pub persistent_tool: bool,
    #[serde(default)]
    pub action: Option<tool_pick::ActionState>,
    /// 类注册时已明确声明的图标；真实运行时动作存在时使用动作自身的图标状态。
    #[serde(default)]
    pub icon: Option<Vec<tool_pick::IconFile>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InspectorPatch {
    Single(mcedit2_world::nbt_document::Operation),
    Batch(Vec<mcedit2_world::nbt_document::Operation>),
}
impl InspectorPatch {
    pub fn operations(&self) -> &[mcedit2_world::nbt_document::Operation] {
        match self {
            Self::Single(operation) => std::slice::from_ref(operation),
            Self::Batch(operations) => operations,
        }
    }
    pub fn into_operations(self) -> Vec<mcedit2_world::nbt_document::Operation> {
        match self {
            Self::Single(operation) => vec![operation],
            Self::Batch(operations) => operations,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginMessage {
    pub level: String,
    pub text: String,
}
/// 插件进程和原生启动脚本共用日志语义；完整记录内容，未知级别正常报告。
pub fn record_message(source: &str, message: &PluginMessage) -> Result<(), String> {
    let level = match message.level.as_str() {
        "info" => log::Level::Info,
        "warning" => log::Level::Warn,
        "error" => log::Level::Error,
        "debug" => log::Level::Debug,
        _ => return Err("脚本日志级别无效".into()),
    };
    log::log!(target:"mcedit2::plugin",level,"{source}：{}",message.text);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutput {
    pub protocol_version: u32,
    pub commands: Vec<PluginCommand>,
    pub messages: Vec<PluginMessage>,
    /// 完整选区的最终状态；读取命令可以返回与输入相同的状态。
    pub blocks: Option<Vec<State>>,
    /// 完整选区对象结果；None 明确表示未请求对象修改，不表示清空原有对象。
    #[serde(default)]
    pub content: Option<mcedit2_world::editing::snapshot::Content>,
    /// 生成器返回的统一结构；所有附加载荷由原生粘贴预检，不允许丢弃。
    #[serde(default)]
    pub structure: Option<BlockClipboard>,
    #[serde(default)]
    pub inspector_operation: Option<InspectorPatch>,
    #[serde(default)]
    pub tool_scene: Option<tool_scene::Scene>,
    #[serde(default)]
    pub tool_pick: Option<tool_pick::Pick>,
    #[serde(default)]
    pub object_bindings: Option<object_receipt::Bindings>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunnerReply {
    Success { output: RunOutput },
    Error { message: String },
}

impl RunRequest {
    /// 原生宿主绑定真实插件定义、命令或目录查询及文档来源，触点范围可随输入变化。
    pub fn tool_session_key(&self) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        let origin = if let Some(snapshot) = &self.snapshot {
            serde_json::json!({"root":snapshot.source_root,"dimension":snapshot.dimension,
                "entity_dimension":snapshot.entity_dimension,"data_version":snapshot.data_version,
                "numeric":snapshot.numeric,"source_release":snapshot.source_release})
        } else {
            let source = &self
                .native_source
                .as_ref()
                .ok_or("原生插件宿主需要实际文档来源")?
                .document;
            serde_json::json!({"root":source["root"],"dimension":source["dimension"],
                "entity_dimension":source["entity_dimension"],"data_version":source["data_version"],
                "kind":source["kind"],"source_release":source["source_release"]})
        };
        let bytes = serde_json::to_vec(&(
            self.protocol_version,
            self.work_budget_bytes,
            &self.source,
            &self.filename,
            self.package,
            &self.modules,
            &self.command,
            self.read_only,
            origin,
        ))
        .map_err(|error| error.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    pub fn inspect(source: &catalog::PluginSource) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            work_budget_bytes: EditWorkBudget::default().bytes() as u64,
            source: source.text.clone(),
            filename: if source.package {
                source.path.join("__init__.py")
            } else {
                source.path.clone()
            }
            .display()
            .to_string(),
            package: source.package,
            modules: source.modules.clone(),
            command: None,
            options: BTreeMap::new(),
            selection: None,
            read_only: false,
            form_action: None,
            form_initialize: false,
            form_source: None,
            tool_event: None,
            gesture: None,
            tool_phase: None,
            inspector: None,
            snapshot: None,
            object_receipt: None,
            native_source: None,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.validate_context(self.native_source.is_some())
    }
    pub(crate) fn validate_native(&self) -> Result<(), String> {
        self.validate_context(true)
    }
    fn validate_context(&self, native: bool) -> Result<(), String> {
        if let Some(receipt) = &self.object_receipt {
            if !self
                .command
                .as_deref()
                .is_some_and(|id| id.starts_with("tool:"))
                || self.snapshot.is_none()
            {
                return Err("对象回执只属于持久工具的实际文档请求".into());
            }
            object_receipt::validate_budget(receipt, self.work_budget_bytes)?;
        }
        if self.tool_phase.is_some()
            && (!self
                .command
                .as_deref()
                .is_some_and(|id| id.starts_with("tool:"))
                || self.tool_event.is_some()
                || self.gesture.is_some()
                || self.form_initialize
                || (self.tool_phase != Some(tool_input::ToolPhase::Options)
                    && self.form_action.is_some())
                || self.inspector.is_some())
        {
            return Err("工具生命周期阶段不能混入鼠标、Inspector 或其他表单初始化请求".into());
        }
        if let Some(selection) = self.selection {
            selection
                .bounds
                .volume()
                .map_err(|error| error.to_string())?;
        }
        if let Some(gesture) = &self.gesture {
            gesture.validate()?;
            if !self
                .command
                .as_deref()
                .is_some_and(|id| id.starts_with("tool:"))
                || self.tool_event.is_some()
            {
                return Err("完整手势只能用于工具命令，不能同时携带单点事件".into());
            }
        }
        let budget = work_budget(self.work_budget_bytes)?;
        if self.protocol_version != PROTOCOL_VERSION
            || self.filename.contains('\0')
            || self.command.as_ref().is_some_and(|v| v.is_empty())
            || self
                .form_action
                .as_ref()
                .is_some_and(|name| name.is_empty())
            || (self.form_initialize && self.form_action.is_some())
        {
            return Err("插件请求版本、源码或命令无效".into());
        }
        if !self.package && !self.modules.is_empty() {
            return Err("插件模块需要 Python 包入口".into());
        }
        if self.package {
            let normalized = self.filename.replace('\\', "/");
            let mut parts = normalized.rsplit('/');
            if parts.next() != Some("__init__.py")
                || !parts.next().is_some_and(valid_module_component)
            {
                return Err("Python 包入口必须是有效模块目录中的 __init__.py".into());
            }
        }
        let mut source_bytes = self.source.len();
        for (name, module) in &self.modules {
            if !name.split('.').all(valid_module_component) || module.filename.contains('\0') {
                return Err(format!("无效 Python 模块快照：{name}"));
            }
            if let Some((parent, _)) = name.rsplit_once('.') {
                if !self.modules.get(parent).is_some_and(|m| m.is_package) {
                    return Err(format!("Python 模块缺少父包：{name}"));
                }
            }
            source_bytes += module.source.len();
        }
        budget
            .check(source_bytes.checked_mul(16).ok_or("插件源码工作集溢出")?)
            .map_err(|error| error.to_string())?;
        let option_bytes = transport::serialized_size(&self.options, self.work_budget_bytes)?;
        let inspector_bytes = transport::serialized_size(&self.inspector, self.work_budget_bytes)?;
        let gesture_bytes = transport::serialized_size(&self.gesture, self.work_budget_bytes)?;
        budget
            .check(
                usize::try_from(
                    option_bytes
                        .checked_add(inspector_bytes)
                        .and_then(|n| n.checked_add(gesture_bytes))
                        .and_then(|n| n.checked_mul(16))
                        .ok_or("插件上下文工作集溢出")?,
                )
                .map_err(|_| "插件上下文超出地址范围")?,
            )
            .map_err(|error| error.to_string())?;
        if source_bytes > MAX_PYTHON_SOURCE_BYTES {
            return Err("插件源码总量超过 4 MiB".into());
        }
        if self.tool_event.as_ref().is_some_and(|event| {
            event
                .camera
                .is_some_and(|camera| camera.iter().any(|v| !v.is_finite()))
                || event.pan.iter().any(|v| !v.is_finite())
                || event.view.is_empty()
                || !event.yaw.is_finite()
                || !event.pitch.is_finite()
                || !event.zoom.is_finite()
                || event.zoom <= 0.
        }) {
            return Err("插件工具相机上下文无效".into());
        }
        if self.command.is_some() && self.snapshot.is_none() && !native {
            return Err("执行插件需要实际原生文档或已读取的原始选区".into());
        }
        if let Some(snapshot) = &self.snapshot {
            work::check_snapshot(snapshot, budget)?;
            if snapshot.offset != [0; 3] || snapshot.metadata.is_some() {
                return Err("脚本选区不能携带结构偏移或文件级元数据".into());
            }
            let count = snapshot
                .size
                .iter()
                .try_fold(1usize, |n, &v| n.checked_mul(v as usize));
            if snapshot.blocks.is_empty() || count != Some(snapshot.blocks.len()) {
                return Err("插件选区必须包含非空源方块且尺寸一致".into());
            }
            if let Some(gesture) = self.gesture.as_ref().filter(|_| !native) {
                gesture.verify_snapshot(snapshot)?;
            }
        }
        Ok(())
    }
}

pub fn capture(session: &Session, bounds: Bounds) -> Result<BlockClipboard, String> {
    session.capture_clipboard(bounds).map_err(|e| e.to_string())
}
pub fn capture_cancellable(
    session: &Session,
    bounds: Bounds,
    cancelled: impl FnMut() -> bool,
) -> Result<BlockClipboard, String> {
    capture_with_budget_cancellable(session, bounds, EditWorkBudget::default(), cancelled)
}
pub fn capture_with_budget_cancellable(
    session: &Session,
    region: impl Into<mcedit2_world::editing::SelectionRegion>,
    budget: EditWorkBudget,
    cancelled: impl FnMut() -> bool,
) -> Result<BlockClipboard, String> {
    let region = region
        .into()
        .resolved(session.original_world(), session.dimension())
        .map_err(|e| e.to_string())?;
    budget
        .check(
            region
                .bounds
                .volume()
                .map_err(|error| error.to_string())?
                .checked_mul(2048)
                .ok_or("插件选区工作集溢出")?,
        )
        .map_err(|error| error.to_string())?;
    let snapshot = session
        .capture_block_states_with_budget_cancellable(region, budget, cancelled)
        .map_err(|error| error.to_string())?;
    work::check_snapshot(&snapshot, budget)?;
    Ok(snapshot)
}
/// 原生脚本快照包含完整对象；群系按明确开关捕获，不编造未保存的群系。
pub fn capture_snapshot_with_budget_cancellable(
    session: &Session,
    region: impl Into<mcedit2_world::editing::SelectionRegion>,
    biomes: bool,
    budget: EditWorkBudget,
    cancelled: impl FnMut() -> bool,
) -> Result<BlockClipboard, String> {
    let snapshot = session
        .capture_structure_with_budget_cancellable(
            region,
            mcedit2_world::clipboard::CaptureOptions {
                entities: true,
                biomes,
            },
            budget,
            cancelled,
        )
        .map_err(|error| error.to_string())?;
    work::check_snapshot(&snapshot, budget)?;
    Ok(snapshot)
}
pub fn prepare_snapshot_output_with_budget_cancellable(
    session: &Session,
    snapshot: &BlockClipboard,
    output: &RunOutput,
    budget: EditWorkBudget,
    cancelled: impl FnMut() -> bool,
) -> Result<Option<mcedit2_world::editing::snapshot::Prepared>, String> {
    output.validate_with_budget(budget)?;
    if output.structure.is_some() || output.inspector_operation.is_some() {
        return Err("普通快照差异不能同时返回生成结构或 Inspector 操作".into());
    }
    let original;
    let content = if let Some(content) = &output.content {
        content
    } else {
        original = mcedit2_world::editing::snapshot::Content::capture(snapshot)
            .map_err(|error| error.to_string())?;
        &original
    };
    session
        .prepare_snapshot_difference_cancellable(
            snapshot,
            output.blocks.as_deref().ok_or("插件执行结果缺少选区方块")?,
            content,
            budget,
            cancelled,
        )
        .map_err(|error| error.to_string())
}
/// 普通命令同时编辑冻结源并返回结构，复用领域差异和生成预检并保留完整顺序。
pub fn prepare_composed_output_with_budget_cancellable(
    session: &Session,
    snapshot: &BlockClipboard,
    output: &RunOutput,
    profile: Option<mcedit2_world::chunk_creation::ChunkCreationProfile>,
    budget: EditWorkBudget,
    cancelled: impl FnMut() -> bool,
) -> Result<Option<mcedit2_world::editing::snapshot::Prepared>, String> {
    output.validate_with_budget(budget)?;
    if output.inspector_operation.is_some() {
        return Err("Inspector 操作不能夹带普通选区修改或结构".into());
    }
    let original;
    let content = if let Some(content) = &output.content {
        content
    } else {
        original = mcedit2_world::editing::snapshot::Content::capture(snapshot)
            .map_err(|error| error.to_string())?;
        &original
    };
    session
        .prepare_script_composition_cancellable(
            snapshot,
            output.blocks.as_deref().ok_or("组合脚本缺少源方块结果")?,
            content,
            output.structure.as_ref().ok_or("组合脚本没有返回结构")?,
            profile,
            budget,
            cancelled,
        )
        .map_err(|error| error.to_string())
}
/// 校验整个输入选区仍与运行前一致，再委托已有原生粘贴验证生成预览。
pub fn prepare_output(
    session: &Session,
    snapshot: &BlockClipboard,
    output: &RunOutput,
) -> Result<Option<Preview>, String> {
    prepare_output_with_budget_cancellable(
        session,
        snapshot,
        output,
        EditWorkBudget::default(),
        || false,
    )
}
pub fn prepare_output_with_budget_cancellable(
    session: &Session,
    snapshot: &BlockClipboard,
    output: &RunOutput,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<Option<Preview>, String> {
    verify_snapshot(session, snapshot, output, budget, &mut cancelled)?;
    if let Some(content) = &output.content {
        if mcedit2_world::editing::snapshot::Content::capture(snapshot)
            .map_err(|error| error.to_string())?
            != *content
        {
            return Err("该入口只处理方块结果；完整对象差异须使用快照差异入口".into());
        }
    }
    if let Some(structure) = &output.structure {
        structure
            .validate_with_budget(session.original_world(), session.dimension(), budget)
            .map_err(|e| e.to_string())?;
        let preview = session
            .prepare_paste_clipboard_with_budget_cancellable(
                structure,
                snapshot.source_min.into(),
                budget,
                &mut cancelled,
            )
            .map_err(|e| e.to_string())?;
        return Ok(Some(preview));
    }
    let blocks = output.blocks.as_ref().ok_or("插件执行结果缺少选区状态")?;
    if blocks.len() != snapshot.blocks.len() {
        return Err("插件结果改变了选区尺寸".into());
    }
    if blocks == &snapshot.blocks {
        return Ok(None);
    }
    if snapshot.mask.as_ref().is_some_and(|mask| {
        snapshot
            .blocks
            .iter()
            .zip(blocks)
            .enumerate()
            .any(|(i, (a, b))| !mask[i] && a != b)
    }) {
        return Err("插件试图改变选区形状外的方块".into());
    }
    let mut candidate = snapshot.clone();
    candidate.blocks = blocks.clone();
    let preview = session
        .prepare_paste_clipboard_with_budget_cancellable(
            &candidate,
            snapshot.source_min.into(),
            budget,
            &mut cancelled,
        )
        .map_err(|e| e.to_string())?;
    Ok(Some(preview))
}

fn verify_snapshot(
    session: &Session,
    snapshot: &BlockClipboard,
    output: &RunOutput,
    budget: EditWorkBudget,
    cancelled: impl FnMut() -> bool,
) -> Result<(), String> {
    output.validate_with_budget(budget)?;
    snapshot
        .validate_with_budget(session.original_world(), session.dimension(), budget)
        .map_err(|e| e.to_string())?;
    if snapshot.source_bound {
        session
            .verify_structure_snapshot_with_budget_cancellable(snapshot, budget, cancelled)
            .map_err(|error| error.to_string())?;
    } else {
        session
            .verify_block_snapshot_with_budget_cancellable(snapshot, budget, cancelled)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 生成器的结构结果沿原生完整结构预检；实体等未支持载荷由内核拒绝。
pub fn prepare_generated_structure(
    session: &Session,
    snapshot: &BlockClipboard,
    output: &RunOutput,
    profile: Option<mcedit2_world::chunk_creation::ChunkCreationProfile>,
    cancelled: impl FnMut() -> bool,
) -> Result<mcedit2_world::editing::PreparedStructure, String> {
    prepare_generated_structure_with_budget(
        session,
        snapshot,
        output,
        profile,
        EditWorkBudget::default(),
        cancelled,
    )
}
pub fn prepare_generated_structure_with_budget(
    session: &Session,
    snapshot: &BlockClipboard,
    output: &RunOutput,
    profile: Option<mcedit2_world::chunk_creation::ChunkCreationProfile>,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
) -> Result<mcedit2_world::editing::PreparedStructure, String> {
    verify_snapshot(session, snapshot, output, budget, &mut cancelled)?;
    if !output.keeps_snapshot(snapshot)? {
        return Err("生成器不能同时修改源选区；请通过返回的完整结构表达生成结果".into());
    }
    let structure = output.structure.as_ref().ok_or("生成器未返回结构")?;
    structure
        .validate_with_budget(session.original_world(), session.dimension(), budget)
        .map_err(|e| e.to_string())?;
    session
        .prepare_structure_paste_with_budget_cancellable(
            structure,
            snapshot.source_min.into(),
            mcedit2_world::clipboard::PasteOptions {
                include_biomes: structure.biomes.is_some() || structure.numeric_biomes.is_some(),
            },
            profile,
            budget,
            cancelled,
        )
        .map_err(|e| e.to_string())
}

impl RunOutput {
    pub fn keeps_snapshot(&self, snapshot: &BlockClipboard) -> Result<bool, String> {
        let original = mcedit2_world::editing::snapshot::Content::capture(snapshot)
            .map_err(|error| error.to_string())?;
        Ok(self.blocks.as_ref() == Some(&snapshot.blocks)
            && self
                .content
                .as_ref()
                .is_none_or(|content| *content == original))
    }
    pub fn validate_with_budget(&self, budget: EditWorkBudget) -> Result<(), String> {
        self.validate()?;
        work::check_output(self, budget)
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(bindings) = &self.object_bindings {
            bindings.validate(self.content.as_ref().ok_or("工具引用令牌缺少输出内容")?)?;
        }
        if let Some(scene) = &self.tool_scene {
            scene.validate()?;
        }
        if let Some(pick) = &self.tool_pick {
            pick.validate()?;
            if let tool_pick::Pick::Plugin { command } = pick {
                if !self.commands.iter().any(|entry| &entry.id == command) {
                    return Err("切换目标不在实际插件命令目录中".into());
                }
            }
        }
        if self.protocol_version != PROTOCOL_VERSION {
            return Err("插件返回了无效协议版本".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for command in &self.commands {
            if let Some(icon) = &command.icon {
                tool_pick::validate_icon(icon)?;
            }
            if let Some(action) = &command.action {
                action.validate()?;
            }
            for spec in &command.options {
                if let Some(state) = spec.get("action") {
                    let state: tool_pick::ActionState = serde_json::from_value(state.clone())
                        .map_err(|error| format!("表单动作状态无效：{error}"))?;
                    state.validate()?;
                }
            }
            if command.id.is_empty()
                || command.name.is_empty()
                || (command.preview_on_change && !command.id.starts_with("generator:"))
                || ((command.gesture_input || command.persistent_tool)
                    && !command.id.starts_with("tool:"))
                || (!command.inspector_ids.is_empty() && !command.id.starts_with("inspector:"))
                || command.inspector_ids.iter().any(|id| id.is_empty())
                || !ids.insert(&command.id)
            {
                return Err("插件命令标识、名称或选项无效".into());
            }
        }
        if self
            .inspector_operation
            .as_ref()
            .is_some_and(|patch| patch.operations().is_empty())
        {
            return Err("Inspector 操作批次不能为空".into());
        }
        if self
            .messages
            .iter()
            .any(|m| !matches!(m.level.as_str(), "info" | "warning" | "error" | "debug"))
        {
            return Err("插件日志级别无效".into());
        }
        if let Some(content) = &self.content {
            for object in content.block_entities.iter().chain(&content.entities) {
                if !matches!(object.nbt, mcedit2_world::nbt_document::Value::Compound(_)) {
                    return Err("插件对象根必须是 Compound".into());
                }
                object.nbt.payload().map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}
