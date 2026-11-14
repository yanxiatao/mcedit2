//! 选区 / 维度多规则替换；世界复用候选场景，独立结构复用原生文档任务。
use crate::{brush_stroke::Rule, App, Message};
use iced::{
    widget::{button, checkbox, column, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::editing::{parse_state, replacement::Options, BrushReplacement};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    World(usize, usize),
    Structure(u64),
}
fn target(app: &App) -> Option<Target> {
    if let Some(id) = crate::structure_workspace::active(app) {
        Some(Target::Structure(id))
    } else if app.desktop.show_recent {
        None
    } else {
        app.current_key().map(|(w, d)| Target::World(w, d))
    }
}

pub(crate) struct State {
    rules: Vec<Rule>,
    revision: u64,
    in_selection: bool,
    remove_block_entities: bool,
    creation: crate::structure_edit::CreationPanel,
    report: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            revision: 0,
            in_selection: true,
            remove_block_entities: false,
            creation: Default::default(),
            report: String::new(),
        }
    }
}
impl State {
    pub(crate) fn picker_value(
        &self,
        revision: u64,
        index: usize,
        sources: bool,
    ) -> Result<&str, String> {
        if revision != self.revision {
            return Err("替换规则已改变，请重新选择方块".into());
        }
        let rule = self.rules.get(index).ok_or("替换规则不存在")?;
        Ok(if sources { &rule.sources } else { &rule.target })
    }
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Show,
    Add,
    Remove(u64, usize),
    Sources(u64, usize, String),
    Target(u64, usize, String),
    InSelection(bool),
    RemoveBlockEntities(bool),
    Creation(crate::structure_edit::Action),
    UsePicked(u64, usize, bool),
    Start(Target),
}
fn rules(state: &State) -> Result<Vec<BrushReplacement>, String> {
    if state.rules.is_empty() {
        return Err("请至少添加一条替换规则".into());
    }
    state
        .rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let sources = rule
                .sources
                .split('|')
                .map(|state| {
                    parse_state(state.trim())
                        .map_err(|error| format!("第 {} 条规则的源状态：{error}", index + 1))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let target = parse_state(&rule.target)
                .map_err(|error| format!("第 {} 条规则的目标：{error}", index + 1))?;
            Ok(BrushReplacement { sources, target })
        })
        .collect()
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
    match handle(app, action) {
        Ok(task) => task,
        Err(error) => {
            log::error!("[block-replacement] {error}");
            app.block_replacement.report = error;
            Task::none()
        }
    }
}
fn handle(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    if matches!(action, Action::Show) {
        crate::keyboard_bindings::interrupt(app);
        if let Some(id) = crate::structure_workspace::active(app) {
            return Ok(crate::structure_workspace::update(
                app,
                id,
                crate::structure_workspace::Action::Page(
                    crate::structure_workspace::Page::Replacement,
                ),
            ));
        }
        if app.desktop.show_recent || app.current_key().is_none() {
            return Err("请先打开世界或结构文档".into());
        }
        if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
            app.advanced_section = crate::AdvancedSection::Replacement;
        } else {
            app.editor_tool = crate::editor_options::Tool::Replacement;
            app.workspace_pane = crate::workspace_ui::Pane::Edit;
        }
        app.workspace.layout.show(if app.show_advanced {
            crate::workspace_ui::layout::Kind::Tools
        } else {
            crate::workspace_ui::layout::Kind::Editor
        });
        return Ok(crate::workspace_ui::scroll_start(
            if app.show_advanced && !crate::workspace_ui::scene_workspace(app) {
                crate::workspace_ui::DOCUMENT_SCROLL
            } else {
                crate::workspace_ui::OPTIONS_SCROLL
            },
        ));
    }
    if let Action::Start(owner) = action {
        if target(app) != Some(owner) {
            return Err("批量替换所属文档已切换，请在当前文档重新操作".into());
        }
        let rules = rules(&app.block_replacement)?;
        if let Some(id) = crate::structure_workspace::active(app) {
            let entry = &app.structures.files[&id];
            if !entry.history_editable() {
                return Err("当前结构文档不可编辑或正在处理任务".into());
            }
            let region = if app.block_replacement.in_selection {
                Some(entry.analysis_region()?)
            } else {
                None
            };
            let remove_block_entities = app.block_replacement.remove_block_entities;
            app.block_replacement.report.clear();
            return Ok(crate::structure_workspace::update(
                app,
                id,
                crate::structure_workspace::Action::Work(
                    crate::structure_workspace::Work::Replace {
                        rules,
                        region,
                        remove_block_entities,
                    },
                ),
            ));
        }
        if app.desktop.show_recent {
            return Err("请先选择实际世界或结构文档".into());
        }
        if app.block_edit.busy
            || app.save_worker
            || app.save_planning
            || app.mixed_busy
            || app.nbt_group_busy
        {
            return Err("请等待当前编辑或保存任务完成后再预检批量替换".into());
        }
        let key = app.current_key().ok_or("请先选择世界与维度")?;
        let region = if app.block_replacement.in_selection {
            Some(crate::selection_edit::region(app)?)
        } else {
            None
        };
        let creation = app.block_replacement.creation.profile(
            &app.worlds[key.0],
            &app.worlds[key.0].dimensions[key.1].name,
        )?;
        let remove_block_entities = app.block_replacement.remove_block_entities;
        crate::block_edit::start_replacement(
            app,
            Options {
                rules,
                region,
                creation,
                remove_block_entities,
            },
        )?;
        app.block_replacement.report.clear();
        return Ok(Task::none());
    }
    let row_revision = match &action {
        Action::Remove(revision, ..)
        | Action::Sources(revision, ..)
        | Action::Target(revision, ..)
        | Action::UsePicked(revision, ..) => Some(*revision),
        _ => None,
    };
    if row_revision.is_some_and(|revision| revision != app.block_replacement.revision) {
        return Ok(Task::none());
    }
    let picked = if matches!(&action, Action::UsePicked(..)) {
        let state = if let Some(id) = crate::structure_workspace::active(app) {
            app.structures.files[&id].picked_block_state()?
        } else {
            app.selected_block
                .as_ref()
                .filter(|selected| Some(selected.key) == app.current_key())
                .ok_or("请先在实际世界视图中点选一个方块")?
                .state
                .clone()
        };
        Some(mcedit2_world::editing::render_state(&state))
    } else {
        None
    };
    let source_field = matches!(&action, Action::Sources(..));
    // 只有本工具的在途候选随规则更改撤回，其他工具的未确认内容不受输入框影响。
    if app.block_edit.replacement {
        app.clear_block_preview();
    }
    let state = &mut app.block_replacement;
    match action {
        Action::Add => {
            state.rules.push(Rule {
                sources: String::new(),
                target: app.edit_target.clone(),
            });
            state.revision = state.revision.checked_add(1).expect("替换规则代次溢出");
        }
        Action::Remove(_, index) => {
            if index >= state.rules.len() {
                return Err("替换规则不存在".into());
            }
            state.rules.remove(index);
            state.revision = state.revision.checked_add(1).expect("替换规则代次溢出");
        }
        Action::Sources(_, index, value) | Action::Target(_, index, value) => {
            let rule = state.rules.get_mut(index).ok_or("替换规则不存在")?;
            if source_field {
                rule.sources = value;
            } else {
                rule.target = value;
            }
        }
        Action::UsePicked(_, index, source) => {
            let value = picked.expect("已读取点选状态");
            let rule = state.rules.get_mut(index).ok_or("替换规则不存在")?;
            if source {
                rule.sources = value;
            } else {
                rule.target = value;
            }
        }
        Action::InSelection(value) => state.in_selection = value,
        Action::RemoveBlockEntities(value) => state.remove_block_entities = value,
        Action::Creation(action) => state.creation.update(action),
        Action::Show | Action::Start(..) => unreachable!("导航和任务已处理"),
    }
    state.report.clear();
    Ok(Task::none())
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let state = &app.block_replacement;
    let revision = state.revision;
    let structure = crate::structure_workspace::active(app);
    let editable = structure.map_or(!app.readonly, |id| {
        app.structures.files[&id].history_editable()
    });
    let mut body = column![
        text("批量方块替换").size(20),
        text("每行可用 | 分隔多个完整源状态。规则从上到下覆盖同一源状态，每格只匹配修改前状态。"),
        checkbox(state.in_selection)
            .label("只替换当前选区；取消勾选则处理整个文档 / 维度")
            .on_toggle(|value| Message::BlockReplacement(Action::InSelection(value))),
        checkbox(state.remove_block_entities)
            .label("允许移除不兼容的方块实体及其库存")
            .on_toggle(|value| Message::BlockReplacement(Action::RemoveBlockEntities(value))),
        text("兼容载荷保持原样；普通实体和生物群系保持原内容。"),
        crate::block_edit::budget_view(app)
    ]
    .spacing(8)
    .width(Length::Fill);
    for (index, rule) in state.rules.iter().enumerate() {
        body = body.push(
            column![
                row![
                    text(format!("规则 {}", index + 1)),
                    button("删除")
                        .on_press(Message::BlockReplacement(Action::Remove(revision, index)))
                ]
                .spacing(5),
                text_input("完整源状态，可用 | 分隔", &rule.sources).on_input(move |value| {
                    Message::BlockReplacement(Action::Sources(revision, index, value))
                }),
                text_input("完整目标状态", &rule.target).on_input(move |value| {
                    Message::BlockReplacement(Action::Target(revision, index, value))
                }),
                row![
                    crate::block_picker::choose(crate::block_picker::Target::ReplacementSources(
                        revision, index
                    )),
                    text("→"),
                    crate::block_picker::choose(crate::block_picker::Target::ReplacementTarget(
                        revision, index
                    )),
                ]
                .spacing(5),
                row![
                    button("源取点选方块").on_press(Message::BlockReplacement(Action::UsePicked(
                        revision, index, true
                    ))),
                    button("目标取点选方块").on_press(Message::BlockReplacement(
                        Action::UsePicked(revision, index, false)
                    ))
                ]
                .spacing(5)
                .wrap()
            ]
            .spacing(4),
        );
    }
    body = body.push(button("添加规则").on_press(Message::BlockReplacement(Action::Add)));
    if structure.is_none() {
        body = body.push(
            state
                .creation
                .view_with_label("替换空气时允许补建现有区块的缺失区段")
                .map(|action| Message::BlockReplacement(Action::Creation(action))),
        );
    } else {
        body = body.push(text(
            "独立结构按实际局部格子替换；稀疏文件未声明的格子保持未声明。",
        ));
    }
    body = body.push(
        button(if structure.is_some() {
            "替换结构方块（一次撤销）"
        } else {
            "预检并预览批量替换"
        })
        .on_press_maybe(
            (editable
                && !app.block_edit.busy
                && !app.save_worker
                && !app.save_planning
                && !app.mixed_busy
                && !app.nbt_group_busy)
                .then(|| target(app).map(|target| Message::BlockReplacement(Action::Start(target))))
                .flatten(),
        ),
    );
    if !state.report.is_empty() {
        body = body.push(text(&state.report));
    }
    body.into()
}
