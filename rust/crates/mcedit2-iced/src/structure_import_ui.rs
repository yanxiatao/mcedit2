//! 独立结构的导入、克隆与移动：冻结原稿、完整候选、参数历史和一次正式发布。
use super::{Entry, Legacy, Page, Work};
use crate::{App, Message};
use iced::{
    widget::{button, checkbox, column, pick_list, row, text, text_input},
    Element, Task,
};
use mcedit2_world::{
    clipboard::BlockClipboard,
    editing::{Bounds, ClonePattern, SelectionShape, TransferKind},
    structure_document::{
        Document, ImportOptions, PasteOptions, PreparedImport, PreparedTransfer, TransferSource,
    },
    structure_exchange::Format,
    structure_transform::Transform,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
#[path = "structure_import_history.rs"]
mod completed;
pub(crate) use completed::maintain as maintain_history;
pub(super) use completed::{finished, intent, jump, jumped, Intent, Jump};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Parameters {
    position: String,
    position_mode: crate::position_input::Mode,
    rotation: String,
    scale: String,
    rotate_states: bool,
    entities: bool,
    biomes: bool,
    skip_air: bool,
    remove_be: bool,
    copies: String,
    rotate_repeats: bool,
    rotate_offset: bool,
}
impl Default for Parameters {
    fn default() -> Self {
        Self {
            position: "0,0,0".into(),
            position_mode: Default::default(),
            rotation: "0,0,0".into(),
            scale: "1,1,1".into(),
            rotate_states: true,
            entities: true,
            biomes: false,
            skip_air: false,
            remove_be: false,
            copies: "1".into(),
            rotate_repeats: false,
            rotate_offset: false,
        }
    }
}
impl Parameters {
    fn options(&self, origin: Option<[i32; 3]>) -> Result<ImportOptions, String> {
        let point = self.position_mode.resolve(&self.position, origin)?;
        let transform = Transform {
            rotation: super::vector(&self.rotation)?,
            scale: super::vector(&self.scale)?,
            rotate_states: self.rotate_states,
        };
        transform.validate().map_err(|error| error.to_string())?;
        Ok(ImportOptions {
            anchor: point,
            transform,
            entities: self.entities,
            paste: PasteOptions {
                skip_air: self.skip_air,
                remove_block_entities: self.remove_be,
                include_biomes: self.biomes,
            },
        })
    }
}
struct Ready {
    proof: Arc<Proof>,
    params: Parameters,
    selection: super::selection::ScriptTicket,
}
pub(crate) struct State {
    format: Format,
    path: String,
    legacy: Option<Legacy>,
    dimension: String,
    params: Parameters,
    source: Option<Arc<BlockClipboard>>,
    transfer: Option<(TransferKind, Arc<TransferSource>)>,
    source_label: String,
    target_minimum: Option<[i32; 3]>,
    ready: Option<Ready>,
    epoch: u64,
    cancel: Arc<AtomicBool>,
    running: bool,
    pub(super) committing: bool,
    pub(super) active: bool,
    history: Arc<Vec<Parameters>>,
    recapture: bool,
    completed: completed::Completed,
    cursor: usize,
    editing: Option<Field>,
    pointer: Arc<Mutex<crate::selection_handle::Pointer>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            format: Format::Clipboard,
            path: String::new(),
            legacy: None,
            dimension: String::new(),
            params: Default::default(),
            source: None,
            transfer: None,
            source_label: String::new(),
            target_minimum: None,
            ready: None,
            epoch: 0,
            cancel: Default::default(),
            running: false,
            committing: false,
            active: false,
            history: Arc::new(Vec::new()),
            recapture: false,
            completed: Default::default(),
            cursor: 0,
            editing: None,
            pointer: Default::default(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    Position,
    Rotation,
    Scale,
    Copies,
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Flag {
    RotateStates,
    Entities,
    Biomes,
    SkipAir,
    RemoveBe,
    RotateRepeats,
    RotateOffset,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Coordinates(crate::position_input::Request),
    Show,
    Selection(TransferKind),
    Captured(
        u64,
        TransferKind,
        (String, SelectionShape),
        Result<Arc<TransferSource>, String>,
    ),
    Format(Format),
    Path(String),
    Legacy(Legacy),
    Choose,
    Chosen(u64, Format, Result<Option<PathBuf>, String>),
    Read,
    ReadReady(u64, Result<Arc<BlockClipboard>, String>),
    Text(Field, String),
    Toggle(Flag, bool),
    Dimension(String),
    BindDimension,
    Preview,
    Prepared(
        u64,
        Parameters,
        Arc<BlockClipboard>,
        super::selection::ScriptTicket,
        Result<(Arc<Proof>, Option<(TransferKind, Arc<TransferSource>)>), String>,
    ),
    Confirm,
    Cancel,
    History(bool),
    Translate(usize, i32),
    RotateY,
    Adjust(crate::transform_controls::Adjustment),
    Dragged(crate::selection_handle::Finished),
    Clipboard(Arc<BlockClipboard>),
    File(PathBuf, Format, [i32; 3]),
    Restore(completed::Restore),
}
/// 外部导入和同一文件选区共用原生候选、放置手势及参数历史。
#[derive(Debug, Clone)]
pub(crate) enum Proof {
    Import(PreparedImport),
    Transfer(PreparedTransfer),
}
impl Proof {
    pub(super) fn matches(&self, document: &Document) -> bool {
        match self {
            Self::Import(proof) => proof.matches(document),
            Self::Transfer(proof) => proof.matches(document),
        }
    }
    fn candidate(&self) -> &Document {
        match self {
            Self::Import(proof) => proof.candidate(),
            Self::Transfer(proof) => proof.candidate(),
        }
    }
    pub(super) fn bounds(&self) -> Bounds {
        match self {
            Self::Import(proof) => proof.bounds,
            Self::Transfer(proof) => proof.bounds,
        }
    }
    fn summary(&self) -> String {
        match self {
            Self::Import(proof) => proof.summary(),
            Self::Transfer(proof) => proof.summary(),
        }
    }
    pub(super) fn commit(
        &self,
        document: &mut Document,
        cancelled: impl FnMut() -> bool,
    ) -> Result<bool, String> {
        match self {
            Self::Import(proof) => document.commit_import_cancellable(proof, cancelled),
            Self::Transfer(proof) => document.commit_transfer_cancellable(proof, cancelled),
        }
        .map_err(|error| error.to_string())
    }
    pub(super) fn completed(&self) -> String {
        match self {
            Self::Import(proof) if !proof.changed => {
                "导入内容未变，仅记录完成放置的界面历史。".into()
            }
            Self::Import(_) => "结构导入已应用；一次文件历史，尚未保存。".into(),
            Self::Transfer(proof) if proof.changed => format!(
                "{}已应用；一次文件历史，尚未保存。",
                if proof.kind == TransferKind::Move {
                    "选区移动"
                } else {
                    "重复克隆"
                }
            ),
            Self::Transfer(_) => "内容未变，仅记录完成放置的界面历史。".into(),
        }
    }
}
impl State {
    fn position_origin(&self) -> Option<[i32; 3]> {
        if let Some((_, source)) = &self.transfer {
            Some(source.region().bounds.minimum)
        } else {
            self.source.as_ref().map(|source| source.source_min)
        }
    }
    pub(super) fn engaged(&self) -> bool {
        self.active || self.running || self.ready.is_some() || self.committing
    }
    pub(super) fn cancel_gesture(&self) {
        self.pointer.lock().expect("独立导入手势锁").cancel();
    }
    pub(super) fn candidate(&self) -> Option<&Document> {
        self.ready.as_ref().map(|ready| ready.proof.candidate())
    }
    pub(super) fn bounds(&self) -> Option<Bounds> {
        self.ready.as_ref().map(|ready| ready.proof.bounds())
    }
    pub(super) fn invalidate(&mut self, clear_source: bool) {
        self.pointer.lock().expect("独立导入手势锁").cancel();
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.running = false;
        self.ready = None;
        self.committing = false;
        if clear_source {
            self.source = None;
            self.recapture = false;
            self.transfer = None;
            self.source_label.clear();
            self.target_minimum = None;
            self.active = false;
            Arc::make_mut(&mut self.history).clear();
            self.cursor = 0;
            self.editing = None;
        }
    }
    fn begin(&mut self) -> (u64, Arc<AtomicBool>) {
        self.pointer.lock().expect("独立导入手势锁").cancel();
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.running = true;
        self.active = true;
        self.editing = None;
        (self.epoch, Arc::clone(&self.cancel))
    }
    fn record(&mut self, before: Parameters, field: Option<Field>) {
        if before == self.params || !self.active {
            return;
        }
        if self.history.is_empty() {
            Arc::make_mut(&mut self.history).push(before);
            self.cursor = 0;
        }
        Arc::make_mut(&mut self.history).truncate(self.cursor + 1);
        if field.is_some() && self.editing == field && self.cursor > 0 {
            Arc::make_mut(&mut self.history)[self.cursor] = self.params.clone();
        } else {
            Arc::make_mut(&mut self.history).push(self.params.clone());
            self.cursor += 1;
        }
        self.editing = field;
    }
    fn parameters_changed(&mut self, before: Parameters, field: Option<Field>) {
        self.pointer.lock().expect("独立导入手势锁").cancel();
        self.cancel.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.running = false;
        self.record(before, field);
    }
    fn can_confirm(&self, document: &Document) -> bool {
        !self.running
            && !self.committing
            && self
                .ready
                .as_ref()
                .is_some_and(|ready| ready.params == self.params && ready.proof.matches(document))
    }
    pub(super) fn can_history(&self, redo: bool) -> bool {
        self.active
            && !self.committing
            && if redo {
                self.cursor + 1 < self.history.len()
            } else {
                self.cursor > 0
            }
    }
}
pub(crate) fn owns_history(app: &App, id: u64) -> bool {
    app.structures
        .files
        .get(&id)
        .is_some_and(|entry| entry.import.active)
}
pub(super) fn completed_history(entry: &Entry, redo: bool) -> bool {
    completed::available(entry, redo)
}
pub(crate) fn transfer_kind(app: &App, id: u64) -> Option<TransferKind> {
    let entry = app.structures.files.get(&id)?;
    entry.import.active.then_some(())?;
    entry.import.transfer.as_ref().map(|(kind, _)| *kind)
}
pub(crate) fn layer(app: &App, id: u64) -> Option<crate::selection_handle::Layer> {
    if super::active(app) != Some(id)
        || app.shortcuts.open
        || app.block_picker.open()
        || app.workspace.closing.is_some()
        || crate::has_flight_input(&app.spectator_keys)
    {
        return None;
    }
    let entry = app.structures.files.get(&id)?;
    let document = entry.document.as_ref()?;
    if entry.busy
        || entry.readonly
        || entry.recovery.is_some()
        || !entry.import.can_confirm(document)
    {
        return None;
    }
    let ready = entry.import.ready.as_ref()?;
    Some(crate::selection_handle::Layer::placement(
        crate::keyboard_bindings::Owner::Structure(id),
        app.input_epoch,
        entry.import.epoch,
        Some(crate::selection_handle::SelectionBox::from_bounds(
            ready.proof.bounds(),
        )),
        Arc::clone(&entry.import.pointer),
        Arc::clone(&app.selection_edit.contour),
    ))
}
fn send(id: u64, action: Action) -> Message {
    Message::StructureWorkspace(id, super::Action::Import(action))
}
fn check_editable(entry: &Entry) -> Result<(), String> {
    if entry.readonly
        || entry.busy
        || entry.plugin.engaged()
        || entry.search.engaged()
        || entry.brush.engaged()
        || entry.recovery.is_some()
        || entry.document.is_none()
    {
        return Err("当前结构文档不可放置或正在执行其他任务".into());
    }
    Ok(())
}
fn prepare(app: &mut App, id: u64) -> Result<Task<Message>, String> {
    let budget = app.block_edit.budget.parse()?;
    let entry = app.structures.files.get_mut(&id).ok_or("结构文档已关闭")?;
    check_editable(entry)?;
    let source = entry
        .import
        .source
        .clone()
        .ok_or("请先读取结构原稿或选择待克隆 / 移动的选区")?;
    let params = entry.import.params.clone();
    let options = params.options(entry.import.position_origin())?;
    let transfer = entry.import.transfer.clone();
    let pattern = if let Some((kind, frozen)) = &transfer {
        if !entry.import.recapture
            && !frozen.matches(entry.document.as_ref().expect("正式文档存在"))
        {
            return Err("克隆 / 移动原稿已过期，请重新选择选区".into());
        }
        Some(ClonePattern {
            transform: options.transform,
            copies: if *kind == TransferKind::Clone {
                params
                    .copies
                    .trim()
                    .parse()
                    .map_err(|error| format!("重复份数无效：{error}"))?
            } else {
                1
            },
            rotate_repeats: *kind == TransferKind::Clone && params.rotate_repeats,
            rotate_offset: *kind == TransferKind::Clone && params.rotate_offset,
            copy_air: !params.skip_air,
        })
    } else {
        None
    };
    let document = entry.document.as_ref().expect("已核实正式文档").clone();
    if !entry.import.active {
        entry.import.history = Arc::new(vec![params.clone()]);
        entry.import.cursor = 0;
    }
    let selection = super::selection::script_ticket(entry);
    let (epoch, token) = entry.import.begin();
    entry.page = Page::Import;
    entry.report = "正在准备完整结构放置候选…".into();
    let source_worker = Arc::clone(&source);
    let recapture = entry.import.recapture;
    Ok(Task::perform(
        crate::nbt_background(move || {
            let mut refreshed = None;
            let proof = if let Some((kind, frozen)) = transfer {
                let frozen = if recapture {
                    Arc::new(
                        document
                            .capture_transfer_cancellable(frozen.region(), budget, || {
                                token.load(Ordering::Acquire)
                            })
                            .map_err(|error| error.to_string())?,
                    )
                } else {
                    frozen
                };
                if recapture {
                    refreshed = Some((kind, Arc::clone(&frozen)));
                }
                document
                    .prepare_transfer_cancellable(
                        frozen,
                        kind,
                        options.anchor,
                        pattern.expect("已解析克隆 / 移动参数"),
                        options.entities,
                        options.paste.include_biomes,
                        options.paste.remove_block_entities,
                        budget,
                        || token.load(Ordering::Acquire),
                    )
                    .map(Proof::Transfer)
            } else {
                document
                    .prepare_import_cancellable(source_worker, options, budget, || {
                        token.load(Ordering::Acquire)
                    })
                    .map(Proof::Import)
            };
            proof
                .map(|proof| (Arc::new(proof), refreshed))
                .map_err(|error| error.to_string())
        }),
        move |result| {
            send(
                id,
                Action::Prepared(
                    epoch,
                    params.clone(),
                    Arc::clone(&source),
                    selection.clone(),
                    result,
                ),
            )
        },
    ))
}
fn handle(app: &mut App, id: u64, action: Action) -> Result<Task<Message>, String> {
    if !app.structures.files.contains_key(&id) {
        return Ok(Task::none());
    }
    match action {
        Action::Show => {
            return Ok(super::update(app, id, super::Action::Page(Page::Import)));
        }
        Action::Selection(kind) => {
            if !matches!(kind, TransferKind::Clone | TransferKind::Move) {
                return Err("请选择克隆或移动选区".into());
            }
            crate::keyboard_bindings::interrupt(app);
            let budget = app.block_edit.budget.parse()?;
            let defaults = crate::transfer_edit::placement_parameters(app);
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            check_editable(entry)?;
            if entry.import.engaged() {
                return Err("请先确认或取消当前放置候选".into());
            }
            let region = entry.analysis_region()?;
            let document = entry.document.clone().expect("已核对文档");
            let selection = (entry.bounds.clone(), entry.shape);
            let [x, y, z] = region.bounds.minimum;
            entry.import.invalidate(true);
            entry.import.params = Parameters {
                position: format!("{x},{y},{z}"),
                position_mode: Default::default(),
                rotation: defaults.rotation,
                scale: defaults.scale,
                copies: defaults.copies,
                rotate_states: defaults.rotate_states,
                rotate_repeats: defaults.rotate_repeats,
                rotate_offset: defaults.rotate_offset,
                entities: defaults.entities,
                biomes: defaults.biomes,
                skip_air: !defaults.copy_air,
                remove_be: entry.remove_be,
            };
            let (epoch, token) = entry.import.begin();
            entry.page = Page::Import;
            entry.report = "正在冻结当前文件的完整选区原稿…".into();
            return Ok(Task::perform(
                crate::nbt_background(move || {
                    document
                        .capture_transfer_cancellable(region, budget, || {
                            token.load(Ordering::Acquire)
                        })
                        .map(Arc::new)
                        .map_err(|error| error.to_string())
                }),
                move |result| send(id, Action::Captured(epoch, kind, selection.clone(), result)),
            ));
        }
        Action::Captured(epoch, kind, selection, result) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if epoch != entry.import.epoch || !entry.import.running {
                return Ok(Task::none());
            }
            entry.import.running = false;
            let frozen = result?;
            if selection != (entry.bounds.clone(), entry.shape)
                || entry
                    .document
                    .as_ref()
                    .is_none_or(|document| !frozen.matches(document))
            {
                return Err("冻结期间选区、文档或定义已变化，请取消后重新选择".into());
            }
            entry.import.source = Some(frozen.clipboard());
            entry.import.source_label = format!(
                "当前文件选区 {:?} 至 {:?} · {}",
                frozen.region().bounds.minimum,
                frozen.region().bounds.maximum,
                if kind == TransferKind::Move {
                    "移动，保留实体身份"
                } else {
                    "克隆，新实体身份"
                }
            );
            entry.import.transfer = Some((kind, frozen));
            entry.import.history = Arc::new(vec![entry.import.params.clone()]);
            entry.import.cursor = 0;
            return prepare(app, id);
        }
        Action::Clipboard(source) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            check_editable(entry)?;
            if entry.import.engaged() {
                return Err("请先确认或取消当前放置候选".into());
            }
            let minimum = entry.import_minimum()?;
            let anchor = std::array::from_fn::<_, 3, _>(|axis| {
                minimum[axis].checked_sub(source.offset[axis])
            });
            let [Some(x), Some(y), Some(z)] = anchor else {
                return Err("剪贴板放置锚点溢出".into());
            };
            entry.import.invalidate(true);
            entry.import.params = Parameters {
                position: format!("{x},{y},{z}"),
                skip_air: entry.skip_air,
                remove_be: entry.remove_be,
                biomes: entry.paste_biomes,
                ..Default::default()
            };
            entry.import.source = Some(source);
            entry.import.source_label = "内部剪贴板".into();
            entry.import.format = Format::Clipboard;
            entry.import.path.clear();
            entry.import.legacy = None;
            entry.import.dimension.clear();
            entry.page = Page::Import;
            completed::begin(entry)?;
            let active = super::activate(app, id);
            let preview = prepare(app, id)?;
            return Ok(Task::batch([active, preview]));
        }
        Action::File(path, format, minimum) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            check_editable(entry)?;
            if entry.import.engaged() {
                return Err("请先确认或取消当前放置候选".into());
            }
            entry.import.invalidate(true);
            entry.import.path = path.to_string_lossy().into_owned();
            entry.import.format = format;
            entry.import.legacy = None;
            entry.import.params = Parameters {
                skip_air: entry.skip_air,
                remove_be: entry.remove_be,
                biomes: entry.paste_biomes,
                ..Default::default()
            };
            entry.import.target_minimum = Some(minimum);
            entry.page = Page::Import;
            let activation = super::activate(app, id);
            if format == Format::Legacy {
                let entry = app.structures.files.get_mut(&id).expect("文档存在");
                entry.import.active = true;
                entry.report =
                    "资料库结构未记录版本，请选择输入文件的真实版本或明确保留无版本后读取。".into();
                return Ok(activation);
            }
            return Ok(Task::batch([activation, handle(app, id, Action::Read)?]));
        }
        Action::Choose => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            check_editable(entry)?;
            let format = entry.import.format;
            let (epoch, _) = entry.import.begin();
            let draft = PathBuf::from(entry.import.path.trim());
            let start = crate::dialog_paths::start(
                app,
                crate::dialog_paths::Kind::StructureImport,
                Some(&draft),
                false,
            );
            return Ok(Task::perform(
                async move {
                    let (send, receive) = iced::futures::channel::oneshot::channel();
                    std::thread::spawn(move || {
                        let path = start
                            .configure(
                                rfd::FileDialog::new()
                                    .add_filter(format.label(), format.extensions()),
                            )
                            .pick_file();
                        if send.send(path).is_err() {
                            log::debug!("[structure-import] 文件选择接收端已关闭");
                        }
                    });
                    receive.await.map_err(|error| error.to_string())
                },
                move |result| {
                    let path = match &result {
                        Ok(path) => path.as_deref(),
                        Err(_) => None,
                    };
                    let choice = crate::dialog_paths::Choice::file(
                        crate::dialog_paths::Kind::StructureImport,
                        path,
                    );
                    choice.then(send(id, Action::Chosen(epoch, format, result)))
                },
            ));
        }
        Action::Chosen(epoch, format, result) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if epoch != entry.import.epoch || format != entry.import.format {
                return Ok(Task::none());
            }
            entry.import.running = false;
            if let Some(path) = result? {
                let displayed = entry.import.ready.is_some();
                entry.import.invalidate(true);
                entry.import.path = path.to_string_lossy().into_owned();
                let refresh = if displayed {
                    super::refresh_display(app, id)
                } else {
                    Task::none()
                };
                return Ok(Task::batch([refresh, Task::done(send(id, Action::Read))]));
            }
            if entry.import.source.is_none() {
                entry.import.active = entry.import.target_minimum.is_some();
            }
        }
        Action::Read => {
            let budget = app.block_edit.budget.parse()?;
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            check_editable(entry)?;
            let format = entry.import.format;
            let path = PathBuf::from(entry.import.path.trim());
            let legacy = if format == Format::Legacy {
                Some(
                    entry
                        .import
                        .legacy
                        .ok_or("旧 schematic 未记录版本，请明确选择输入文件的真实版本或保留无版本")?
                        .source(),
                )
            } else {
                None
            };
            let displayed = entry.import.ready.is_some();
            let target_minimum = entry.import.target_minimum;
            entry.import.invalidate(true);
            entry.import.target_minimum = target_minimum;
            let (epoch, token) = entry.import.begin();
            entry.page = Page::Import;
            entry.report = "正在读取外部结构原稿…".into();
            let refresh = if displayed {
                super::refresh_display(app, id)
            } else {
                Task::none()
            };
            return Ok(Task::batch([
                refresh,
                Task::perform(
                    crate::nbt_background(move || {
                        if token.load(Ordering::Acquire) {
                            return Err("读取结构已取消".into());
                        }
                        let document = Document::open_read_only(&path, format, legacy, budget)
                            .map_err(|error| error.to_string())?;
                        if token.load(Ordering::Acquire) {
                            return Err("读取结构已取消".into());
                        }
                        Ok(document.content())
                    }),
                    move |result| send(id, Action::ReadReady(epoch, result)),
                ),
            ]));
        }
        Action::ReadReady(epoch, result) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if epoch != entry.import.epoch {
                return Ok(Task::none());
            }
            entry.import.running = false;
            let source = result?;
            if let Some(minimum) = entry.import.target_minimum.take() {
                let anchor = std::array::from_fn::<_, 3, _>(|axis| {
                    minimum[axis].checked_sub(source.offset[axis])
                });
                let [Some(x), Some(y), Some(z)] = anchor else {
                    return Err("资料库结构放置锚点溢出".into());
                };
                entry.import.params.position = entry.import.params.position_mode.encode([x, y, z]);
            }
            entry.import.source = Some(source);
            entry.import.source_label = entry.import.path.clone();
            entry.import.history = Arc::new(vec![entry.import.params.clone()]);
            entry.import.cursor = 0;
            completed::begin(entry)?;
            return prepare(app, id);
        }
        Action::Preview => return prepare(app, id),
        Action::Dragged(finished) => {
            if layer(app, id).is_none_or(|layer| layer.config != finished.config) {
                return Ok(Task::none());
            }
            let original = finished.config.selection.expect("放置拖动具有候选范围");
            let next = finished.selection.validate_for(finished.config.owner)?;
            if (0..3).any(|axis| {
                next.maximum[axis] - next.minimum[axis]
                    != original.maximum[axis] - original.minimum[axis]
            }) {
                return Err("导入拖动只能平移，缩放请使用结构参数".into());
            }
            let delta =
                std::array::from_fn::<_, 3, _>(|axis| next.minimum[axis] - original.minimum[axis]);
            if delta == [0; 3] {
                return Ok(Task::none());
            }
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            let before = entry.import.params.clone();
            let point = before
                .position_mode
                .resolve(&before.position, entry.import.position_origin())?;
            let moved = std::array::from_fn::<_, 3, _>(|axis| point[axis].checked_add(delta[axis]));
            let [Some(x), Some(y), Some(z)] = moved else {
                return Err("导入拖动位置溢出".into());
            };
            entry.import.params.position = before.position_mode.encode([x, y, z]);
            entry.import.parameters_changed(before, None);
            return prepare(app, id);
        }
        Action::Prepared(epoch, params, source, selection, result) => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if epoch != entry.import.epoch || !entry.import.running {
                return Ok(Task::none());
            }
            entry.import.running = false;
            if entry.import.params != params
                || entry
                    .import
                    .source
                    .as_ref()
                    .is_none_or(|current| !Arc::ptr_eq(current, &source))
            {
                return Ok(Task::none());
            }
            let (proof, refreshed) = result?;
            if entry
                .document
                .as_ref()
                .is_none_or(|document| !proof.matches(document))
            {
                return Err("目标文档或定义在预览期间已经改变".into());
            }
            if let Some((kind, frozen)) = refreshed {
                entry.import.source = Some(frozen.clipboard());
                entry.import.transfer = Some((kind, frozen));
                entry.import.recapture = false;
            }
            entry.report = proof.summary();
            entry.import.ready = Some(Ready {
                proof,
                params,
                selection,
            });
            return Ok(super::refresh_display(app, id));
        }
        Action::Confirm => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            check_editable(entry)?;
            if !entry
                .import
                .can_confirm(entry.document.as_ref().expect("文档存在"))
            {
                return Err("放置参数或来源已改变，请更新预览后确认".into());
            }
            let proof = Arc::clone(&entry.import.ready.as_ref().expect("已核实候选").proof);
            entry.import.committing = true;
            return match super::begin(app, id, Work::Import(proof)) {
                Ok(task) => Ok(task),
                Err(error) => {
                    app.structures
                        .files
                        .get_mut(&id)
                        .expect("文档存在")
                        .import
                        .committing = false;
                    Err(error)
                }
            };
        }
        Action::Cancel => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.import.committing {
                entry.cancel.store(true, Ordering::Release);
            }
            entry.import.invalidate(entry.import.transfer.is_some());
            entry.import.target_minimum = None;
            entry.import.active = false;
            Arc::make_mut(&mut entry.import.history).clear();
            entry.import.cursor = 0;
            entry.import.editing = None;
            entry.report = "已取消结构放置，正式文档未改变".into();
            return Ok(super::refresh_display(app, id));
        }
        Action::History(redo) => {
            if app.structures.files[&id].timeline_editable()
                && completed::available(&app.structures.files[&id], redo)
            {
                return super::begin(app, id, if redo { Work::Redo } else { Work::Undo });
            }
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if !entry.import.active || entry.busy || entry.import.committing {
                return Ok(Task::none());
            }
            let state = &mut entry.import;
            if redo {
                if state.cursor + 1 >= state.history.len() {
                    return Ok(Task::none());
                }
                state.cursor += 1;
            } else {
                if state.cursor == 0 {
                    return Ok(Task::none());
                }
                state.cursor -= 1;
            }
            state.params = state.history[state.cursor].clone();
            state.editing = None;
            return prepare(app, id);
        }
        Action::Restore(request) => return completed::restore(app, id, request),
        other => {
            let entry = app.structures.files.get_mut(&id).expect("文档存在");
            if entry.busy {
                return Err("请等待当前结构任务完成".into());
            }
            let before = entry.import.params.clone();
            let displayed = entry.import.ready.is_some();
            let mut field = None;
            let mut preview = false;
            let mut refresh = false;
            match other {
                Action::Coordinates(request) => {
                    let typing = request.typing();
                    let (mode, value) = request.apply(
                        entry.import.params.position_mode,
                        &entry.import.params.position,
                        entry.import.position_origin(),
                    )?;
                    entry.import.target_minimum = None;
                    entry.import.params.position_mode = mode;
                    entry.import.params.position = value;
                    field = typing.then_some(Field::Position);
                }
                Action::Format(format) => {
                    entry.import.invalidate(true);
                    entry.import.format = format;
                    entry.import.legacy = None;
                    refresh = displayed;
                }
                Action::Path(path) => {
                    entry.import.invalidate(true);
                    entry.import.path = path;
                    refresh = displayed;
                }
                Action::Legacy(legacy) => {
                    let target_minimum = entry.import.target_minimum;
                    entry.import.invalidate(true);
                    entry.import.target_minimum = target_minimum;
                    entry.import.active = target_minimum.is_some();
                    entry.import.legacy = Some(legacy);
                    refresh = displayed;
                }
                Action::Dimension(dimension) => {
                    entry.import.dimension = dimension;
                    return Ok(Task::none());
                }
                Action::BindDimension => {
                    if entry.import.transfer.is_some() {
                        return Err("同一文件的选区操作不更换来源维度".into());
                    }
                    let previous = entry.import.source.clone().ok_or("请先读取原稿")?;
                    let mut source = (*previous).clone();
                    source
                        .bind_entity_dimension(entry.import.dimension.trim())
                        .map_err(|error| error.to_string())?;
                    entry.import.invalidate(false);
                    entry.import.source = Some(Arc::new(source));
                    completed::rebind(entry, &previous);
                    preview = true;
                }
                Action::Text(input, value) => {
                    entry.import.target_minimum = None;
                    field = Some(input);
                    match input {
                        Field::Position => entry.import.params.position = value,
                        Field::Rotation => entry.import.params.rotation = value,
                        Field::Scale => entry.import.params.scale = value,
                        Field::Copies => entry.import.params.copies = value,
                    }
                }
                Action::Toggle(flag, value) => match flag {
                    Flag::RotateStates => entry.import.params.rotate_states = value,
                    Flag::Entities => entry.import.params.entities = value,
                    Flag::Biomes => entry.import.params.biomes = value,
                    Flag::SkipAir => entry.import.params.skip_air = value,
                    Flag::RemoveBe => entry.import.params.remove_be = value,
                    Flag::RotateRepeats => entry.import.params.rotate_repeats = value,
                    Flag::RotateOffset => entry.import.params.rotate_offset = value,
                },
                Action::Translate(axis, delta) => {
                    let mut point = entry.import.params.position_mode.resolve(
                        &entry.import.params.position,
                        entry.import.position_origin(),
                    )?;
                    point[axis] = point[axis].checked_add(delta).ok_or("导入位置溢出")?;
                    entry.import.params.position = entry.import.params.position_mode.encode(point);
                    preview = true;
                }
                Action::Adjust(adjustment) => {
                    let value = adjustment
                        .apply(&entry.import.params.rotation, &entry.import.params.scale)?;
                    if adjustment.rotation() {
                        entry.import.params.rotation = value;
                    } else {
                        entry.import.params.scale = value;
                    }
                    preview = entry.import.source.is_some();
                }
                Action::RotateY => {
                    let mut rotation = super::vector(&entry.import.params.rotation)?;
                    rotation[1] = (rotation[1].rem_euclid(360.) + 90.).rem_euclid(360.);
                    entry.import.params.rotation =
                        format!("{},{},{}", rotation[0], rotation[1], rotation[2]);
                    preview = true;
                }
                _ => unreachable!("异步和控制消息已处理"),
            }
            entry.import.parameters_changed(before, field);
            if entry.import.ready.is_some() {
                entry.report = "参数已修改，画面保留上次完成的候选；更新预览后才能确认".into();
            }
            if preview {
                return prepare(app, id);
            }
            if refresh {
                return Ok(super::refresh_display(app, id));
            }
        }
    }
    Ok(Task::none())
}
pub(crate) fn update(app: &mut App, id: u64, action: Action) -> Task<Message> {
    if super::active(app) == Some(id) {
        let target = match &action {
            Action::Selection(TransferKind::Clone) => {
                Some(mcedit2_plugins::tool_pick::Builtin::Clone)
            }
            Action::Selection(TransferKind::Move) => {
                Some(mcedit2_plugins::tool_pick::Builtin::Move)
            }
            _ => None,
        };
        if let Some(target) = target {
            if let Some(task) = crate::plugin_ui::handoff_builtin(app, target) {
                return task;
            }
        }
    }
    match handle(app, id, action) {
        Ok(task) => task,
        Err(error) => {
            if let Some(entry) = app.structures.files.get_mut(&id) {
                super::report(entry, error);
            }
            Task::none()
        }
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    for (&id, entry) in &app.structures.files {
        if layer(app, id).is_none() {
            entry
                .import
                .pointer
                .lock()
                .expect("独立导入手势锁")
                .cancel();
        }
    }
    let stale = app
        .structures
        .files
        .iter()
        .filter_map(|(&id, entry)| {
            let stale = entry.import.ready.as_ref().is_some_and(|ready| {
                entry
                    .document
                    .as_ref()
                    .is_none_or(|document| !ready.proof.matches(document))
            }) || entry.import.transfer.as_ref().is_some_and(|(_, source)| {
                !entry.import.recapture
                    && entry
                        .document
                        .as_ref()
                        .is_none_or(|document| !source.matches(document))
            });
            stale.then_some(id)
        })
        .collect::<Vec<_>>();
    let mut tasks = Vec::new();
    for id in stale {
        let entry = app.structures.files.get_mut(&id).expect("已枚举文档");
        if entry.import.committing {
            entry.cancel.store(true, Ordering::Release);
        }
        entry.import.invalidate(entry.import.transfer.is_some());
        entry.import.active = false;
        super::report(
            entry,
            "文档或定义已改变，放置候选已撤回；选区操作需重新捕获原稿。",
        );
        tasks.push(super::refresh_display(app, id));
    }
    Task::batch(tasks)
}
pub(crate) fn view(app: &App, id: u64) -> Element<'_, Message> {
    let entry = &app.structures.files[&id];
    let state = &entry.import;
    let params = &state.params;
    let editable =
        !entry.readonly && !entry.busy && entry.recovery.is_none() && entry.document.is_some();
    let action = move |action| send(id, action);
    let kind = state.transfer.as_ref().map(|(kind, _)| *kind);
    let mut body = column![
        text(match kind {
            Some(TransferKind::Clone) => "克隆当前文件选区",
            Some(TransferKind::Move) => "移动当前文件选区",
            _ => "导入结构到当前文档",
        })
        .size(18),
        row![
            button("克隆当前选区").on_press_maybe(
                (editable && !entry.engaged())
                    .then_some(action(Action::Selection(TransferKind::Clone)))
            ),
            button("移动当前选区").on_press_maybe(
                (editable && !entry.engaged())
                    .then_some(action(Action::Selection(TransferKind::Move)))
            ),
        ]
        .spacing(5)
        .wrap(),
        text("外部文件来源；修改来源会结束当前放置候选。"),
        row![
            pick_list(Format::ALL, Some(state.format), move |format| action(
                Action::Format(format)
            )),
            text_input("输入结构文件", &state.path)
                .on_input(move |value| action(Action::Path(value))),
            button("浏览…").on_press_maybe(editable.then_some(action(Action::Choose)))
        ]
        .spacing(5),
    ]
    .spacing(7);
    if state.format == Format::Legacy {
        body = body.push(
            pick_list(
                [
                    Legacy::Unversioned,
                    Legacy::V1122,
                    Legacy::V1201,
                    Legacy::V1211,
                ],
                state.legacy,
                move |legacy| action(Action::Legacy(legacy)),
            )
            .placeholder("输入文件的真实版本"),
        );
    }
    if !state.source_label.is_empty() {
        body = body.push(text(format!("当前放置原稿：{}", state.source_label)).size(12));
    }
    if let Some(minimum) = state.target_minimum {
        body =
            body.push(text(format!("放置最小格：{minimum:?}；读取实际原稿后计算锚点。")).size(12));
    }
    body = body
        .push(button("读取外部文件并预览").on_press_maybe(editable.then_some(action(Action::Read))))
        .push(
            crate::position_input::view(
                &params.position,
                params.position_mode,
                state.position_origin(),
                editable,
            )
            .map(move |request| action(Action::Coordinates(request))),
        )
        .push(
            text_input("XYZ 旋转角度", &params.rotation)
                .on_input(move |value| action(Action::Text(Field::Rotation, value))),
        )
        .push(
            text_input("XYZ 缩放，负值镜像", &params.scale)
                .on_input(move |value| action(Action::Text(Field::Scale, value))),
        )
        .push(
            crate::transform_controls::view(editable)
                .map(move |adjustment| action(Action::Adjust(adjustment))),
        )
        .push(
            row![
                checkbox(params.rotate_states)
                    .label("变换朝向")
                    .on_toggle(move |value| action(Action::Toggle(Flag::RotateStates, value))),
                checkbox(params.entities)
                    .label("包含实体树")
                    .on_toggle(move |value| action(Action::Toggle(Flag::Entities, value))),
                checkbox(params.biomes)
                    .label("包含生物群系")
                    .on_toggle(move |value| action(Action::Toggle(Flag::Biomes, value)))
            ]
            .spacing(5)
            .wrap(),
        )
        .push(
            row![
                checkbox(params.skip_air)
                    .label("跳过空气")
                    .on_toggle(move |value| action(Action::Toggle(Flag::SkipAir, value))),
                checkbox(params.remove_be)
                    .label("允许覆盖目标方块实体与物品")
                    .on_toggle(move |value| action(Action::Toggle(Flag::RemoveBe, value)))
            ]
            .spacing(5)
            .wrap(),
        );
    if kind == Some(TransferKind::Clone) {
        body = body
            .push(
                text_input("重复份数", &params.copies)
                    .on_input(move |value| action(Action::Text(Field::Copies, value))),
            )
            .push(
                checkbox(params.rotate_repeats)
                    .label("逐份累加旋转")
                    .on_toggle(move |value| action(Action::Toggle(Flag::RotateRepeats, value))),
            )
            .push(
                checkbox(params.rotate_offset)
                    .label("逐份旋转位移方向")
                    .on_toggle(move |value| action(Action::Toggle(Flag::RotateOffset, value))),
            );
    }
    if kind.is_some() {
        body = body.push(text("克隆生成新实体身份，移动保留原身份。目标实际写入格优先，源生物群系保留。修改变换或重复参数后先更新预览。"));
    }
    if kind.is_none()
        && state
            .source
            .as_ref()
            .is_some_and(|source| !source.entities.is_empty() && source.entity_dimension.is_none())
    {
        body = body.push(
            row![
                text_input("实体原先所在维度", &state.dimension)
                    .on_input(move |value| action(Action::Dimension(value))),
                button("绑定来源维度").on_press(action(Action::BindDimension))
            ]
            .spacing(5),
        );
    }
    body = body
        .push(crate::block_edit::budget_view(app))
        .push(review(app, id));
    body.into()
}
pub(crate) fn review(app: &App, id: u64) -> Element<'_, Message> {
    let entry = &app.structures.files[&id];
    let state = &entry.import;
    let action = move |action| send(id, action);
    let editable = !entry.readonly && !entry.busy && entry.recovery.is_none();
    let mut moves = row![].spacing(4);
    for (axis, label, delta) in [
        (0, "X−", -1),
        (0, "X+", 1),
        (1, "Y−", -1),
        (1, "Y+", 1),
        (2, "Z−", -1),
        (2, "Z+", 1),
    ] {
        moves = moves.push(button(label).on_press_maybe(
            (editable && state.source.is_some()).then_some(action(Action::Translate(axis, delta))),
        ));
    }
    let mut body=column![moves.wrap(),row![button("更新预览").on_press_maybe((editable&&state.source.is_some()).then_some(action(Action::Preview))),
        button("旋转 90°").on_press_maybe((editable&&state.source.is_some()).then_some(action(Action::RotateY))),
        button("确认放置").on_press_maybe((editable&&entry.document.as_ref().is_some_and(|document|state.can_confirm(document))).then_some(action(Action::Confirm))),
        button("取消放置").on_press_maybe(state.engaged().then_some(action(Action::Cancel)))].spacing(5).wrap(),
        row![text(format!("放置参数 {}/{}",state.cursor,state.history.len().saturating_sub(1))),button("撤销参数").on_press_maybe((editable&&!state.running&&state.active&&state.cursor>0).then_some(action(Action::History(false)))),
            button("重做参数").on_press_maybe((editable&&!state.running&&state.active&&state.cursor+1<state.history.len()).then_some(action(Action::History(true))))].spacing(5).wrap(),
        text("左键拖动候选框可平移；先检查位置和内容，确认后才修改文档。不会扩展文档边界；版本或载荷不能表达时会报告错误。").size(12)
    ].spacing(6);
    if let Some(ready) = &state.ready {
        body = body.push(text(ready.proof.summary()).size(12));
    }
    if state.running {
        body = body.push(text("正在读取或构建完整放置候选…").size(12));
    }
    body.into()
}
