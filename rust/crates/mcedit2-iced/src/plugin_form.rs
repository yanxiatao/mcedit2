//! 插件类型选项的原生表单；控件携带文档和命令表单身份，方块复用真实来源选择器。
use super::Panel;
use crate::{keyboard_bindings::Owner, App, Message};
use iced::{
    widget::{button, checkbox, column, pick_list, row, slider, text, text_input},
    Element, Length, Task,
};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Key {
    owner: Owner,
    generation: u64,
    index: usize,
}
pub(super) fn key(owner: Owner, generation: u64, index: usize) -> Key {
    Key {
        owner,
        generation,
        index,
    }
}
pub(super) fn focus_id(key: Key) -> iced::advanced::widget::Id {
    iced::advanced::widget::Id::from(format!(
        "plugin-option:{:?}:{}:{}",
        key.owner, key.generation, key.index
    ))
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Value(Key, String),
    Step(Key, bool),
    Slider(Key, f64),
    Selection(Owner, u64, String),
    Invoke(Key),
}
fn message(action: Action) -> Message {
    Message::Plugin(super::Action::Form(action))
}
fn spec_for(app: &App, key: Key) -> Result<&Value, String> {
    context(app, key.owner, key.generation)?;
    app.plugins
        .commands
        .iter()
        .find(|command| Some(&command.id) == app.plugins.command.as_ref())
        .and_then(|command| command.options.get(key.index))
        .ok_or_else(|| "插件选项已不存在".into())
}
fn context(app: &App, owner: Owner, generation: u64) -> Result<(), String> {
    if app.desktop.show_recent
        || matches!(owner, Owner::World(..)) && crate::file_ui::document(app).is_some()
        || crate::keyboard_bindings::owner(app) != Some(owner)
    {
        return Err("插件选项所属文档已切换，请使用当前文档的表单".into());
    }
    let panel = &app.plugins;
    if generation != panel.form_generation {
        return Err("插件命令或表单已更新，请使用当前选项".into());
    }
    Ok(())
}
fn name(spec: &Value) -> Result<&str, String> {
    spec.get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "插件选项缺少名称".into())
}
fn flag(spec: &Value, name: &str, default: bool) -> Result<bool, String> {
    spec.get(name).map_or(Ok(default), |value| {
        value
            .as_bool()
            .ok_or_else(|| format!("插件选项的 {name} 必须为布尔值"))
    })
}
fn editable(spec: &Value) -> Result<bool, String> {
    Ok(flag(spec, "enabled", true)? && flag(spec, "visible", true)?)
}
pub(crate) fn picker_value(app: &App, key: Key) -> Result<&str, String> {
    let spec = spec_for(app, key)?;
    if !editable(spec)? {
        return Err("当前插件选项不可编辑".into());
    }
    if spec.get("type").and_then(Value::as_str) != Some("blocktype") {
        return Err("当前插件选项不是方块类型".into());
    }
    app.plugins
        .values
        .get(name(spec)?)
        .map(String::as_str)
        .ok_or_else(|| "插件方块选项缺少当前值".into())
}
pub(super) fn update(app: &mut App, action: Action) -> Result<Task<Message>, String> {
    if let Action::Invoke(key) = &action {
        let spec = spec_for(app, *key)?;
        if !editable(spec)? || spec.get("type").and_then(Value::as_str) != Some("action") {
            return Err("当前表单动作已不可执行".into());
        }
        let name = name(spec)?.to_owned();
        return Ok(super::update(app, super::Action::Invoke(name)));
    }
    if let Action::Selection(owner, generation, value) = action {
        context(app, owner, generation)?;
        if !matches!(owner, Owner::World(..)) {
            return Err("独立结构选区应在本文件选区面板调整".into());
        }
        return Ok(crate::update_inner(app, Message::SelectionText(value)));
    }
    let key = match &action {
        Action::Value(key, _) | Action::Step(key, _) | Action::Slider(key, _) => *key,
        Action::Selection(..) => unreachable!("选区事件已按文档分派"),
        Action::Invoke(..) => unreachable!("按钮动作已按文档分派"),
    };
    let spec = spec_for(app, key)?;
    if !editable(spec)? {
        return Err("当前插件选项不可编辑".into());
    }
    let name = name(spec)?.to_owned();
    let value = match action {
        Action::Value(_, value) => value,
        Action::Selection(..) => unreachable!("选区事件已按文档分派"),
        Action::Invoke(..) => unreachable!("按钮动作已按文档分派"),
        Action::Slider(_, fraction) => slider_value(spec, fraction)?,
        Action::Step(_, increase) => {
            let raw = app
                .plugins
                .values
                .get(&name)
                .ok_or("插件数值选项缺少当前值")?;
            let steps = numeric_steps(spec, raw)?;
            let Some(value) = steps[usize::from(increase)].clone() else {
                return Ok(Task::none());
            };
            value
        }
    };
    Ok(super::update(app, super::Action::Value(name, value)))
}

