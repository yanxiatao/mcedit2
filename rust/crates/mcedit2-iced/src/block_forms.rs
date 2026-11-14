//! 命令方块 / 告示牌草稿；组件、过滤文字与多行编辑共用对象文档历史。
use iced::{
    widget::{button, checkbox, column, pick_list, row, text, text_editor, text_input},
    Element,
};
use mcedit2_world::{
    inspector_data::{Editor, Input, Kind, Mode, Snapshot},
    nbt_document::{Document, Operation},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Input(Arc<Snapshot>, String, String),
    Edit(Arc<Snapshot>, String, text_editor::Action),
    Mode(Arc<Snapshot>, String, Mode),
    Filtered(Arc<Snapshot>, String, bool),
    Remove(Arc<Snapshot>, String, bool),
    Apply(Arc<Snapshot>),
    Reset(Arc<Snapshot>),
    Face(Arc<Snapshot>, bool),
    Goto(Arc<Snapshot>, Option<usize>),
}
pub(crate) enum Command {
    None,
    Edit(Vec<Operation>),
    Goto(Option<usize>),
}
#[derive(Default)]
pub(crate) struct State {
    source: Option<Result<Arc<Snapshot>, String>>,
    inputs: BTreeMap<String, Input>,
    editors: BTreeMap<String, text_editor::Content>,
    modes: BTreeMap<String, Mode>,
    groups: BTreeMap<String, bool>,
    removed: BTreeSet<String>,
    back: bool,
    navigation: Mutex<
        Option<(
            String,
            [i32; 3],
            Option<String>,
            Result<Arc<mcedit2_world::command_visuals::Visuals>, String>,
        )>,
    >,
}
fn translation(
    bank: Option<&crate::textures::TextureBank>,
    language: crate::language::Language,
    key: &str,
) -> Option<String> {
    bank.and_then(|b| b.translation(key, language))
        .map(str::to_owned)
}
impl State {
    pub fn new(doc: &Document, version: Option<i32>) -> Self {
        let source = match Snapshot::capture(doc, version) {
            Ok(source) => source.map(|s| Ok(Arc::new(s))),
            Err(error) => {
                log::error!("[block-form] {error}");
                Some(Err(error.to_string()))
            }
        };
        let mut result = Self {
            source,
            ..Self::default()
        };
        if let Some(Ok(source)) = &result.source {
            for property in &source.properties {
                if property.editor == Editor::Command {
                    let mcedit2_world::nbt_document::Value::String(value) = &property.value else {
                        unreachable!()
                    };
                    result.editors.insert(
                        property.path.clone(),
                        text_editor::Content::with_text(value),
                    );
                }
            }
        }
        result
    }
    pub fn available(&self) -> bool {
        self.source.is_some()
    }
    pub(crate) fn is_command(&self) -> bool {
        matches!(&self.source,Some(Ok(source)) if source.kind==Kind::Command)
    }
    pub fn refresh(&mut self, doc: &Document, version: Option<i32>) {
        *self = Self::new(doc, version)
    }
    pub fn update(
        &mut self,
        doc: &Document,
        action: Action,
        editable: bool,
        bank: Option<&crate::textures::TextureBank>,
        language: crate::language::Language,
    ) -> Result<Command, String> {
        let source = Arc::clone(match &action {
            Action::Input(s, ..)
            | Action::Edit(s, ..)
            | Action::Mode(s, ..)
            | Action::Filtered(s, ..)
            | Action::Remove(s, ..)
            | Action::Apply(s)
            | Action::Reset(s)
            | Action::Face(s, ..)
            | Action::Goto(s, ..) => s,
        });
        if !source.current(doc) {
            return Err("表单文档已变化，请使用当前表单".into());
        }
        match action {
            Action::Input(source, path, value) => {
                if !source.properties.iter().any(|p| p.path == path) {
                    return Err("该字段不属于当前表单".into());
                }
                let mode = self.modes.get(&path).copied().unwrap_or_default();
                if let Some(editor) = self.editors.get_mut(&path) {
                    *editor = text_editor::Content::with_text(&value);
                }
                self.inputs.insert(path, Input { text: value, mode });
            }
            Action::Edit(source, path, action) => {
                let content = self.editors.get_mut(&path).ok_or("当前字段不是多行命令")?;
                content.perform(action);
                self.inputs.insert(
                    path,
                    Input {
                        text: content.text(),
                        mode: Mode::Plain,
                    },
                );
                if source.kind != Kind::Command {
                    return Err("当前对象不是命令方块".into());
                }
            }
            Action::Mode(source, path, mode) => {
                let property = source
                    .properties
                    .iter()
                    .find(|p| p.path == path && p.editor == Editor::Component)
                    .ok_or("该字段不是文字组件")?;
                if mode == self.modes.get(&path).copied().unwrap_or_default() {
                    return Ok(Command::None);
                }
                let mut preview = property.clone();
                if let Some(input) = self.inputs.get(&path) {
                    preview.value = property
                        .input_value(input, &|key| translation(bank, language, key))
                        .map_err(|e| e.to_string())?;
                }
                preview
                    .display(mode, &|key| translation(bank, language, key))
                    .map_err(|e| e.to_string())?;
                if self.inputs.contains_key(&path) {
                    let mcedit2_world::nbt_document::Value::String(encoded) = preview.value else {
                        unreachable!("组件字段必须为 String")
                    };
                    self.inputs.insert(
                        path.clone(),
                        Input {
                            text: encoded,
                            mode: Mode::Json,
                        },
                    );
                }
                self.modes.insert(path, mode);
            }
            Action::Filtered(source, path, enabled) => {
                if !source.filtered.contains_key(&path) {
                    return Err("当前对象没有该过滤文字组".into());
                }
                self.groups.insert(path, enabled);
            }
            Action::Remove(source, path, remove) => {
                if !source
                    .properties
                    .iter()
                    .any(|p| p.path == path && p.removable)
                {
                    return Err("该字段不能由表单删除".into());
                }
                if remove {
                    self.removed.insert(path);
                } else {
                    self.removed.remove(&path);
                }
            }
            Action::Face(_, back) => self.back = back,
            Action::Reset(_) => self.refresh(doc, Some(source.version)),
            Action::Apply(source) => {
                if !editable {
                    return Err("只读或正在处理任务，不能修改对象表单".into());
                }
                let operations = source
                    .edits(doc, &self.inputs, &self.groups, &self.removed, &|key| {
                        translation(bank, language, key)
                    })
                    .map_err(|e| e.to_string())?;
                return Ok(Command::Edit(operations));
            }
            Action::Goto(source, index) => {
                if source.kind != Kind::Command {
                    return Err("该对象不是命令方块".into());
                }
                return Ok(Command::Goto(index));
            }
        }
        Ok(Command::None)
    }
    pub fn navigation(
        &self,
        position: [i32; 3],
        dimension: Option<&str>,
    ) -> Result<Arc<mcedit2_world::command_visuals::Visuals>, String> {
        let source = self
            .source
            .as_ref()
            .ok_or("没有专用表单")?
            .as_ref()
            .map_err(Clone::clone)?;
        if source.kind != Kind::Command {
            return Err("该对象不是命令方块".into());
        }
        let command = if let Some(input) = self.inputs.get("/Command") {
            input.text.as_str()
        } else {
            let property = source
                .properties
                .iter()
                .find(|p| p.path == "/Command")
                .expect("原生命令字段");
            let mcedit2_world::nbt_document::Value::String(command) = &property.value else {
                unreachable!()
            };
            command
        };
        let mut cache = self.navigation.lock().expect("命令定位缓存锁被毒化");
        if let Some((_, _, _, result)) = cache.as_ref().filter(|(text, origin, dim, _)| {
            text == command && *origin == position && dim.as_deref() == dimension
        }) {
            return result.clone();
        }
        let result = mcedit2_world::command_visuals::analyze_context(
            command,
            position,
            dimension,
            Some(source.version),
        )
        .map(Arc::new);
        if let Err(error) = &result {
            log::error!("[command-targets] {error}");
        }
        *cache = Some((
            command.into(),
            position,
            dimension.map(str::to_owned),
            result.clone(),
        ));
        result
    }
}
pub(crate) fn view<'a>(
    state: &'a State,
    editable: bool,
    bank: Option<&crate::textures::TextureBank>,
    language: crate::language::Language,
    position: [i32; 3],
    dimension: Option<&str>,
) -> Element<'a, Action> {
    let source = match &state.source {
        Some(Ok(source)) => source,
        Some(Err(error)) => return text(format!("专用表单：{error}；可在 NBT 页修正。")).into(),
        None => return text("当前对象没有专用表单").into(),
    };
    let mut body = column![text(if source.kind == Kind::Command {
        "命令方块"
    } else {
        "告示牌"
    })
    .size(18)]
    .spacing(6);
    if source.kind == Kind::Sign && source.version != 1343 {
        body = body.push(
            row![
                button("正面")
                    .style(if state.back {
                        button::secondary
                    } else {
                        button::primary
                    })
                    .on_press(Action::Face(Arc::clone(source), false)),
                button("背面")
                    .style(if state.back {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press(Action::Face(Arc::clone(source), true)),
            ]
            .spacing(4),
        );
        let group = format!(
            "/{}/filtered_messages",
            if state.back {
                "back_text"
            } else {
                "front_text"
            }
        );
        let checked = state
            .groups
            .get(&group)
            .copied()
            .unwrap_or(source.filtered[&group]);
        let snapshot = Arc::clone(source);
        body = body.push(
            checkbox(checked)
                .label("保存独立的过滤后文字")
                .on_toggle(move |v| Action::Filtered(Arc::clone(&snapshot), group.clone(), v)),
        );
    }
    for property in &source.properties {
        if source.kind == Kind::Sign && source.version != 1343 {
            let face = if state.back {
                "/back_text/"
            } else {
                "/front_text/"
            };
            if property.path != "/is_waxed" && !property.path.starts_with(face) {
                continue;
            }
            if source.filtered.keys().any(|group| {
                property.path.starts_with(&(group.clone() + "/"))
                    && !state
                        .groups
                        .get(group)
                        .copied()
                        .unwrap_or(source.filtered[group])
            }) {
                continue;
            }
        }
        let path = property.path.clone();
        let snapshot = Arc::clone(source);
        let mode = state.modes.get(&path).copied().unwrap_or_default();
        let original = property.display(mode, &|key| translation(bank, language, key));
        let mut field = column![text(property.label.clone())].spacing(3);
        if property.editor == Editor::Component {
            let snapshot = Arc::clone(source);
            let key = path.clone();
            field = field.push(
                row![
                    button("文字")
                        .style(if mode == Mode::Plain {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .on_press(Action::Mode(
                            Arc::clone(&snapshot),
                            key.clone(),
                            Mode::Plain
                        )),
                    button("组件 JSON")
                        .style(if mode == Mode::Json {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .on_press(Action::Mode(snapshot, key, Mode::Json)),
                ]
                .spacing(3),
            );
        }
        let value = match state.inputs.get(&path) {
            Some(input) if property.editor == Editor::Component && input.mode != mode => property
                .input_value(input, &|key| translation(bank, language, key))
                .and_then(|value| {
                    let mut preview = property.clone();
                    preview.value = value;
                    preview.display(mode, &|key| translation(bank, language, key))
                }),
            Some(input) => Ok(input.text.clone()),
            None => original,
        };
        if property.editor == Editor::Command {
            field = field.push(text_editor(&state.editors[&path]).height(140).on_action(
                move |action| Action::Edit(Arc::clone(&snapshot), path.clone(), action),
            ));
        } else if let Ok(value) = value {
            if property.editor == Editor::Boolean {
                let checked = value != "0";
                field = field.push(checkbox(checked).label("启用").on_toggle(move |v| {
                    Action::Input(
                        Arc::clone(&snapshot),
                        path.clone(),
                        if v { "1" } else { "0" }.into(),
                    )
                }));
            } else if property.editor == Editor::Dye {
                field = field.push(pick_list(
                    mcedit2_world::banner::COLORS.map(str::to_owned),
                    Some(value),
                    move |value| Action::Input(Arc::clone(&snapshot), path.clone(), value),
                ));
            } else {
                field = field.push(text_input("", &value).on_input(move |value| {
                    Action::Input(Arc::clone(&snapshot), path.clone(), value)
                }));
            }
        } else if let Err(error) = value {
            field = field.push(text(format!(
                "文字无法静态展开：{error}；可编辑组件 JSON。"
            )))
        }
        if property.removable {
            let path = property.path.clone();
            let snapshot = Arc::clone(source);
            field = field.push(
                checkbox(state.removed.contains(&path))
                    .label(match property.path.as_str() {
                        "/CustomName" => "恢复默认名称",
                        "/LastOutput" => "清除上次输出",
                        "/LastExecution" => "清除执行时间记录",
                        _ => unreachable!("可清除的原生属性"),
                    })
                    .on_toggle(move |v| Action::Remove(Arc::clone(&snapshot), path.clone(), v)),
            );
        }
        body = body.push(field);
    }
    body = body.push(
        row![
            button("应用全部修改")
                .on_press_maybe(editable.then(|| Action::Apply(Arc::clone(source)))),
            button("取消表单修改").on_press(Action::Reset(Arc::clone(source))),
        ]
        .spacing(4),
    );
    if source.kind == Kind::Command {
        body = body.push(button("定位命令方块").on_press(Action::Goto(Arc::clone(source), None)));
        match state.navigation(position, dimension) {
            Ok(visuals) => {
                for (index, area) in visuals.boxes.iter().enumerate() {
                    let center: [f64; 3] =
                        std::array::from_fn(|i| (area.min[i] + area.max[i]) * 0.5);
                    body = body.push(
                        button(text(format!(
                            "定位目标 {} · {:.2}, {:.2}, {:.2}",
                            index + 1,
                            center[0],
                            center[1],
                            center[2]
                        )))
                        .on_press(Action::Goto(Arc::clone(source), Some(index))),
                    );
                }
                for diagnostic in &visuals.diagnostics {
                    body = body.push(text(diagnostic.clone()).size(12))
                }
            }
            Err(error) => body = body.push(text(format!("目标定位：{error}")).size(12)),
        }
        body = body.push(text("定位使用当前命令草稿。").size(12));
    } else {
        body = body.push(
            text("未修改的行保留组件样式；改写普通文字会重设该行样式。过滤后文字独立保存。")
                .size(12),
        );
    }
    body.into()
}
