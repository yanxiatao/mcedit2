//! 世界日期 / 时刻控件；沿原面板的 DayTime + 30000 显示日历，计算全程使用整数。
use crate::nbt_editor::Action as NbtAction;
use iced::{
    widget::{button, column, row, slider, text, text_input},
    Element,
};
use mcedit2_world::nbt_document::{Document, Operation, Value};
use std::collections::BTreeMap;

const SOURCE: &str = "@world-clock/source";
const DAY: &str = "@world-clock/day";
const TIME: &str = "@world-clock/time";
const TICK: &str = "@world-clock/tick";
#[derive(Debug, Clone)]
pub(crate) enum Edit {
    Day(String),
    Time(String),
    Tick(u16),
    Preset(u16),
    Apply,
    Reset,
}
#[derive(Debug, Clone)]
pub(crate) struct Action {
    source: i64,
    edit: Edit,
}
pub(crate) enum Command {
    Draft,
    Apply(Option<Operation>),
}
fn message(source: i64, edit: Edit) -> NbtAction {
    NbtAction::WorldClock(Action { source, edit })
}
fn read(doc: &Document) -> Result<i64, String> {
    match doc
        .inspect("/Data/DayTime")
        .map_err(|error| error.to_string())?
    {
        Value::Long(value) => Ok(value),
        _ => Err("日期与时刻要求 DayTime 为 Long".into()),
    }
}
fn split(source: i64) -> (i128, u16) {
    let calendar = i128::from(source) + 30000;
    (
        calendar.div_euclid(24000),
        calendar.rem_euclid(24000) as u16,
    )
}
fn format_time(tick: u16) -> String {
    format!(
        "{:02}:{:02}",
        tick / 1000,
        u32::from(tick % 1000) * 60 / 1000
    )
}
fn parse_time(value: &str) -> Result<u16, String> {
    let (hour, minute) = value.split_once(':').ok_or("时刻须为 HH:MM")?;
    if hour.len() != 2
        || minute.len() != 2
        || !hour
            .bytes()
            .chain(minute.bytes())
            .all(|value| value.is_ascii_digit())
    {
        return Err("时刻须为两位小时与分钟 HH:MM".into());
    }
    let hour = hour.parse::<u16>().map_err(|error| error.to_string())?;
    let minute = minute.parse::<u16>().map_err(|error| error.to_string())?;
    if hour >= 24 || minute >= 60 {
        return Err("时刻须在 00:00 到 23:59 之间".into());
    }
    // 对齐到所填分钟的首个游戏刻；向下截断会使部分分钟保存后显示成上一分钟。
    Ok(hour * 1000 + (minute * 1000 + 59) / 60)
}
fn active(input: &BTreeMap<String, String>, source: i64) -> bool {
    input
        .get(SOURCE)
        .is_some_and(|value| *value == source.to_string())
}
fn phase(input: &BTreeMap<String, String>, source: i64) -> Result<u16, String> {
    if active(input, source) {
        if let Some(time) = input.get(TIME) {
            return parse_time(time);
        }
        if let Some(tick) = input.get(TICK) {
            let tick = tick.parse::<u16>().map_err(|error| error.to_string())?;
            if tick >= 24000 {
                return Err("日内时刻必须小于 24000 刻".into());
            }
            return Ok(tick);
        }
    }
    Ok(split(source).1)
}
pub(crate) fn clear(input: &mut BTreeMap<String, String>) {
    for key in [SOURCE, DAY, TIME, TICK] {
        input.remove(key);
    }
}
pub(crate) fn update(
    doc: &Document,
    input: &mut BTreeMap<String, String>,
    action: Action,
) -> Result<Command, String> {
    let source = read(doc)?;
    if source != action.source {
        return Err("DayTime 在编辑日期 / 时刻期间已改变，请重新编辑".into());
    }
    if matches!(&action.edit, Edit::Reset) {
        clear(input);
        return Ok(Command::Draft);
    }
    if !active(input, source) {
        clear(input);
        input.insert(SOURCE.into(), source.to_string());
    }
    let apply = matches!(&action.edit, Edit::Preset(_) | Edit::Apply);
    match action.edit {
        Edit::Day(value) => {
            input.insert(DAY.into(), value);
            return Ok(Command::Draft);
        }
        Edit::Time(value) => {
            input.remove(TICK);
            input.insert(TIME.into(), value);
            return Ok(Command::Draft);
        }
        Edit::Tick(value) | Edit::Preset(value) => {
            if value >= 24000 {
                return Err("日内时刻必须小于 24000 刻".into());
            }
            input.remove(TIME);
            input.insert(TICK.into(), value.to_string());
            if !apply {
                return Ok(Command::Draft);
            }
        }
        Edit::Apply => (),
        Edit::Reset => unreachable!("重设已处理"),
    }
    let day = input
        .get(DAY)
        .map(|value| {
            value
                .parse::<i64>()
                .map(i128::from)
                .map_err(|error| format!("日期必须为整数：{error}"))
        })
        .transpose()?
        .unwrap_or(split(source).0);
    let value = day * 24000 + i128::from(phase(input, source)?) - 30000;
    let value = i64::try_from(value).map_err(|_| "日期与时刻超出 DayTime 的 Long 范围")?;
    Ok(Command::Apply((value != source).then(|| {
        Operation::Replace {
            path: "/Data/DayTime".into(),
            value: Value::Long(value),
        }
    })))
}
pub(crate) fn view<'a>(doc: &Document, input: &BTreeMap<String, String>) -> Element<'a, NbtAction> {
    let source = match read(doc) {
        Ok(source) => source,
        Err(error) => return text(error).into(),
    };
    let (day, _) = split(source);
    let day = if active(input, source) {
        input.get(DAY).cloned().unwrap_or_else(|| day.to_string())
    } else {
        day.to_string()
    };
    let time = if let Some(value) = input.get(TIME).filter(|_| active(input, source)) {
        value.clone()
    } else {
        match phase(input, source) {
            Ok(tick) => format_time(tick),
            Err(error) => {
                return column![
                    text(error),
                    button("重设草稿").on_press(message(source, Edit::Reset))
                ]
                .into()
            }
        }
    };
    let mut body = column![
        text("日期与日内时刻").size(17),
        row![
            text("日期"),
            text_input("整数日期", &day)
                .on_input(move |value| message(source, Edit::Day(value)))
                .on_submit(message(source, Edit::Apply)),
            text("时刻"),
            text_input("HH:MM", &time)
                .on_input(move |value| message(source, Edit::Time(value)))
                .on_submit(message(source, Edit::Apply)),
            button("暂存日期 / 时刻").on_press(message(source, Edit::Apply)),
            button("重设草稿").on_press(message(source, Edit::Reset)),
        ]
        .spacing(6),
    ]
    .spacing(5);
    body = match phase(input, source) {
        Ok(tick) => body.push(
            slider(0_u16..=23999, tick, move |value| {
                message(source, Edit::Tick(value))
            })
            .step(1_u16)
            .on_release(message(source, Edit::Apply)),
        ),
        Err(error) => body.push(text(error)),
    };
    body.push(
        row![
            button("日出").on_press(message(source, Edit::Preset(6000))),
            button("正午").on_press(message(source, Edit::Preset(12000))),
            button("日落").on_press(message(source, Edit::Preset(18000))),
            button("午夜").on_press(message(source, Edit::Preset(0))),
        ]
        .spacing(4),
    )
    .into()
}
