//! 原版历史面板的原生对应：当前文档的真实操作顺序与可取消的修订跳转。
use crate::{App, Message};
use iced::{
    widget::{button, column, row, text, tooltip},
    Element, Length, Task,
};
use mcedit2_world::history::Timeline;
use std::{collections::BTreeMap, sync::Arc};

const PAGE_SIZE: usize = 32;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Target {
    World(usize, usize),
    Maps(usize),
    Structure(u64),
    File(u64),
}
struct Page {
    revision: Arc<()>,
    index: usize,
}
#[derive(Default)]
pub(crate) struct State {
    pages: BTreeMap<Target, Page>,
    report: Option<(Option<Target>, Option<Arc<()>>, String)>,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Show,
    Tools(bool),
    Page(Target, Arc<()>, bool),
    Jump(Target, Arc<()>, usize),
    Clear(Target, Arc<()>),
}
struct Context {
    target: Target,
    label: String,
    timeline: Timeline,
    editable: bool,
    clearable: bool,
    dirty: bool,
    report: String,
    cancel: Option<Message>,
    progress: Option<(usize, usize)>,
}
fn context(app: &App) -> Option<Context> {
    if let Some(id) = crate::structure_workspace::active(app) {
        let entry = &app.structures.files[&id];
        let document = entry.document.as_ref()?;
        return Some(Context {
            target: Target::Structure(id),
            label: entry.path.display().to_string(),
            timeline: document.history(),
            editable: entry.timeline_editable(),
            clearable: entry.history_editable(),
            dirty: document.dirty(),
            report: entry.history_report().into(),
            cancel: entry
                .history_cancelable()
                .then_some(Message::StructureWorkspace(
                    id,
                    crate::structure_workspace::Action::Cancel,
                )),
            progress: None,
        });
    }
    if app.desktop.show_recent {
        return None;
    }
    if app.show_advanced && app.advanced_section == crate::AdvancedSection::MapsEditor {
        let world = app.current?;
        let timeline = app.map_editor.history(&app.worlds[world].root);
        return Some(Context {
            target: Target::Maps(world),
            label: format!("{} · 地图候选", app.worlds[world].root.display()),
            dirty: timeline.cursor > 0,
            timeline,
            editable: !app.readonly
                && app.map_editor.history_editable()
                && !app.save_worker
                && !app.save_planning
                && !app.mixed_busy
                && !app.nbt_group_busy,
            report: app.map_editor.history_report().into(),
            clearable: false,
            cancel: None,
            progress: None,
        });
    }
    if let Some(panel) = crate::file_ui::document(app) {
        let id = app.nbt_document_id();
        return Some(Context {
            target: Target::File(id),
            label: panel.source.path().display().to_string(),
            timeline: panel.history(),
            editable: panel.inventory_editable(),
            clearable: panel.inventory_editable(),
            dirty: panel.source.document.changed(),
            report: panel.history_report().into(),
            cancel: panel
                .history_running()
                .then_some(Message::Nbt(id, crate::nbt_editor::Action::CancelHistory)),
            progress: None,
        });
    }
    let key = app.current_key()?;
    let session = app.sessions.get(&key)?;
    let task = crate::transfer_edit::history_task(app);
    Some(Context {
        target: Target::World(key.0, key.1),
        label: format!(
            "{} · {}",
            app.worlds[key.0].root.display(),
            session.dimension()
        ),
        timeline: session.history(),
        editable: crate::transfer_edit::history_editable(app),
        clearable: false,
        dirty: session.has_pending_changes(),
        report: app.edit_report.clone(),
        cancel: task.map(|(nonce, _)| {
            Message::Transfer(crate::transfer_edit::Action::CancelHistory(nonce))
        }),
        progress: task.map(|(_, progress)| (progress.examined, progress.selected)),
    })
}
fn page(app: &App, context: &Context) -> usize {
    app.history
        .pages
        .get(&context.target)
        .filter(|page| Arc::ptr_eq(&page.revision, &context.timeline.revision))
        .map_or_else(
            || context.timeline.cursor.saturating_sub(1) / PAGE_SIZE,
            |page| page.index,
        )
}
pub(crate) fn maintain(app: &mut App) {
    app.history.pages.retain(|target, _| match target {
        Target::World(world, _) | Target::Maps(world) => app.workspace.contains(*world),
        Target::Structure(id) => app.structures.files.contains_key(id),
        Target::File(id) => app.nbt_documents.files.contains_key(id),
    });
}
fn report(app: &mut App, message: impl Into<String>) -> Task<Message> {
    let message = message.into();
    log::error!("[history] {message}");
    let current = context(app);
    app.history.report = Some((
        current.as_ref().map(|current| current.target),
        current.map(|current| current.timeline.revision),
        message.clone(),
    ));
    app.edit_report = message.clone();
    app.messages.push(message);
    Task::none()
}
pub(crate) fn world_result(app: &mut App, key: (usize, usize), revision: Arc<()>, report: String) {
    app.history.report = Some((Some(Target::World(key.0, key.1)), Some(revision), report));
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if app.shortcuts.open
        || app.workspace.closing.is_some()
        || app
            .current
            .is_some_and(|world| app.nbt_documents.pending_close(world))
    {
        return Task::none();
    }
    match action {
        Action::Show => {
            app.history.report = None;
            crate::keyboard_bindings::interrupt(app);
            if let Some(id) = crate::structure_workspace::active(app) {
                return crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::History,
                    ),
                );
            }
            let Some(key) = app.current_key().filter(|_| !app.desktop.show_recent) else {
                return report(app, "请先打开一个文档");
            };
            if crate::file_ui::document(app).is_none() {
                if let Err(error) = app.current_session(key) {
                    return report(app, error);
                }
            }
            app.tools_pane = crate::workspace_ui::ToolsPane::History;
            app.workspace_pane = crate::workspace_ui::Pane::History;
            app.workspace.layout.show(if app.show_advanced {
                crate::workspace_ui::layout::Kind::Tools
            } else {
                crate::workspace_ui::layout::Kind::Editor
            });
            return crate::workspace_ui::scroll_start(
                if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                    crate::workspace_ui::TOOLS_NAV_SCROLL
                } else {
                    crate::workspace_ui::OPTIONS_SCROLL
                },
            );
        }
        Action::Tools(show) => {
            crate::keyboard_bindings::interrupt(app);
            app.tools_pane = if show {
                crate::workspace_ui::ToolsPane::History
            } else {
                crate::workspace_ui::ToolsPane::Navigation
            };
            if crate::workspace_ui::scene_workspace(app) {
                app.workspace_pane = if show {
                    crate::workspace_ui::Pane::History
                } else {
                    crate::workspace_ui::Pane::Edit
                };
                return crate::workspace_ui::scroll_start(crate::workspace_ui::OPTIONS_SCROLL);
            }
            return crate::workspace_ui::scroll_start(crate::workspace_ui::TOOLS_NAV_SCROLL);
        }
        Action::Page(target, revision, previous) => {
            let Some(current) = context(app).filter(|current| {
                current.target == target && Arc::ptr_eq(&current.timeline.revision, &revision)
            }) else {
                return Task::none();
            };
            let maximum = current
                .timeline
                .entries
                .len()
                .div_ceil(PAGE_SIZE)
                .saturating_sub(1);
            let index = if previous {
                page(app, &current).saturating_sub(1)
            } else {
                (page(app, &current) + 1).min(maximum)
            };
            app.history.pages.insert(target, Page { revision, index });
            return crate::workspace_ui::scroll_start(if matches!(target, Target::Structure(_)) {
                crate::structure_workspace::OPTIONS_SCROLL
            } else if crate::workspace_ui::scene_workspace(app) {
                crate::workspace_ui::OPTIONS_SCROLL
            } else if app.show_advanced && app.tools_pane == crate::workspace_ui::ToolsPane::History
            {
                crate::workspace_ui::TOOLS_NAV_SCROLL
            } else if app.show_advanced {
                crate::workspace_ui::TOOLS_SCROLL
            } else {
                crate::workspace_ui::OPTIONS_SCROLL
            });
        }
        Action::Clear(target, revision) => {
            let Some(current) = context(app).filter(|current| current.target == target) else {
                return report(app, "历史所属文档已切换");
            };
            if !current.clearable || !Arc::ptr_eq(&revision, &current.timeline.revision) {
                return report(app, "历史列表已改变或文档当前不能清理历史");
            }
            app.history.report = None;
            crate::keyboard_bindings::interrupt(app);
            return match target {
                Target::Structure(id) => crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Work(
                        crate::structure_workspace::Work::ClearHistory(revision),
                    ),
                ),
                Target::File(id) => crate::nbt_documents::edit(
                    app,
                    id,
                    crate::nbt_editor::Action::ClearHistory(revision),
                ),
                Target::World(..) | Target::Maps(_) => report(app, "当前文档不支持清空撤销记录"),
            };
        }
        Action::Jump(target, revision, index) => {
            let Some(current) = context(app).filter(|current| current.target == target) else {
                return report(app, "历史所属文档已切换");
            };
            if !current.editable || !Arc::ptr_eq(&revision, &current.timeline.revision) {
                return report(app, "历史列表已改变或文档当前不能编辑");
            }
            app.history.report = None;
            crate::keyboard_bindings::interrupt(app);
            return match target {
                Target::World(_, _) => crate::transfer_edit::update(
                    app,
                    crate::transfer_edit::Action::GotoHistory(revision, index),
                ),
                Target::Maps(_) => crate::map_editor::update(
                    app,
                    crate::map_editor::Action::History(revision, index),
                ),
                Target::Structure(id) => crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Work(
                        crate::structure_workspace::Work::History(revision, index),
                    ),
                ),
                Target::File(id) => crate::nbt_documents::edit(
                    app,
                    id,
                    crate::nbt_editor::Action::History(revision, index),
                ),
            };
        }
    }
    Task::none()
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let mut body = column![text(l.pick("历史", "History")).size(20)]
        .spacing(8)
        .width(Length::Fill);
    let current = context(app);
    if let Some((owner, revision, message)) = &app.history.report {
        if *owner == current.as_ref().map(|current| current.target)
            && revision.as_ref().is_none_or(|revision| {
                current
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(revision, &current.timeline.revision))
            })
        {
            body = body.push(text(message));
        }
    }
    let Some(current) = current else {
        return body
            .push(text(l.pick(
                "当前文档尚未读取。",
                "The current document is not loaded.",
            )))
            .into();
    };
    let timeline = &current.timeline;
    let target = current.target;
    let message =
        |index| Message::History(Action::Jump(target, Arc::clone(&timeline.revision), index));
    body = body
        .push(text(current.label.clone()))
        .push(text(format!(
            "当前修订 {} / {} · {}",
            timeline.cursor,
            timeline.entries.len(),
            if current.dirty {
                l.pick("有未保存修改", "Unsaved changes")
            } else {
                l.pick("内容与保存基线一致", "Matches saved content")
            }
        )))
        .push(
            row![
                button(l.pick("撤销", "Undo")).on_press_maybe(
                    (current.editable && timeline.cursor > 0).then(|| message(timeline.cursor - 1))
                ),
                button(l.pick("重做", "Redo")).on_press_maybe(
                    (current.editable && timeline.cursor < timeline.entries.len())
                        .then(|| message(timeline.cursor + 1))
                )
            ]
            .spacing(6),
        );
    if matches!(target, Target::Structure(_) | Target::File(_)) {
        body = body.push(
            button(l.pick("清空撤销记录", "Clear undo history")).on_press_maybe(
                (current.clearable && !timeline.entries.is_empty()).then(|| {
                    Message::History(Action::Clear(target, Arc::clone(&timeline.revision)))
                }),
            ),
        );
    }
    if let Some((done, total)) = current.progress {
        body = body.push(text(format!("{done} / {total}")));
    }
    if let Some(cancel) = &current.cancel {
        body = body
            .push(button(l.pick("取消当前任务", "Cancel current task")).on_press(cancel.clone()));
    }
    if !current.report.is_empty() {
        body = body.push(text(current.report.clone()));
    }
    let index = page(app, &current);
    let pages = timeline.entries.len().div_ceil(PAGE_SIZE).max(1);
    body = body.push(
        row![
            button("‹").on_press_maybe((index > 0).then(|| Message::History(Action::Page(
                target,
                Arc::clone(&timeline.revision),
                true
            )))),
            text(format!("{} / {}", index + 1, pages)),
            button("›").on_press_maybe((index + 1 < pages).then(|| Message::History(
                Action::Page(target, Arc::clone(&timeline.revision), false)
            )))
        ]
        .spacing(6),
    );
    body = body.push(
        button(l.pick("0 · 最早保留状态", "0 · Earliest retained state"))
            .style(if timeline.cursor == 0 {
                button::primary
            } else {
                button::secondary
            })
            .on_press_maybe((current.editable && timeline.cursor != 0).then(|| message(0)))
            .width(Length::Fill),
    );
    for (position, entry) in timeline
        .entries
        .iter()
        .enumerate()
        .skip(index * PAGE_SIZE)
        .take(PAGE_SIZE)
    {
        let cursor = position + 1;
        let label = format!(
            "{}{} · {}",
            if cursor == timeline.cursor {
                "● "
            } else if cursor > timeline.cursor {
                "↶ "
            } else {
                ""
            },
            cursor,
            entry.label
        );
        body = body.push(tooltip(
            button(text(label))
                .style(if cursor == timeline.cursor {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press_maybe(
                    (current.editable && cursor != timeline.cursor).then(|| message(cursor)),
                )
                .width(Length::Fill),
            text(entry.detail.clone()),
            tooltip::Position::Top,
        ));
    }
    body.into()
}