#[derive(Debug, Clone, PartialEq)]
struct Choice {
    label: String,
    value: Value,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label)
    }
}
fn choices(spec: &Value) -> Result<Vec<Choice>, String> {
    let entries = spec
        .get("choices")
        .and_then(Value::as_array)
        .filter(|entries| !entries.is_empty())
        .ok_or("插件选择项缺少候选值")?;
    entries
        .iter()
        .map(|entry| {
            if let Some(pair) = entry.as_array() {
                if pair.len() != 2 {
                    return Err("插件候选值应包含显示名称与实际值".into());
                }
                Ok(Choice {
                    label: pair[0]
                        .as_str()
                        .ok_or("插件候选值的显示名称必须是文本")?
                        .to_owned(),
                    value: pair[1].clone(),
                })
            } else {
                Ok(Choice {
                    label: entry
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| entry.to_string()),
                    value: entry.clone(),
                })
            }
        })
        .collect()
}
fn integer(spec: &Value, key: &str, default: i64) -> Result<i64, String> {
    match spec.get(key) {
        Some(value) => value
            .as_i64()
            .ok_or_else(|| format!("整数选项的 {key} 必须是 i64 整数")),
        None => Ok(default),
    }
}
fn decimal(spec: &Value, key: &str, default: f64) -> Result<f64, String> {
    match spec.get(key) {
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .ok_or_else(|| format!("数值选项的 {key} 必须是有限数字")),
        None => Ok(default),
    }
}
fn float_increment(spec: &Value) -> Result<f64, String> {
    decimal(
        spec,
        "increment",
        if spec.get("min").is_some() && spec.get("max").is_some() {
            0.1
        } else {
            1.
        },
    )
}
/// 步进按选项实际边界到达端点；手工输入的草稿不自动改写。
fn numeric_steps(spec: &Value, raw: &str) -> Result<[Option<String>; 2], String> {
    match spec.get("type").and_then(Value::as_str) {
        Some("int") => {
            let current = raw
                .parse::<i64>()
                .map_err(|error| format!("请输入整数：{error}"))?;
            let low = integer(spec, "min", i64::MIN)?;
            let high = integer(spec, "max", i64::MAX)?;
            let increment = integer(spec, "increment", 1)?;
            if low > high || increment <= 0 {
                return Err("整数选项的范围无效或步长不为正数".into());
            }
            if !(low..=high).contains(&current) {
                return Err(format!("当前整数应在 {low}..{high} 之间"));
            }
            Ok([-1i128, 1].map(|sign| {
                let next = (i128::from(current) + sign * i128::from(increment))
                    .clamp(i128::from(low), i128::from(high));
                (next != i128::from(current)).then(|| next.to_string())
            }))
        }
        Some("float") => {
            let current = raw
                .parse::<f64>()
                .map_err(|error| format!("请输入数字：{error}"))?;
            let low = decimal(spec, "min", f64::NEG_INFINITY)?;
            let high = decimal(spec, "max", f64::INFINITY)?;
            let increment = float_increment(spec)?;
            if low > high || increment <= 0. {
                return Err("数值选项的范围无效或步长不为正数".into());
            }
            if !current.is_finite() || !(low..=high).contains(&current) {
                return Err(format!("当前值应为 {low}..{high} 之间的有限数字"));
            }
            let step = |sign: f64| {
                let next = (current + sign * increment).clamp(low, high);
                if !next.is_finite() {
                    return Err("数值步进超出有限浮点范围".to_owned());
                }
                Ok((next != current).then(|| next.to_string()))
            };
            Ok([step(-1.)?, step(1.)?])
        }
        _ => Err("只有数值选项可以步进".into()),
    }
}

