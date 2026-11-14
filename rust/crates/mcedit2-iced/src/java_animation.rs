//! 客户端动画元数据与静态上传规则；不创建游戏中不存在的帧。
use super::Frame;
use crate::textures::texture_animation::{Sequence, Step};
use serde_json::{Map, Value};

pub(super) struct Selection {
    pub frame: Option<Frame>,
    pub diagnostics: Vec<String>,
    pub animation: Option<Sequence>,
}

use mcedit2_world::client_json::integer;

fn field(object: &Map<String, Value>, name: &str, default: i32) -> Result<i32, String> {
    object
        .get(name)
        .map_or(Ok(default), integer)
        .map_err(|error| format!("动画 {name} 无效：{error}"))
}

pub(super) fn select(
    width: u32,
    height: u32,
    metadata: Option<&[u8]>,
) -> Result<Selection, String> {
    let Some(bytes) = metadata else {
        return Ok(Selection {
            frame: None,
            diagnostics: Vec::new(),
            animation: None,
        });
    };
    let document = mcedit2_world::client_json::resource_object(bytes)
        .map_err(|e| format!("动画声明无效：{e}"))?;
    let mut diagnostics = document.diagnostic().into_iter().collect::<Vec<_>>();
    let Some(value) = document.value.get("animation") else {
        return Ok(Selection {
            frame: None,
            diagnostics,
            animation: None,
        });
    };
    let animation = value.as_object().ok_or("animation 必须是对象")?;
    let dimension = |name: &str| -> Result<Option<u32>, String> {
        match field(animation, name, -1)? {
            -1 => Ok(None),
            n if n > 0 => Ok(Some(n as u32)),
            _ => Err(format!("动画 {name} 无效")),
        }
    };
    let (w, h) = match (dimension("width")?, dimension("height")?) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, height),
        (None, Some(h)) => (width, h),
        (None, None) => {
            let edge = width.min(height);
            (edge, edge)
        }
    };
    if !width.is_multiple_of(w) || !height.is_multiple_of(h) {
        return Err("动画帧不能整除 PNG 尺寸".into());
    }
    let count = (width / w) * (height / h);
    let duration = field(animation, "frametime", 1)?;
    if duration <= 0 {
        return Err("动画帧时间无效".into());
    }
    if let Some(value) = animation.get("interpolate") {
        if !matches!(value, Value::Bool(_) | Value::Number(_) | Value::String(_)) {
            return Err("动画 interpolate 必须是原始值".into());
        }
    }
    let interpolate = animation
        .get("interpolate")
        .is_some_and(|value| match value {
            Value::Bool(b) => *b,
            Value::String(s) => s.eq_ignore_ascii_case("true"),
            _ => false,
        });
    let mut frames = Vec::new();
    if let Some(entries) = animation.get("frames") {
        let entries = entries.as_array().ok_or("动画 frames 必须是数组")?;
        for (position, entry) in entries.iter().enumerate() {
            let mut time = duration;
            let index = match entry {
                Value::Null | Value::Array(_) => continue,
                Value::Object(object) => {
                    if object.contains_key("time") && field(object, "time", -1)? <= 0 {
                        return Err(format!("动画帧 {position} 时间无效"));
                    }
                    time = field(object, "time", duration)?;
                    let index = integer(
                        object
                            .get("index")
                            .ok_or_else(|| format!("动画帧 {position} 缺少 index"))?,
                    )?;
                    if index < 0 {
                        return Err(format!("动画帧 {position} 对象 index 不能为负数"));
                    }
                    index
                }
                _ => integer(entry).map_err(|e| format!("动画帧 {position}：{e}"))?,
            };
            frames.push((position, index, time));
        }
    }
    let mut steps = Vec::new();
    let (first, animated) = if frames.is_empty() {
        (0, count > 1)
    } else {
        let mut valid = Vec::with_capacity(frames.len());
        for (position, index, time) in frames {
            if index < 0 || index as u32 >= count {
                diagnostics.push(format!(
                    "动画帧列表第 {position} 项引用 {index}，实际只有 {count} 帧；客户端移除此项"
                ));
            } else {
                valid.push(index as u32);
                steps.push(Step {
                    index: index as u32,
                    duration: time as u32,
                });
            }
        }
        let used = valid
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let mut ranges = Vec::new();
        let mut start = 0;
        for index in used {
            if start < index {
                ranges.push(format!("{start}..={}", index - 1));
            }
            start = index + 1;
        }
        if start < count {
            ranges.push(format!("{start}..={}", count - 1));
        }
        if !ranges.is_empty() {
            diagnostics.push(format!("动画图片中未使用的帧：{}", ranges.join(", ")));
        }
        // SpriteContents 在有效序列只有零 / 一项时不创建 AnimatedTexture，静态上传从左上角开始。
        if valid.len() > 1 {
            (valid[0], true)
        } else {
            (0, false)
        }
    };
    Ok(Selection {
        frame: Some((
            first % (width / w) * w,
            first / (width / w) * h,
            w,
            h,
            animated,
        )),
        diagnostics,
        animation: animated.then_some(Sequence {
            width: w,
            height: h,
            columns: width / w,
            count,
            duration: duration as u32,
            steps,
            interpolate,
        }),
    })
}

