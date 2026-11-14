//! 脚本实际新文档的整组发布；先预建完整标签，正式安装时不重新读取内容。
use super::application::Target;
use crate::{tab_bar::Id, App, Message};
use iced::Task;
use mcedit2_plugins::application::Directory;
use mcedit2_world::{
    editing::{script, EditWorkBudget},
    player_catalog::Locations,
    structure_document::ObjectSummary,
};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};
#[path = "script_private_documents.rs"]
mod private;
pub(crate) use private::{Absorbed, Candidates};

#[derive(Debug, Clone)]
enum Details {
    World(Locations),
    Structure(Vec<ObjectSummary>),
}
#[derive(Debug, Clone)]
pub(crate) struct Ready {
    output: script::OpenedOutput,
    details: Details,
}
impl Ready {
    /// 来源位置和对象摘要沿用原生领域读取器，并在后台完成。
    pub(crate) fn read(
        output: script::OpenedOutput,
        budget: EditWorkBudget,
        cancelled: &AtomicBool,
    ) -> Result<Self, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("脚本新文档准备已取消".into());
        }
        let details = match &output.document {
            script::OpenedDocument::World { input, .. } => {
                if mcedit2_world::mixed_transaction::pending_summary(&input.world)
                    .map_err(|error| error.to_string())?
                    .is_some()
                {
                    return Err("新世界存在未完成保存事务，请先处理原生保存恢复".into());
                }
                let locations = mcedit2_world::player_catalog::locations(&input.world)
                    .map_err(|error| error.to_string())?;
                for diagnostic in &locations.diagnostics {
                    log::error!("[script-open] {diagnostic}");
                }
                Details::World(locations)
            }
            script::OpenedDocument::Structure { document, .. } => {
                if mcedit2_world::structure_document::save::pending(document.path(), budget)
                    .map_err(|error| error.to_string())?
                    .is_some()
                {
                    return Err("新结构存在未完成保存事务，请先处理原生文件恢复".into());
                }
                Details::Structure(
                    document
                        .object_summaries(|| cancelled.load(Ordering::Acquire))
                        .map_err(|error| error.to_string())?,
                )
            }
        };
        if cancelled.load(Ordering::Acquire) {
            return Err("脚本新文档准备已取消".into());
        }
        Ok(Self { output, details })
    }
}
enum Entry {
    World {
        index: usize,
        input: script::Input,
        locations: Locations,
        focus: Option<script::Focus>,
    },
    Structure(crate::structure_workspace::startup::Opened),
}
pub(crate) struct Publication {
    entries: Vec<Entry>,
    targets: Vec<Target>,
    saves: BTreeMap<usize, crate::startup_script_ui::SaveRequest>,
    slot_identities: BTreeMap<usize, String>,
}
impl Publication {
    pub(crate) fn targets(&self) -> &[Target] {
        &self.targets
    }
    pub(crate) fn take_saves(&mut self) -> BTreeMap<usize, crate::startup_script_ui::SaveRequest> {
        std::mem::take(&mut self.saves)
    }
    pub(crate) fn take_named_saves(
        &mut self,
    ) -> BTreeMap<String, Option<crate::startup_script_ui::SaveRequest>> {
        self.slot_identities
            .iter()
            .map(|(slot, identity)| (identity.clone(), self.saves.remove(slot)))
            .collect()
    }
    /// 调用方已共同核对既有来源、所有新标签和应用目标；本段只安装预建结果。
    pub(crate) fn publish(self, app: &mut App) -> Task<Message> {
        let mut tasks = Vec::new();
        for (entry, target) in self.entries.into_iter().zip(self.targets) {
            match entry {
                Entry::World {
                    index,
                    input,
                    locations,
                    focus,
                } => {
                    let root = input.world.root.clone();
                    let registered =
                        crate::workspace_tabs::register_world(app, input.world, input.readonly);
                    assert_eq!(registered, index, "整组预检已核对真实世界登记位置");
                    app.workspace
                        .session_dimensions
                        .insert(index, input.dimension);
                    app.source_locations.insert(root, locations);
                    for (name, session) in input.sessions {
                        let dim = app.worlds[index]
                            .dimensions
                            .iter()
                            .position(|dim| dim.name == name)
                            .expect("新世界的完整会话维度已在内核验证");
                        let key = (index, dim);
                        app.sessions.insert(key, session);
                        let selected = input.selections[&name];
                        let ticket = crate::selection_edit::script_ticket(app, key);
                        crate::selection_edit::apply_completed(app, key, selected, &ticket);
                    }
                    if let Some(focus) = focus {
                        app.workspace
                            .session_focus
                            .insert(index, (focus.dimension, focus.position, focus.distance));
                    }
                }
                Entry::Structure(opened) => {
                    tasks.push(crate::structure_workspace::startup::publish_open(
                        app, opened,
                    ));
                }
            }
            app.workspace
                .document_identities
                .push((target.id, target.identity));
        }
        Task::batch(tasks)
    }
}
pub(crate) fn prepare(
    app: &App,
    ready: Vec<Ready>,
    directory: &Directory,
) -> Result<Publication, String> {
    let count = directory
        .tabs
        .iter()
        .filter(|tab| tab.document.is_some())
        .count();
    let identities = directory.identities(count)?;
    let named = ready
        .into_iter()
        .map(|ready| {
            let identity = identities
                .get(ready.output.slot())
                .ok_or("新文档没有应用目录身份")?
                .clone();
            Ok((identity, ready))
        })
        .collect::<Result<Vec<_>, String>>()?;
    prepare_named(app, named)
}
pub(crate) fn prepare_named(app: &App, ready: Vec<(String, Ready)>) -> Result<Publication, String> {
    let mut next_world = app.worlds.len();
    let mut structure_offset = 0u64;
    let mut entries = Vec::new();
    let mut targets = Vec::new();
    let mut saves = BTreeMap::new();
    let mut slot_identities = BTreeMap::new();
    for (identity, ready) in ready {
        let slot = ready.output.slot();
        slot_identities.insert(slot, identity.clone());
        if app
            .workspace
            .order
            .iter()
            .any(|id| super::application::identity(app, *id).as_deref() == Some(identity.as_str()))
        {
            return Err("新增文档的应用身份已由其他打开请求占用".into());
        }
        let (entry, id) = match (ready.output.document, ready.details) {
            (script::OpenedDocument::World { input, .. }, Details::World(locations)) => {
                let known = app
                    .worlds
                    .iter()
                    .position(|world| world.root == input.world.root);
                if known.is_some_and(|index| app.workspace.contains(index)) {
                    return Err("脚本执行期间同一世界已由其他请求打开，整组结果未发布".into());
                }
                let index = match known {
                    Some(index) => index,
                    None => {
                        let index = next_world;
                        next_world += 1;
                        index
                    }
                };
                if ready.output.save {
                    let sessions = input
                        .sessions
                        .values()
                        .filter(|session| session.has_pending_changes())
                        .cloned()
                        .collect::<Vec<_>>();
                    if !sessions.is_empty() {
                        saves.insert(
                            slot,
                            crate::startup_script_ui::SaveRequest::World(
                                crate::startup_script_ui::Save {
                                    root: input.world.root.clone(),
                                    sessions,
                                },
                            ),
                        );
                    }
                }
                (
                    Entry::World {
                        index,
                        input,
                        locations,
                        focus: ready.output.focus,
                    },
                    Id::World(index),
                )
            }
            (script::OpenedDocument::Structure { document, .. }, Details::Structure(objects)) => {
                if app
                    .structures
                    .files
                    .values()
                    .any(|entry| entry.path == document.path())
                {
                    return Err("脚本执行期间同一结构已由其他请求打开，整组结果未发布".into());
                }
                structure_offset = structure_offset.checked_add(1).ok_or("新增结构数量溢出")?;
                let id = crate::structure_workspace::startup::next_open_id(app, structure_offset)?;
                if ready.output.save {
                    saves.insert(
                        slot,
                        crate::startup_script_ui::SaveRequest::Structure(id, document.clone()),
                    );
                }
                let selected = ready.output.selections[&document.content().dimension];
                let opened = crate::structure_workspace::startup::prepare_open(
                    app,
                    id,
                    document,
                    objects,
                    selected,
                    ready.output.focus.as_ref(),
                )?;
                (Entry::Structure(opened), Id::Structure(id))
            }
            _ => return Err("新文档内容与原生读取结果类别不同".into()),
        };
        entries.push(entry);
        targets.push(Target { identity, id });
    }
    Ok(Publication {
        entries,
        targets,
        saves,
        slot_identities,
    })
}