/// 滑条只表示已声明的有限范围；整数结果由原整数起点和步长生成。
fn slider_state(spec: &Value, raw: &str) -> Result<Option<(f64, f64)>, String> {
    if !flag(spec, "slider", true)? {
        return Ok(None);
    }
    if spec.get("min").is_none() || spec.get("max").is_none() {
        return Ok(None);
    }
    match spec.get("type").and_then(Value::as_str) {
        Some("int") => {
            let low = integer(spec, "min", i64::MIN)?;
            let high = integer(spec, "max", i64::MAX)?;
            let current = raw.parse::<i64>().map_err(|error| error.to_string())?;
            let span = i128::from(high) - i128::from(low);
            if span == 0 {
                return Ok(None);
            }
            let increment = integer(spec, "increment", 1)?;
            Ok(Some((
                (i128::from(current) - i128::from(low)) as f64 / span as f64,
                (increment as f64 / span as f64).min(1.),
            )))
        }
        Some("float") => {
            let low = decimal(spec, "min", 0.)?;
            let high = decimal(spec, "max", 0.)?;
            let current = raw.parse::<f64>().map_err(|error| error.to_string())?;
            let span = high - low;
            if span == 0. {
                return Ok(None);
            }
            if !span.is_finite() {
                return Err("滑条范围跨度超出有限浮点范围；可继续输入或步进".into());
            }
            let step = (float_increment(spec)? / span).min(1.);
            if step == 0. {
                return Err("滑条步长相对范围过小；可继续输入或步进".into());
            }
            Ok(Some(((current - low) / span, step)))
        }
        _ => Err("只有数值选项可以使用滑条".into()),
    }
}
fn slider_value(spec: &Value, fraction: f64) -> Result<String, String> {
    if !fraction.is_finite() || !(0. ..=1.).contains(&fraction) {
        return Err("数值滑条位置必须在声明范围内".into());
    }
    match spec.get("type").and_then(Value::as_str) {
        Some("int") => {
            let low = i128::from(integer(spec, "min", i64::MIN)?);
            let high = i128::from(integer(spec, "max", i64::MAX)?);
            let increment = i128::from(integer(spec, "increment", 1)?);
            if low > high || increment <= 0 {
                return Err("整数滑条范围或步长无效".into());
            }
            if fraction == 1. {
                return Ok(high.to_string());
            }
            let offset = ((high - low) as f64 * fraction).round() as i128;
            let index = (offset + increment / 2) / increment;
            Ok((low + index * increment).clamp(low, high).to_string())
        }
        Some("float") => {
            let low = decimal(spec, "min", 0.)?;
            let high = decimal(spec, "max", 0.)?;
            let increment = float_increment(spec)?;
            let span = high - low;
            if low > high || !span.is_finite() || increment <= 0. {
                return Err("数值滑条范围或步长无效".into());
            }
            if fraction == 1. {
                return Ok(high.to_string());
            }
            let offset = span * fraction;
            let remainder = offset % increment;
            let lower = offset - remainder;
            let rounded = if remainder >= increment / 2. {
                lower + increment
            } else {
                lower
            };
            let value = if rounded >= span { high } else { low + rounded }.clamp(low, high);
            if !value.is_finite() {
                return Err("数值滑条结果超出有限浮点范围".into());
            }
            Ok(value.to_string())
        }
        _ => Err("只有数值选项可以使用滑条".into()),
    }
}

