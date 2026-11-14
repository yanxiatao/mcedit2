//! 启动脚本沿现有结构文档完成读取、发布、选区 / 相机跟随及持久保存。
use super::*;
use mcedit2_world::{editing::script, structure_document::startup as model};

pub(crate) struct Opened {
    id: u64,
    entry: Entry,
}
pub(crate) fn next_open_id(app: &App, offset: u64) -> Result<u64, String> {
    app.structures
        .next
        .checked_add(offset)
        .ok_or("结构文档身份已耗尽".into())
}
/// 用真实完成的文档预建整个标签；不触发 Load，也不先登记一个空会话。
pub(crate) fn prepare_open(
    app: &App,
    id: u64,
    document: Document,
    objects: Vec<ObjectSummary>,
    selected: Option<SelectionRegion>,
    focus: Option<&script::Focus>,
) -> Result<Opened, String> {
    let mut entry = Entry::new(
        document.path().to_path_buf(),
        document.format(),
        document.read_only(),
    );
    entry.epoch = 1;
    entry.shape = app.selection_edit.default_shape;
    entry.search = search::State::new(app.nbt_search.document_options());
    if let Some(paths) = crate::resource_bindings::structure(app, &entry.path) {
        entry.scene.paths = paths.resources();
        entry.scene.data_paths = paths.data();
    }
    let content = document.content();
    entry.scene.reset(content.size);
    entry.replacement = if content.numeric {
        "numeric:1[data=0]"
    } else {
        "minecraft:stone"
    }
    .into();
    let local = selected
        .map(|region| {
            document
                .script_local_region(region)
                .map_err(|error| error.to_string())
        })
        .transpose()?;
    entry.document = Some(document);
    entry.objects = objects;
    entry.report = "脚本已打开真实结构文档；编辑保留在内存中，文件历史可撤销。".into();
    entry.recovery_checked = Some((entry.epoch, Ok(())));
    let ticket = selection::script_ticket(&entry);
    let selection = selection::prepare_script(&entry, id, &ticket, local)?
        .expect("尚未发布的新标签没有并发选区变更");
    selection::publish_script(&mut entry, id, selection);
    if let Some(focus) = focus {
        let origin = entry
            .document
            .as_ref()
            .expect("新标签含真实文档")
            .content()
            .source_min;
        apply_focus(
            &mut entry,
            std::array::from_fn(|axis| focus.position[axis] - f64::from(origin[axis])),
            focus.distance,
        );
    }
    Ok(Opened { id, entry })
}
pub(crate) fn publish_open(app: &mut App, opened: Opened) -> Task<Message> {
    let id = opened.id;
    assert_eq!(
        Some(id),
        app.structures.next.checked_add(1),
        "整组预检已分配真实结构身份"
    );
    app.structures.next = id;
    app.structures.files.insert(id, opened.entry);
    app.workspace.order.push(crate::tab_bar::Id::Structure(id));
    crate::desktop_state::structure_opened(app, id);
    Task::batch([preview(app, id), scene_task(app, id, true)])
}
fn apply_focus(entry: &mut Entry, target: [f64; 3], distance: f64) {
    entry.focus(target);
    for camera in &mut entry.scene.cameras {
        if camera.perspective {
            let (_, _, forward) = camera.basis();
            camera.position =
                std::array::from_fn(|axis| target[axis] - f64::from(forward[axis]) * distance);
        } else {
            camera.zoom = (24. / distance as f32).clamp(
                crate::view_navigation::MIN_ZOOM,
                crate::view_navigation::MAX_ZOOM,
            );
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Ticket {
    id: u64,
    epoch: u64,
    assets_epoch: u64,
    selection: selection::ScriptTicket,
    cameras: [crate::spectator::Camera; 5],
    mode: crate::workspace_ui::View,
    axis: Axis,
    layer: String,
    slice: (f32, [f32; 2]),
}
pub(crate) fn ready(app: &App, id: u64) -> Result<bool, String> {
    let entry = app
        .structures
        .files
        .get(&id)
        .ok_or("启动脚本输入结构已关闭")?;
    if entry.busy || entry.scene.assets_busy || entry.scene.picking {
        return Ok(false);
    }
    if entry.document.is_none() {
        if entry.format == Format::Legacy && (entry.source_version().is_none() || entry.epoch == 0)
        {
            return Ok(false);
        }
        return Err(format!(
            "启动脚本输入结构未读取：{}；{}",
            entry.path.display(),
            entry.report
        ));
    }
    let Some((epoch, checked)) = &entry.recovery_checked else {
        return Ok(false);
    };
    if *epoch != entry.epoch {
        return Ok(false);
    }
    checked
        .as_ref()
        .map_err(|error| format!("启动结构的保存恢复状态读取失败：{error}"))?;
    if entry.recovery.is_some() || entry.engaged() {
        return Err("请先处理结构文档的任务、候选或保存恢复".into());
    }
    Ok(true)
}
pub(crate) fn input(app: &App, id: u64) -> Result<(Ticket, model::Input), String> {
    if !ready(app, id)? {
        return Err("启动脚本输入结构仍在读取或等待实际来源版本".into());
    }
    let entry = &app.structures.files[&id];
    capture_input(entry, id)
}
/// 连续插件允许读取本插件尚未确认的候选；其他任务仍由实际入口互斥。
pub(crate) fn plugin_input(app: &App, id: u64) -> Result<(Ticket, model::Input), String> {
    let entry = app.structures.files.get(&id).ok_or("插件主结构已关闭")?;
    if !entry.plugin_pointer_available() || entry.scene.assets_busy || entry.scene.picking {
        return Err("插件主结构仍有未完成的其他任务".into());
    }
    capture_input(entry, id)
}
fn capture_input(entry: &Entry, id: u64) -> Result<(Ticket, model::Input), String> {
    Ok((
        Ticket {
            id,
            epoch: entry.epoch,
            assets_epoch: entry.scene.assets_epoch,
            selection: selection::script_ticket(entry),
            cameras: entry.scene.cameras,
            mode: entry.scene.mode,
            axis: entry.axis,
            layer: entry.layer.clone(),
            slice: entry.camera.context(),
        },
        model::Input {
            document: entry.document.clone().expect("实际结构已读取"),
            selection: selection::script_region(entry)?,
        },
    ))
}
pub(crate) struct Publication {
    id: u64,
    epoch: u64,
    output: Option<Output>,
    selection: Option<selection::ScriptSelection>,
    focus: Option<([f64; 3], f64)>,
}
pub(crate) fn prepare_view(
    app: &App,
    ticket: &Ticket,
    document: &Document,
    selected: Option<Option<SelectionRegion>>,
    focus: Option<&script::Focus>,
) -> Result<Publication, String> {
    let entry = app
        .structures
        .files
        .get(&ticket.id)
        .ok_or("插件视图目标结构已关闭")?;
    let local = selected
        .map(|region| {
            region
                .map(|region| {
                    document
                        .script_local_region(region)
                        .map_err(|error| error.to_string())
                })
                .transpose()
        })
        .transpose()?;
    let selection = local
        .map(|region| selection::prepare_script(entry, ticket.id, &ticket.selection, region))
        .transpose()?
        .flatten();
    let follow = entry.scene.cameras == ticket.cameras
        && entry.scene.mode == ticket.mode
        && entry.axis == ticket.axis
        && entry.layer == ticket.layer
        && entry.camera.context() == ticket.slice;
    let focus = focus.filter(|_| follow).map(|focus| {
        let origin = document.content().source_min;
        (
            std::array::from_fn(|axis| focus.position[axis] - f64::from(origin[axis])),
            focus.distance,
        )
    });
    Ok(Publication {
        id: ticket.id,
        epoch: ticket.epoch,
        output: None,
        selection,
        focus,
    })
}
/// 数据来源检查供启动脚本和插件候选共用，不构造发布内容或改变文档。
pub(crate) fn matches(app: &App, ticket: &Ticket, baseline: &Document) -> bool {
    app.structures.files.get(&ticket.id).is_some_and(|entry| {
        entry.epoch == ticket.epoch
            && entry.scene.assets_epoch == ticket.assets_epoch
            && !entry.busy
            && !entry.scene.assets_busy
            && !entry.scene.picking
            && !entry.engaged()
            && entry.recovery.is_none()
            && entry
                .document
                .as_ref()
                .is_some_and(|document| baseline.same_source(document))
    })
}
pub(crate) fn prepare(
    app: &App,
    ticket: &Ticket,
    result: &model::Output,
    selected: Option<Option<SelectionRegion>>,
    focus: Option<&script::Focus>,
    objects: Option<Vec<ObjectSummary>>,
) -> Result<Publication, String> {
    let entry = app
        .structures
        .files
        .get(&ticket.id)
        .ok_or("脚本执行期间结构文档已关闭")?;
    if !matches(app, ticket, &result.baseline) {
        return Err("脚本执行期间结构内容、解释来源或任务归属已变化，整组结果未发布".into());
    }
    let local_selection = selected
        .map(|region| {
            region
                .map(|region| {
                    result
                        .document
                        .script_local_region(region)
                        .map_err(|error| error.to_string())
                })
                .transpose()
        })
        .transpose()?;
    let selection = local_selection
        .map(|region| selection::prepare_script(entry, ticket.id, &ticket.selection, region))
        .transpose()?
        .flatten();
    let follow = super::active(app) == Some(ticket.id)
        && entry.scene.cameras == ticket.cameras
        && entry.scene.mode == ticket.mode
        && entry.axis == ticket.axis
        && entry.layer == ticket.layer
        && entry.camera.context() == ticket.slice;
    let focus = focus.filter(|_| follow).map(|focus| {
        let origin = result.document.content().source_min;
        (
            std::array::from_fn(|axis| focus.position[axis] - f64::from(origin[axis])),
            focus.distance,
        )
    });
    Ok(Publication {
        id: ticket.id,
        epoch: ticket.epoch,
        selection,
        focus,
        output: (!result.baseline.same_source(&result.document)).then(|| Output {
            document: result.document.clone(),
            object: None,
            report: "启动脚本整组完成；结构内容仅增加一次文件撤销，未请求保存时仍在内存中。".into(),
            clipboard: None,
            cut: false,
            objects: objects.expect("实际结构修改的对象摘要已在后台完成"),
            import: None,
            import_intent: None,
            history_jump: None,
            plugin: None,
            plugin_group: None,
            inventory: None,
            nbt: None,
            brush: None,
        }),
    })
}
pub(crate) fn publish(app: &mut App, publication: Publication) -> Task<Message> {
    let task = if let Some(output) = publication.output {
        super::update(
            app,
            publication.id,
            Action::Ready(publication.epoch, Ok(output)),
        )
    } else {
        Task::none()
    };
    let entry = app
        .structures
        .files
        .get_mut(&publication.id)
        .expect("已核对脚本目标文档");
    if let Some(selection) = publication.selection {
        selection::publish_script(entry, publication.id, selection);
    }
    if let Some((target, distance)) = publication.focus {
        apply_focus(entry, target, distance);
    }
    task
}
pub(crate) fn begin_save(
    app: &mut App,
    id: u64,
    expected: &Document,
) -> Result<Option<(u64, Task<Message>)>, String> {
    if app.shortcuts.open || app.workspace.closing.is_some() {
        return Err("请先处理当前对话框，再保存结构脚本修改".into());
    }
    let entry = app
        .structures
        .files
        .get(&id)
        .ok_or("脚本保存所属结构已关闭")?;
    if entry.busy
        || entry.engaged()
        || entry.recovery.is_some()
        || entry
            .document
            .as_ref()
            .is_none_or(|document| !expected.same_content_revision(document))
    {
        return Err("脚本保存开始前结构文档已变化，请在文件页处理当前内容".into());
    }
    if !expected.dirty() {
        return Ok(None);
    }
    let task = super::update(app, id, Action::Work(Work::Save));
    let epoch = app.structures.files[&id].epoch;
    Ok(Some((epoch, task)))
}
pub(crate) fn save_completed(app: &App, id: u64, epoch: u64) -> bool {
    app.structures
        .files
        .get(&id)
        .is_none_or(|entry| entry.epoch != epoch || !entry.busy)
}
pub(crate) fn finish_save(app: &App, id: u64, epoch: u64) -> Result<(), String> {
    let entry = app
        .structures
        .files
        .get(&id)
        .ok_or("脚本保存所属结构已关闭")?;
    if entry.epoch != epoch {
        return Err("结构保存任务归属已改变，请在文件页核对实际保存状态".into());
    }
    if entry.document.as_ref().is_none_or(Document::dirty) {
        return Err(format!("结构脚本保存未完成：{}", entry.report));
    }
    Ok(())
}
pub(crate) fn recovery_page(app: &mut App, id: u64) -> Task<Message> {
    let activate = super::activate(app, id);
    let page = super::update(app, id, Action::Page(Page::File));
    Task::batch([activate, page])
}
