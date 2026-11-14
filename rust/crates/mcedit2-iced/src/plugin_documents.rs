//! 插件读取当前已建立的真实文档会话；世界、结构及标签顺序使用同一槽位表。
use super::*;
use mcedit2_world::{
    editing::{script, SelectionRegion, Session},
    structure_document::{startup as structure, Document},
};

#[derive(Debug, Clone)]
pub(crate) struct StructureSource {
    pub ticket: crate::structure_workspace::startup::Ticket,
    pub baseline: Document,
}
#[derive(Debug, Clone, Default)]
pub(crate) struct Sources {
    pub worlds: BTreeMap<(usize, usize), Arc<()>>,
    pub structures: BTreeMap<u64, StructureSource>,
}
impl Sources {
    pub fn contains(&self, id: crate::tab_bar::Id) -> bool {
        match id {
            crate::tab_bar::Id::World(world) => self.worlds.keys().any(|key| key.0 == world),
            crate::tab_bar::Id::Structure(id) => self.structures.contains_key(&id),
            crate::tab_bar::Id::File(_) | crate::tab_bar::Id::Panel(_) => false,
        }
    }
    pub fn single(key: (usize, usize), revision: Arc<()>) -> Self {
        Self {
            worlds: BTreeMap::from([(key, revision)]),
            structures: BTreeMap::new(),
        }
    }
    pub fn matches(&self, app: &App) -> bool {
        self.worlds.iter().all(|(key, revision)| {
            app.workspace.contains(key.0)
                && app
                    .sessions
                    .get(key)
                    .is_some_and(|session| Arc::ptr_eq(&session.revision(), revision))
        }) && self.structures.values().all(|source| {
            crate::structure_workspace::startup::matches(app, &source.ticket, &source.baseline)
        })
    }
}
pub(crate) struct Documents {
    pub worlds: Vec<script::Input>,
    pub structures: Vec<structure::Input>,
    pub order: Vec<usize>,
    pub identities: Vec<String>,
    pub primary: usize,
    pub baselines: BTreeMap<(usize, usize), Session>,
    pub dimensions: BTreeMap<(usize, String), (usize, usize)>,
    pub structure_slots: BTreeMap<usize, u64>,
    pub sources: Sources,
    pub views: super::session_effects::Views,
    pub tabs: Vec<mcedit2_plugins::application::Tab>,
    pub active_tab: Option<String>,
    pub private_documents: Vec<script::OpenedOutput>,
    pub opening_readonly: bool,
}
pub(super) fn collect(
    app: &mut App,
    key: (usize, usize),
    selection: Option<SelectionRegion>,
    previous: Option<&world_tool::Candidate>,
) -> Result<Documents, String> {
    let mut documents = collect_for(
        app,
        crate::tab_bar::Id::World(key.0),
        Some(key),
        selection,
        previous,
    )?;
    if let Some(previous) = previous {
        previous.effects.continue_inputs(&mut documents)?;
    }
    Ok(documents)
}
pub(crate) fn collect_structure(
    app: &mut App,
    id: u64,
    document: Document,
    selection: Option<SelectionRegion>,
    previous: Option<&world_tool::Candidate>,
) -> Result<Documents, String> {
    let mut documents = collect_for(
        app,
        crate::tab_bar::Id::Structure(id),
        None,
        selection,
        previous,
    )?;
    let slot = documents.primary - documents.worlds.len();
    documents.structures[slot].document = document;
    documents.structures[slot].selection = selection;
    // 主文件由结构插件自己的修订、代次和历史证明负责；公共来源只复核其他文档。
    documents.sources.structures.remove(&id);
    if let Some(previous) = previous {
        previous.effects.continue_inputs(&mut documents)?;
    }
    Ok(documents)
}
/// 菜单导入也绑定真实主文档；加载目录期间不允许写入任何文档。
pub(super) fn inspect_structure(app: &mut App, id: u64) -> Result<Task<Message>, String> {
    let (source, enabled) = app.plugins.source()?;
    if !enabled {
        return Err("请先启用所选插件".into());
    }
    source.verify_unchanged()?;
    if source.kind == PluginKind::Declarative {
        return Err("声明式插件没有 Python 命令目录".into());
    }
    if !crate::structure_workspace::startup::ready(app, id)? {
        return Err("请先完成结构文档读取".into());
    }
    let documents = collect_for(app, crate::tab_bar::Id::Structure(id), None, None, None)?;
    let readonly = documents.structures[documents.primary - documents.worlds.len()]
        .document
        .read_only();
    let budget = app.block_edit.budget.parse()?;
    let timeout = app.plugins.timeout.parse()?;
    invalidate(&mut app.plugins);
    app.plugins.cancelled = Arc::new(AtomicBool::new(false));
    app.plugins.busy = true;
    app.plugins.report = "正在实际文档目录中加载插件命令…".into();
    let nonce = app.plugins.nonce;
    let cancelled = Arc::clone(&app.plugins.cancelled);
    let finished_source = source.clone();
    let sources = documents.sources.clone();
    Ok(background(
        move || {
            let mut request = RunRequest::inspect(&source);
            request.read_only = readonly;
            request.work_budget_bytes = budget.bytes() as u64;
            let mut context = mcedit2_plugins::tool_rpc::Context::documents(
                documents.worlds,
                documents.structures,
                documents.primary,
                documents.order,
                documents.identities,
                None,
                false,
                budget,
                Arc::clone(&cancelled),
            )?;
            context.bind_application(documents.tabs, documents.active_tab)?;
            context
                .bind_private_documents(&documents.private_documents, documents.opening_readonly)?;
            context.restrict_to_reads()?;
            let native = Arc::new(Mutex::new(context));
            let output = mcedit2_plugins::run_native(
                &std::env::current_exe().map_err(|error| error.to_string())?,
                &request,
                &RunLimits {
                    cancelled,
                    timeout,
                    transfer_budget: budget.bytes() as u64,
                },
                Arc::clone(&native),
            )?;
            if output.structure.is_some() || output.inspector_operation.is_some() {
                return Err("加载命令目录不能返回生成结构或 Inspector 修改".into());
            }
            let completed = native
                .lock()
                .expect("插件命令目录读取锁被毒化")
                .finish_command(&output, None)?;
            if !completed.output.opened.is_empty() || !completed.application.is_empty() {
                return Err("加载命令目录不能执行文档编辑或应用操作".into());
            }
            let mut effects = super::session_effects::Effects::default();
            let completed = completed.output;
            effects.merge(
                &completed,
                &documents.views,
                &documents.dimensions,
                &documents.structure_slots,
            )?;
            if !completed.changes.is_empty()
                || !completed.selections.is_empty()
                || completed.focus.is_some()
                || !completed.save_order.is_empty()
                || effects.pending()
            {
                return Err("加载命令目录不能执行文档编辑或应用操作".into());
            }
            source.verify_unchanged()?;
            Ok(output)
        },
        move |result| {
            Action::StructureCatalog(
                nonce,
                id,
                finished_source.clone(),
                sources.clone(),
                budget,
                result,
            )
        },
    ))
}
fn collect_for(
    app: &mut App,
    owner: crate::tab_bar::Id,
    key: Option<(usize, usize)>,
    selection: Option<SelectionRegion>,
    previous: Option<&world_tool::Candidate>,
) -> Result<Documents, String> {
    if previous.is_some_and(|previous| !previous.opened.matches(app)) {
        return Err("私有新文档已被其他原生请求打开，请重新运行插件".into());
    }
    let catalog = app
        .workspace
        .order
        .iter()
        .copied()
        .filter(|id| match id {
            crate::tab_bar::Id::World(index) => app.workspace.contains(*index),
            crate::tab_bar::Id::Structure(id) => app
                .structures
                .files
                .get(id)
                .is_some_and(|entry| entry.document.is_some()),
            crate::tab_bar::Id::File(_) | crate::tab_bar::Id::Panel(_) => false,
        })
        .collect::<Vec<_>>();
    let mut worlds = Vec::new();
    let mut identities = Vec::new();
    let mut world_slots = BTreeMap::new();
    let mut baselines = BTreeMap::new();
    let mut dimensions = BTreeMap::new();
    let mut views = super::session_effects::Views::new(app);
    for id in &catalog {
        let crate::tab_bar::Id::World(index) = *id else {
            continue;
        };
        let slot = worlds.len();
        let active = if let Some(key) = key.filter(|key| index == key.0) {
            key.1
        } else if app.current == Some(index) {
            app.dimension_index
        } else {
            app.initial_dimension(&app.worlds[index])
        };
        let dimension = app.worlds[index]
            .dimensions
            .get(active)
            .ok_or("插件文档没有实际选中的维度")?
            .name
            .clone();
        let mut selections = BTreeMap::new();
        let mut sessions = BTreeMap::new();
        for dim in 0..app.worlds[index].dimensions.len() {
            let target = (index, dim);
            let baseline = app.current_session(target)?.clone();
            let session = match previous.and_then(|previous| previous.changes.get(&target)) {
                Some(prepared) => prepared
                    .continuation(&baseline)
                    .map_err(|error| error.to_string())?,
                None => baseline.clone(),
            };
            let selected = if Some(target) == key {
                selection
            } else {
                crate::selection_edit::script_selection(app, target)?
            };
            selections.insert(session.dimension().to_owned(), selected);
            dimensions.insert((slot, session.dimension().to_owned()), target);
            views
                .worlds
                .insert(target, crate::selection_edit::script_ticket(app, target));
            sessions.insert(session.dimension().to_owned(), session);
            baselines.insert(target, baseline);
        }
        worlds.push(script::Input {
            world: app.worlds[index].clone(),
            dimension,
            selections,
            sessions,
            readonly: crate::workspace_tabs::world_readonly(app, index),
        });
        world_slots.insert(index, slot);
        views.dimensions.insert(slot, (index, active));
        identities.push(
            super::application::identity(app, crate::tab_bar::Id::World(index))
                .ok_or("插件世界会话已关闭")?,
        );
    }
    if let Some(previous) = previous {
        for (identity, output) in previous.opened.documents() {
            if let script::OpenedDocument::World { input, .. } = &output.document {
                let slot = worlds.len();
                worlds.push(input.clone());
                identities.push(identity.to_owned());
                views.private.insert(slot, identity.to_owned());
            }
        }
    }
    let mut sources = Sources {
        worlds: baselines
            .iter()
            .map(|(key, session)| (*key, session.revision()))
            .collect(),
        structures: BTreeMap::new(),
    };
    let mut structures = Vec::new();
    let mut structure_slots = BTreeMap::new();
    let mut slots = BTreeMap::new();
    for id in &catalog {
        let crate::tab_bar::Id::Structure(id) = *id else {
            continue;
        };
        let (ticket, mut input) = if owner == crate::tab_bar::Id::Structure(id) {
            crate::structure_workspace::startup::plugin_input(app, id)?
        } else {
            crate::structure_workspace::startup::input(app, id)?
        };
        sources.structures.insert(
            id,
            StructureSource {
                ticket,
                baseline: input.document.clone(),
            },
        );
        views.structures.insert(
            id,
            (
                sources.structures[&id].ticket.clone(),
                input.document.clone(),
            ),
        );
        if let Some(previous) = previous.and_then(|previous| previous.structures.get(&id)) {
            input.document = previous.document.clone();
        }
        let slot = worlds.len() + structures.len();
        structures.push(input);
        structure_slots.insert(slot, id);
        slots.insert(id, slot);
        identities.push(
            super::application::identity(app, crate::tab_bar::Id::Structure(id))
                .ok_or("插件结构会话已关闭")?,
        );
    }
    if let Some(previous) = previous {
        for (identity, output) in previous.opened.documents() {
            if let script::OpenedDocument::Structure { document, .. } = &output.document {
                let slot = worlds.len() + structures.len();
                let selected = output.selections[&document.content().dimension]
                    .map(|region| {
                        document
                            .script_local_region(region)
                            .map_err(|error| error.to_string())
                    })
                    .transpose()?;
                structures.push(structure::Input {
                    document: document.clone(),
                    selection: selected,
                });
                identities.push(identity.to_owned());
                views.private.insert(slot, identity.to_owned());
            }
        }
    }
    let mut order: Vec<usize> = catalog
        .iter()
        .map(|id| match *id {
            crate::tab_bar::Id::World(index) => world_slots[&index],
            crate::tab_bar::Id::Structure(id) => slots[&id],
            crate::tab_bar::Id::File(_) | crate::tab_bar::Id::Panel(_) => {
                unreachable!("目录只有实际编辑会话")
            }
        })
        .collect();
    let primary = match owner {
        crate::tab_bar::Id::World(index) => *world_slots.get(&index).ok_or("插件主世界已经关闭")?,
        crate::tab_bar::Id::Structure(id) => *slots.get(&id).ok_or("插件主结构已经关闭")?,
        _ => unreachable!("插件操作只归属实际编辑文档"),
    };
    let mut tabs = Vec::new();
    for id in app.workspace.order.iter().copied() {
        if let Some(identity) = application::identity(app, id) {
            let document = match id {
                crate::tab_bar::Id::World(index) => world_slots.get(&index).copied(),
                crate::tab_bar::Id::Structure(id) => slots.get(&id).copied(),
                _ => unreachable!("实际应用标签为世界或结构"),
            };
            tabs.push(mcedit2_plugins::application::Tab {
                identity: identity.clone(),
                document,
            });
            views.tabs.push(application::Target { identity, id });
        }
    }
    if let Some(previous) = previous {
        for (identity, _) in previous.opened.documents() {
            let slot = views
                .private
                .iter()
                .find_map(|(slot, value)| (value == identity).then_some(*slot))
                .expect("每份私有文档已交给真实输入槽位");
            order.push(slot);
            tabs.push(mcedit2_plugins::application::Tab {
                identity: identity.to_owned(),
                document: Some(slot),
            });
        }
    }
    let private_documents = previous
        .map(|previous| previous.opened.inputs(&views.private))
        .transpose()?
        .unwrap_or_default();
    let active_tab =
        crate::workspace_tabs::active_tab(app).and_then(|id| application::identity(app, id));
    Ok(Documents {
        worlds,
        structures,
        order,
        identities,
        primary,
        baselines,
        dimensions,
        structure_slots,
        sources,
        views,
        tabs,
        active_tab,
        private_documents,
        opening_readonly: app.readonly_global,
    })
}
pub(crate) fn merge_structures(
    sources: &Sources,
    slots: &BTreeMap<usize, u64>,
    candidates: &mut BTreeMap<u64, structure::Output>,
    following: Vec<structure::Output>,
    budget: EditWorkBudget,
) -> Result<bool, String> {
    let mut changed = false;
    for mut following in following {
        // 显式保存已由会话请求收集；文件历史证明只负责内容，发布后再进入持久事务。
        following.save = false;
        let id = *slots
            .get(&following.slot)
            .ok_or("插件结构结果不属于本次实际文档目录")?;
        let source = &sources.structures[&id];
        let previous = candidates.get(&id);
        changed |= !previous
            .map_or(&source.baseline, |previous| &previous.document)
            .same_source(&following.document);
        let result = source
            .baseline
            .compose_startup_result(previous, following, budget)
            .map_err(|error| error.to_string())?;
        if source.baseline.same_source(&result.document) {
            candidates.remove(&id);
        } else {
            candidates.insert(id, result);
        }
    }
    Ok(changed)
}