fn field<'a>(panel: &'a Panel, spec: &'a Value, key: Key) -> Result<Element<'a, Message>, String> {
    let enabled = editable(spec)?;
    let kind = spec
        .get("type")
        .and_then(Value::as_str)
        .ok_or("插件选项缺少类型")?;
    if kind == "label" {
        if let Some(space) = spec.get("stretch") {
            let stretch = space
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .ok_or("插件布局伸展无效")?;
            return decorate(
                iced::widget::Space::new()
                    .height(Length::FillPortion(stretch.max(1)))
                    .into(),
                spec,
            );
        }
        if let Some(space) = spec.get("space") {
            let space = space
                .as_f64()
                .filter(|value| value.is_finite() && *value >= 0. && *value <= f64::from(u16::MAX))
                .ok_or("插件布局间距无效")?;
            return decorate(iced::widget::Space::new().height(space as f32).into(), spec);
        }
        return decorate(
            text(
                spec.get("text")
                    .and_then(Value::as_str)
                    .ok_or("标签选项缺少文本")?,
            )
            .into(),
            spec,
        );
    }
    let name = name(spec)?;
    let label = spec.get("text").and_then(Value::as_str).unwrap_or(name);
    if kind == "action" {
        let state = super::presentation::action(spec)?;
        let content = panel
            .presentation
            .label(label.to_owned(), state.as_ref(), false, None)?;
        return decorate(
            button(content)
                .on_press_maybe(enabled.then_some(message(Action::Invoke(key))))
                .into(),
            spec,
        );
    }
    let raw = panel
        .values
        .get(name)
        .ok_or_else(|| format!("插件选项 {name} 缺少当前值"))?;
    let input = || {
        text_input(
            spec.get("placeholder")
                .and_then(Value::as_str)
                .unwrap_or(""),
            raw,
        )
        .id(focus_id(key))
        .on_input_maybe(enabled.then_some(move |value| message(Action::Value(key, value))))
        .width(Length::Fill)
    };
    let content: Element<'a, Message> = match kind {
        "bool" => checkbox(
            raw.parse::<bool>()
                .map_err(|error| format!("{label} 的布尔值无效：{error}"))?,
        )
        .label(label)
        .on_toggle_maybe(
            enabled.then_some(move |value: bool| message(Action::Value(key, value.to_string()))),
        )
        .into(),
        "text" => column![text(label), input()]
            .spacing(4)
            .width(Length::Fill)
            .into(),
        "blocktype" => column![
            text(label),
            row![
                input(),
                if enabled {
                    crate::block_picker::choose(crate::block_picker::Target::Plugin(key))
                } else {
                    button("选择方块")
                }
            ]
            .spacing(5)
            .wrap()
        ]
        .spacing(4)
        .width(Length::Fill)
        .into(),
        "choice" => {
            let options = choices(spec)?;
            let selected: Value = serde_json::from_str(raw)
                .map_err(|error| format!("{label} 的选择值无效：{error}"))?;
            let selected = options
                .iter()
                .find(|option| option.value == selected)
                .cloned()
                .ok_or_else(|| format!("{label} 的当前值不在候选值中"))?;
            let control: Element<'a, Message> = if enabled {
                pick_list(options, Some(selected), move |choice| {
                    message(Action::Value(key, choice.value.to_string()))
                })
                .width(Length::Fill)
                .into()
            } else {
                text(selected.label).into()
            };
            column![text(label), control]
                .spacing(4)
                .width(Length::Fill)
                .into()
        }
        "int" | "float" => {
            let steps = numeric_steps(spec, raw);
            let mut body = column![text(label)].spacing(4).width(Length::Fill);
            let step_row = |down: bool, up: bool| {
                row![
                    button("−").on_press_maybe(
                        (enabled && down).then_some(message(Action::Step(key, false)))
                    ),
                    input(),
                    button("+").on_press_maybe(
                        (enabled && up).then_some(message(Action::Step(key, true)))
                    )
                ]
                .spacing(5)
            };
            match steps {
                Ok([down, up]) => {
                    body = body.push(step_row(down.is_some(), up.is_some()));
                    match slider_state(spec, raw) {
                        Ok(Some((position, increment))) if enabled => {
                            body = body.push(
                                slider(0. ..=1., position, move |fraction| {
                                    message(Action::Slider(key, fraction))
                                })
                                .step(increment),
                            );
                        }
                        Ok(None) | Ok(Some(_)) => (),
                        Err(error) => body = body.push(text(error).size(12)),
                    }
                }
                Err(error) => body = body.push(step_row(false, false)).push(text(error).size(12)),
            }
            let mut limits = Vec::new();
            if let Some(low) = spec.get("min") {
                limits.push(format!("最小 {low}"));
            }
            if let Some(high) = spec.get("max") {
                limits.push(format!("最大 {high}"));
            }
            if let Some(increment) = spec.get("increment") {
                limits.push(format!("步长 {increment}"));
            }
            if !limits.is_empty() {
                body = body.push(text(limits.join(" · ")).size(12));
            }
            body.into()
        }
        _ => return Err(format!("插件选项类型尚未支持：{kind}")),
    };
    decorate(content, spec)
}
fn decorate<'a>(
    content: Element<'a, Message>,
    spec: &'a Value,
) -> Result<Element<'a, Message>, String> {
    let mut content = column![content].spacing(4).width(Length::Fill);
    if let Some(help) = spec.get("help") {
        content = content.push(text(help.as_str().ok_or("插件选项 help 必须为文本")?).size(12));
    }
    let margins = match spec.get("margins") {
        Some(value) => {
            let values = value
                .as_array()
                .filter(|values| values.len() == 4)
                .ok_or("插件选项 margins 必须包含四个数值")?;
            let mut margins = [0.; 4];
            for (index, value) in values.iter().enumerate() {
                margins[index] = value
                    .as_f64()
                    .filter(|value| {
                        value.is_finite() && *value >= 0. && *value <= f64::from(u16::MAX)
                    })
                    .ok_or("插件选项边距无效")? as f32;
            }
            margins
        }
        None => [0.; 4],
    };
    Ok(iced::widget::container(content)
        .padding(iced::Padding {
            left: margins[0],
            top: margins[1],
            right: margins[2],
            bottom: margins[3],
        })
        .width(Length::Fill)
        .into())
}
pub(super) fn view<'a>(panel: &'a Panel, owner: Owner) -> Element<'a, Message> {
    let mut body = column![].width(Length::Fill);
    let mut first = true;
    if let Some(command) = panel
        .commands
        .iter()
        .find(|command| Some(&command.id) == panel.command.as_ref())
    {
        for (index, spec) in command.options.iter().enumerate() {
            match flag(spec, "visible", true) {
                Ok(false) => continue,
                Ok(true) => (),
                Err(error) => {
                    body = body.push(text(error));
                    continue;
                }
            }
            if !first {
                match integer(spec, "spacing", 8).and_then(|value| {
                    u16::try_from(value).map_err(|_| "插件选项间距超出范围".into())
                }) {
                    Ok(spacing) => {
                        body = body.push(iced::widget::Space::new().height(f32::from(spacing)))
                    }
                    Err(error) => body = body.push(text(error)),
                }
            }
            first = false;
            let key = Key {
                owner,
                generation: panel.form_generation,
                index,
            };
            body = body.push(match field(panel, spec, key) {
                Ok(field) => field,
                Err(error) => text(format!("插件选项错误：{error}")).into(),
            });
        }
    }
    body.into()
}
