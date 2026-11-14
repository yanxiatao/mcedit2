//! 插件的选区、视图与显式保存请求随完整候选交接，不在脚本线程改变界面或文件。
use super::*;
use crate::keyboard_bindings::Owner;
use mcedit2_world::{
    editing::{script, SelectionRegion, Session},
    structure_document::{startup, Document},
};
type Key = (usize, usize);
#[derive(Debug, Clone)]
pub(crate) struct Views {
    pub worlds: BTreeMap<Key, crate::selection_edit::ScriptTicket>,
    pub structures: BTreeMap<u64, (crate::structure_workspace::startup::Ticket, Document)>,
    pub dimensions: BTreeMap<usize, Key>,
    pub tabs: Vec<application::Target>,
    pub private: BTreeMap<usize, String>,
    owner: Option<Owner>,
    camera: crate::spectator::Camera,
}
impl Views {
    pub fn new(app: &App) -> Self {
        Self {
            worlds: BTreeMap::new(),
            structures: BTreeMap::new(),
            dimensions: BTreeMap::new(),
            tabs: Vec::new(),
            private: BTreeMap::new(),
            owner: crate::keyboard_bindings::owner(app),
            camera: app.spectator_camera,
        }
    }
}
#[derive(Debug, Clone)]
struct StructureView {
    ticket: crate::structure_workspace::startup::Ticket,
    document: Document,
    selection: Option<Option<SelectionRegion>>,
    focus: Option<script::Focus>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
enum SaveTarget {
    Existing(crate::tab_bar::Id),
    Private(String),
}
#[derive(Debug, Clone, Default)]
pub(crate) struct Effects {
    selections: BTreeMap<Key, (crate::selection_edit::ScriptTicket, Option<SelectionRegion>)>,
    structures: BTreeMap<u64, StructureView>,
    dimensions: BTreeMap<usize, (usize, Option<Owner>, crate::spectator::Camera)>,
    focus: Option<(Key, [f64; 3], f64, Option<Owner>, crate::spectator::Camera)>,
    saves: Vec<SaveTarget>,
    application: Vec<application::Deferred>,
}
impl Effects {
    pub(crate) fn append_application(
        &mut self,
        actions: Vec<mcedit2_plugins::application::Action>,
        views: &Views,
    ) -> Result<(), String> {
        for action in actions {
            if self.application.iter().any(application::Deferred::exits) {
                return Err("退出请求之后不能继续追加应用操作".into());
            }
            self.application.push(application::Deferred::bind(
                action,
                &views.tabs,
                &views.private,
            )?);
        }
        Ok(())
    }
    pub(crate) fn storage_bytes(&self) -> Result<usize, String> {
        let bytes = self.application.iter().try_fold(0usize, |total, action| {
            total
                .checked_add(action.storage_bytes()?)
                .ok_or_else(|| "应用请求累计工作集溢出".to_owned())
        })?;
        self.saves.iter().try_fold(bytes, |bytes, target| {
            bytes
                .checked_add(std::mem::size_of::<SaveTarget>())
                .and_then(|bytes| {
                    bytes.checked_add(match target {
                        SaveTarget::Private(identity) => identity.len(),
                        _ => 0,
                    })
                })
                .ok_or("显式保存请求累计工作集溢出".into())
        })
    }
    pub(crate) fn continue_inputs(
        &self,
        documents: &mut documents::Documents,
    ) -> Result<(), String> {
        let mut directory = mcedit2_plugins::application::Directory {
            tabs: documents.tabs.clone(),
            active_tab: documents.active_tab.clone(),
        };
        for action in self
            .application
            .iter()
            .filter_map(application::Deferred::navigation)
        {
            directory.navigate(&action)?;
        }
        documents.order = directory
            .tabs
            .iter()
            .filter_map(|tab| tab.document)
            .collect();
        documents.tabs = directory.tabs;
        documents.active_tab = directory.active_tab;
        for (slot, (world, initial)) in &mut documents.views.dimensions {
            if let Some((target, _, _)) = self.dimensions.get(world) {
                let name = documents
                    .dimensions
                    .iter()
                    .find_map(|((source, name), key)| {
                        (*source == *slot && *key == (*world, *target)).then_some(name.clone())
                    })
                    .ok_or("前序插件切换的维度已经移除")?;
                documents.worlds[*slot].dimension = name;
                *initial = *target;
            }
        }
        for ((slot, name), key) in &documents.dimensions {
            if let Some((_, region)) = self.selections.get(key) {
                documents.worlds[*slot]
                    .selections
                    .insert(name.clone(), *region);
            }
        }
        for (slot, id) in &documents.structure_slots {
            if let Some(view) = self.structures.get(id) {
                if let Some(selected) = view.selection {
                    let input = &mut documents.structures[*slot - documents.worlds.len()];
                    input.selection = selected
                        .map(|region| {
                            input
                                .document
                                .script_local_region(region)
                                .map_err(|error| error.to_string())
                        })
                        .transpose()?;
                }
            }
        }
        Ok(())
    }
    pub fn pending(&self) -> bool {
        !self.selections.is_empty()
            || !self.structures.is_empty()
            || !self.dimensions.is_empty()
            || self.focus.is_some()
            || !self.saves.is_empty()
            || !self.application.is_empty()
    }
    pub fn summary(&self) -> String {
        format!(
            "选区 / 视图请求 {} 项，显式保存 {} 个文档，应用请求 {} 项",
            self.selections.len()
                + self.structures.len()
                + self.dimensions.len()
                + usize::from(self.focus.is_some()),
            self.saves.len(),
            self.application.len()
        )
    }
    /// 在输出仍保有真实槽位与最终文档时转换身份；后续候选不再依赖槽位顺序。
    pub fn merge(
        &mut self,
        output: &script::Output,
        views: &Views,
        dimensions: &BTreeMap<(usize, String), Key>,
        slots: &BTreeMap<usize, u64>,
    ) -> Result<(), String> {
        self.merge_with_saves(output, views, dimensions, slots, &output.save_order)
    }
    pub(crate) fn merge_with_saves(
        &mut self,
        output: &script::Output,
        views: &Views,
        dimensions: &BTreeMap<(usize, String), Key>,
        slots: &BTreeMap<usize, u64>,
        save_order: &[usize],
    ) -> Result<(), String> {
        for ((slot, name), selected) in &output.selections {
            if let Some(id) = slots.get(slot) {
                let (ticket, baseline) = &views.structures[id];
                let document = output
                    .structures
                    .iter()
                    .find(|result| result.slot == *slot)
                    .map_or(baseline, |result| &result.document)
                    .clone();
                let view = self.structures.entry(*id).or_insert_with(|| StructureView {
                    ticket: ticket.clone(),
                    document: document.clone(),
                    selection: None,
                    focus: None,
                });
                view.ticket = ticket.clone();
                view.document = document;
                view.selection = Some(*selected);
            } else {
                let key = *dimensions
                    .get(&(*slot, name.clone()))
                    .ok_or("插件选区目标维度不在实际目录中")?;
                self.selections
                    .insert(key, (views.worlds[&key].clone(), *selected));
            }
        }
        for (slot, &(world, initial)) in &views.dimensions {
            let target = dimensions
                .get(&(*slot, output.dimensions[*slot].clone()))
                .ok_or("插件切换的维度不在实际目录中")?
                .1;
            if target != initial {
                self.dimensions
                    .insert(world, (target, views.owner, views.camera));
            }
        }
        if let Some(focus) = &output.focus {
            if let Some(id) = slots.get(&focus.world) {
                let (ticket, baseline) = &views.structures[id];
                let document = output
                    .structures
                    .iter()
                    .find(|result| result.slot == focus.world)
                    .map_or(baseline, |result| &result.document)
                    .clone();
                let view = self.structures.entry(*id).or_insert_with(|| StructureView {
                    ticket: ticket.clone(),
                    document: document.clone(),
                    selection: None,
                    focus: None,
                });
                view.ticket = ticket.clone();
                view.document = document;
                view.focus = Some(focus.clone());
            } else {
                let key = *dimensions
                    .get(&(focus.world, focus.dimension.clone()))
                    .ok_or("插件定位目标维度不在实际目录中")?;
                self.dimensions
                    .insert(key.0, (key.1, views.owner, views.camera));
                self.focus = Some((
                    key,
                    focus.position,
                    focus.distance,
                    views.owner,
                    views.camera,
                ));
            }
        }
        for slot in save_order {
            let target = if let Some(identity) = views.private.get(slot) {
                SaveTarget::Private(identity.clone())
            } else {
                SaveTarget::Existing(match slots.get(slot) {
                    Some(id) => crate::tab_bar::Id::Structure(*id),
                    None => crate::tab_bar::Id::World(
                        views
                            .dimensions
                            .get(slot)
                            .ok_or("插件保存目标不在实际目录中")?
                            .0,
                    ),
                })
            };
            if !self.saves.contains(&target) {
                self.saves.push(target);
            }
        }
        Ok(())
    }
    pub(crate) fn prepare(
        self,
        app: &App,
        sessions: &[(Key, Session)],
        structures: &[(
            u64,
            startup::Output,
            Vec<mcedit2_world::structure_document::ObjectSummary>,
        )],
        primary: Option<(u64, &Document)>,
    ) -> Result<Publication, String> {
        self.prepare_with_opened(app, sessions, structures, primary, &[], &BTreeMap::new())
    }
    pub(crate) fn prepare_with_opened(
        self,
        app: &App,
        sessions: &[(Key, Session)],
        structures: &[(
            u64,
            startup::Output,
            Vec<mcedit2_world::structure_document::ObjectSummary>,
        )],
        primary: Option<(u64, &Document)>,
        opened: &[application::Target],
        opened_saves: &BTreeMap<String, Option<crate::startup_script_ui::SaveRequest>>,
    ) -> Result<Publication, String> {
        let mut views = Vec::new();
        if !self.application.is_empty() && app.plugins.application.busy() {
            return Err("请先完成或取消已确认的应用请求，再确认下一组".into());
        }
        let application = self
            .application
            .iter()
            .map(|action| action.resolve(app, opened))
            .collect::<Result<Vec<_>, _>>()?;
        for (id, view) in &self.structures {
            let document = structures
                .iter()
                .find(|(target, _, _)| target == id)
                .map(|(_, result, _)| &result.document)
                .or_else(|| {
                    primary
                        .filter(|(target, _)| target == id)
                        .map(|(_, document)| document)
                })
                .unwrap_or(&view.document);
            views.push(crate::structure_workspace::startup::prepare_view(
                app,
                &view.ticket,
                document,
                view.selection,
                view.focus.as_ref(),
            )?);
        }
        if let Some((_, point, distance, _, _)) = self.focus {
            app.point_camera(point, distance)?;
        }
        if !self.saves.is_empty() && app.startup_script.busy() {
            return Err("已有脚本或保存任务，请完成后再确认插件保存请求".into());
        }
        let mut saves = Vec::new();
        for target in &self.saves {
            let target = match target {
                SaveTarget::Existing(target) => *target,
                SaveTarget::Private(identity) => {
                    if let Some(request) = opened_saves
                        .get(identity)
                        .ok_or("显式保存的私有文档尚未准备发布")?
                    {
                        saves.push(request.clone());
                    }
                    continue;
                }
            };
            saves.push(match target {
                crate::tab_bar::Id::World(index) => {
                    let mut values = Vec::new();
                    for (key, baseline) in app.sessions.iter().filter(|(key, _)| key.0 == index) {
                        let value = sessions
                            .iter()
                            .find(|(target, _)| target == key)
                            .map_or(baseline, |(_, session)| session);
                        if value.has_pending_changes() {
                            values.push(value.clone());
                        }
                    }
                    if values.is_empty() {
                        continue;
                    }
                    crate::startup_script_ui::SaveRequest::World(crate::startup_script_ui::Save {
                        root: app.worlds[index].root.clone(),
                        sessions: values,
                    })
                }
                crate::tab_bar::Id::Structure(id) => {
                    let document = if let Some((_, result, _)) =
                        structures.iter().find(|(target, _, _)| *target == id)
                    {
                        &result.document
                    } else if let Some((_, document)) = primary.filter(|(target, _)| *target == id)
                    {
                        document
                    } else {
                        app.structures.files[&id]
                            .document
                            .as_ref()
                            .ok_or("插件保存目标结构已关闭")?
                    };
                    crate::startup_script_ui::SaveRequest::Structure(id, document.clone())
                }
                _ => unreachable!("插件保存仅针对真实文档"),
            });
        }
        Ok(Publication {
            effects: self,
            structures: views,
            saves,
            application,
        })
    }
}
pub(crate) struct Publication {
    effects: Effects,
    structures: Vec<crate::structure_workspace::startup::Publication>,
    saves: Vec<crate::startup_script_ui::SaveRequest>,
    application: Vec<application::Bound>,
}
impl Publication {
    pub(crate) fn summary(&self) -> String {
        if self.saves.is_empty() {
            "未请求写盘，修改仍在内存中。".into()
        } else {
            format!(
                "将依次保存 {} 个文档，实际结果见脚本保存任务。",
                self.saves.len()
            )
        }
    }
    pub(crate) fn publish(self, app: &mut App) -> Task<Message> {
        let mut tasks = Vec::new();
        let initial_owner = crate::keyboard_bindings::owner(app);
        let initial_camera = app.spectator_camera;
        for (key, (ticket, selection)) in self.effects.selections {
            crate::selection_edit::apply_completed(app, key, selection, &ticket);
        }
        for (world, (dimension, origin, camera)) in self.effects.dimensions {
            app.workspace
                .session_dimensions
                .insert(world, app.worlds[world].dimensions[dimension].name.clone());
            if app.current == Some(world)
                && app.structures.active.is_none()
                && app.dimension_index != dimension
                && initial_owner == origin
                && initial_camera == camera
            {
                tasks.push(crate::update_inner(app, Message::Dimension(dimension)));
            }
        }
        for view in self.structures {
            tasks.push(crate::structure_workspace::startup::publish(app, view));
        }
        if let Some((key, point, distance, owner, camera)) = self.effects.focus {
            if initial_owner == owner
                && owner
                    .is_some_and(|owner| matches!(owner, Owner::World(world, _) if world == key.0))
                && initial_camera == camera
            {
                if let Err(error) = locate(app, point, distance) {
                    log::error!("[plugin-focus] {error}");
                    app.plugins.report.push_str(&format!("\n{error}"));
                }
            } else if app.current != Some(key.0) || app.structures.active.is_some() {
                app.workspace.session_focus.insert(
                    key.0,
                    (
                        app.worlds[key.0].dimensions[key.1].name.clone(),
                        point,
                        distance,
                    ),
                );
            } else {
                log::info!("[plugin-focus] 用户已改变活动视图，保留当前位置");
            }
        }
        let saving = !self.saves.is_empty();
        tasks.push(crate::startup_script_ui::accept_saves(app, self.saves));
        let outcome = if saving {
            app.startup_script.outcome.clone()
        } else {
            None
        };
        application::enqueue(app, self.application, outcome);
        Task::batch(tasks)
    }
}
fn locate(app: &mut App, point: [f64; 3], distance: f64) -> Result<(), String> {
    let camera = app.point_camera(point, distance)?;
    let advanced = app.show_advanced;
    app.locate_camera(camera, point)?;
    app.show_advanced = advanced;
    Ok(())
}
pub(crate) fn restore_focus(app: &mut App, index: usize) {
    if let Some((dimension, point, distance)) = app.workspace.session_focus.remove(&index) {
        if app.worlds[index].dimensions[app.dimension_index].name == dimension {
            if let Err(error) = locate(app, point, distance) {
                log::error!("[plugin-focus] {error}");
                app.plugins.report = error;
            }
        }
    }
}