#[cfg(test)]
pub(super) fn verify_reference() {
    let native: Value = serde_json::from_str(include_str!(
        "../../mcedit2-world/tests/fixtures/java-resource-1.21.1.json"
    ))
    .unwrap();
    for row in native["numbers"].as_array().unwrap() {
        let value: Value = serde_json::from_str(row["input"].as_str().unwrap()).unwrap();
        let actual = integer(&value);
        assert_eq!(actual.is_ok(), row["accepted"].as_bool().unwrap(), "{row}");
        if let Ok(actual) = actual {
            assert_eq!(i64::from(actual), row["value"].as_i64().unwrap(), "{row}");
        }
    }
    for row in native["animations"].as_array().unwrap() {
        let object =
            mcedit2_world::client_json::resource_object(row["input"].as_str().unwrap().as_bytes())
                .unwrap();
        let metadata = serde_json::to_vec(&serde_json::json!({"animation":object.value})).unwrap();
        let actual = select(4, 8, Some(&metadata));
        assert_eq!(actual.is_ok(), row["accepted"].as_bool().unwrap(), "{row}");
        if let Ok(actual) = actual {
            let (_, _, w, h, _) = actual.frame.unwrap();
            assert_eq!(u64::from(w), row["width"].as_u64().unwrap());
            assert_eq!(u64::from(h), row["height"].as_u64().unwrap());
            let physical = (4 / w) * (8 / h);
            let parsed = row["frames"].as_array().unwrap();
            let valid = parsed
                .iter()
                .filter_map(|step| {
                    let index = step[0].as_i64().unwrap();
                    (index >= 0 && index < i64::from(physical))
                        .then_some((index as u32, step[1].as_u64().unwrap() as u32))
                })
                .collect::<Vec<_>>();
            let animated = if parsed.is_empty() {
                physical > 1
            } else {
                valid.len() > 1
            };
            assert_eq!(actual.animation.is_some(), animated, "{row}");
            if let Some(sequence) = actual.animation {
                assert_eq!(
                    sequence.duration as u64,
                    row["frametime"].as_u64().unwrap(),
                    "{row}"
                );
                assert_eq!(
                    sequence.interpolate,
                    row["interpolate"].as_bool().unwrap(),
                    "{row}"
                );
                assert_eq!(
                    sequence
                        .steps
                        .iter()
                        .map(|step| (step.index, step.duration))
                        .collect::<Vec<_>>(),
                    valid,
                    "{row}"
                );
            }
        }
    }
}
