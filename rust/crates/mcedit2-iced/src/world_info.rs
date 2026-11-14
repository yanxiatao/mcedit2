use crate::nbt_editor::Action;
use iced::{
    widget::{button, column, row, text, text_input},
    Element,
};
use mcedit2_world::nbt_document::{Document, Value};
use std::collections::BTreeMap;
#[path = "world_clock.rs"]
pub(crate) mod clock;

pub(crate) fn view<'a>(
    doc: &'a Document,
    input: &'a BTreeMap<String, String>,
) -> Element<'a, Action> {
    let mut result =
        column![text("世界属性（更改暂存在内存，保存仍需候选确认）").size(18)].spacing(4);
    let mut fields = vec![
        ("名称", "/Data/LevelName"),
        ("出生 X", "/Data/SpawnX"),
        ("出生 Y", "/Data/SpawnY"),
        ("出生 Z", "/Data/SpawnZ"),
        ("世界时间", "/Data/Time"),
        ("日间时间", "/Data/DayTime"),
        ("难度 0–3", "/Data/Difficulty"),
        ("难度锁定 0/1", "/Data/DifficultyLocked"),
        ("游戏模式 0–3", "/Data/GameType"),
        ("极限模式 0/1", "/Data/hardcore"),
        ("允许命令 0/1", "/Data/allowCommands"),
        ("旧版种子", "/Data/RandomSeed"),
        ("旧版生成器", "/Data/generatorName"),
        ("生成器选项", "/Data/generatorOptions"),
        ("现代种子", "/Data/WorldGenSettings/seed"),
    ];
    for (label, path) in fields.drain(..) {
        if doc.inspect(path).is_ok() {
            result = result.push(field_row(doc, input, label, path.into()));
            let choices: &[(i32, &str)] = match path {
                "/Data/Difficulty" => &[(0, "和平"), (1, "简单"), (2, "普通"), (3, "困难")],
                "/Data/GameType" => &[(0, "生存"), (1, "创造"), (2, "冒险"), (3, "旁观")],
                "/Data/DifficultyLocked" | "/Data/hardcore" | "/Data/allowCommands" => {
                    &[(0, "关闭"), (1, "开启")]
                }
                _ => &[],
            };
            if !choices.is_empty() {
                let mut buttons = row![].spacing(4);
                for (value, name) in choices {
                    buttons = buttons.push(
                        button(*name).on_press(Action::TypedSet(path.into(), value.to_string())),
                    );
                }
                result = result.push(buttons);
            }
            if path == "/Data/generatorName" && matches!(doc.inspect(path), Ok(Value::String(_))) {
                result = result.push(
                    row![
                        button("默认").on_press(Action::TypedSet(path.into(), "default".into())),
                        button("平坦").on_press(Action::TypedSet(path.into(), "flat".into())),
                        button("巨型群系")
                            .on_press(Action::TypedSet(path.into(), "largeBiomes".into())),
                        button("放大化")
                            .on_press(Action::TypedSet(path.into(), "amplified".into())),
                        button("自定义")
                            .on_press(Action::TypedSet(path.into(), "customized".into())),
                        button("调试").on_press(Action::TypedSet(
                            path.into(),
                            "debug_all_block_states".into()
                        )),
                    ]
                    .spacing(4)
                    .wrap(),
                );
            }
        }
    }
    if doc.inspect("/Data/DayTime").is_ok() {
        result = result.push(clock::view(doc, input));
    }
    if let Ok(Value::Compound(rules)) = doc.inspect("/Data/GameRules") {
        result = result.push(text("游戏规则").size(18));
        for field in rules {
            let path = format!(
                "/Data/GameRules/{}",
                field.name.replace('~', "~0").replace('/', "~1")
            );
            result = result.push(field_row(doc, input, &field.name, path));
        }
    }
    result.into()
}
fn field_row<'a>(
    doc: &Document,
    input: &'a BTreeMap<String, String>,
    label: &str,
    path: String,
) -> Element<'a, Action> {
    let value = doc
        .inspect(&path)
        .ok()
        .and_then(|v| as_text(&v))
        .unwrap_or_default();
    let shown = input.get(&path).cloned().unwrap_or(value);
    let change = path.clone();
    let apply = path.clone();
    row![
        text(label.to_owned())
            .width(200)
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        text_input("值", &shown).on_input(move |v| Action::TypedInput(change.clone(), v)),
        button("暂存").on_press(Action::TypedApply(apply))
    ]
    .spacing(4)
    .into()
}
pub(crate) fn as_text(v: &Value) -> Option<String> {
    Some(match v {
        Value::String(v) => v.clone(),
        Value::Byte(v) => v.to_string(),
        Value::Short(v) => v.to_string(),
        Value::Int(v) => v.to_string(),
        Value::Long(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Double(v) => v.to_string(),
        _ => return None,
    })
}
pub(crate) fn validate(path: &str, value: &Value) -> Result<(), String> {
    let n = match value {
        Value::Byte(v) => Some(i64::from(*v)),
        Value::Int(v) => Some(i64::from(*v)),
        _ => None,
    };
    if [
        "/Data/DifficultyLocked",
        "/Data/hardcore",
        "/Data/allowCommands",
    ]
    .contains(&path)
        && !matches!(n, Some(0 | 1))
    {
        return Err("该开关必须为 0 或 1".into());
    }
    if ["/Data/Difficulty", "/Data/GameType"].contains(&path)
        && !n.is_some_and(|v| (0..=3).contains(&v))
    {
        return Err("该选项必须为 0 到 3".into());
    }
    if (path.ends_with("/count") || path.ends_with("/Count")) && !n.is_some_and(|v| v > 0) {
        return Err("物品数量必须为正数；清空槽位请使用删除".into());
    }
    Ok(())
}
pub(crate) fn parse_like(old: Value, text: &str) -> Result<Value, String> {
    mcedit2_world::nbt_query::parse_scalar_like(old, text)
}

/// 在当前日周期内选择时刻，保留 Long 精度与其他世界字段。
pub(crate) fn time_operation(
    doc: &Document,
    tick: u16,
) -> Result<mcedit2_world::nbt_document::Operation, String> {
    if tick >= 24000 {
        return Err("日内时刻必须小于 24000 刻".into());
    }
    let Value::Long(current) = doc
        .inspect("/Data/DayTime")
        .map_err(|error| error.to_string())?
    else {
        return Err("DayTime 必须为 Long".into());
    };
    let value = i128::from(current).div_euclid(24000) * 24000 + i128::from(tick);
    Ok(mcedit2_world::nbt_document::Operation::Replace {
        path: "/Data/DayTime".into(),
        value: Value::Long(i64::try_from(value).map_err(|_| "日间时间超出 Long 范围")?),
    })
}
