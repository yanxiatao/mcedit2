//! 把离散方块朝向映射到游戏可表达的状态；不可表达的姿态明确报错。
use super::{invalid, Mapping};
use crate::{
    clipboard::{reorient, HorizontalTransform as H},
    sections::State,
    WorldError,
};
use std::collections::BTreeMap;
fn direction(value: &str) -> Option<[i32; 3]> {
    Some(match value {
        "up" => [0, 1, 0],
        "down" => [0, -1, 0],
        "east" => [1, 0, 0],
        "west" => [-1, 0, 0],
        "south" => [0, 0, 1],
        "north" => [0, 0, -1],
        _ => return None,
    })
}
fn name(value: [i32; 3]) -> Result<&'static str, WorldError> {
    match value {
        [0, 1, 0] => Ok("up"),
        [0, -1, 0] => Ok("down"),
        [1, 0, 0] => Ok("east"),
        [-1, 0, 0] => Ok("west"),
        [0, 0, 1] => Ok("south"),
        [0, 0, -1] => Ok("north"),
        _ => Err(invalid("离散旋转不是坐标轴置换")),
    }
}
fn map_name(value: &str, map: &Mapping) -> Result<String, WorldError> {
    let direction = direction(value).ok_or_else(|| invalid(format!("未知方块方向 {value}")))?;
    Ok(name(map.grid_direction(direction))?.into())
}
fn set(state: &mut State, key: &str, value: &str) {
    if let Some((_, v)) = state.properties.iter_mut().find(|(k, _)| k == key) {
        *v = value.into();
    } else {
        state.properties.push((key.into(), value.into()));
    }
}
fn get<'a>(state: &'a State, key: &str) -> Option<&'a str> {
    state
        .properties
        .iter()
        .find_map(|(k, v)| (k == key).then_some(v.as_str()))
}
fn octants(mask: u8, map: &Mapping) -> u8 {
    let mut out = 0;
    for i in 0..8 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let p = map.grid_direction([
            if i & 1 != 0 { 1 } else { -1 },
            if i & 4 != 0 { 1 } else { -1 },
            if i & 2 != 0 { 1 } else { -1 },
        ]);
        let target = usize::from(p[0] > 0) + 2 * usize::from(p[2] > 0) + 4 * usize::from(p[1] > 0);
        out |= 1 << target;
    }
    out
}
fn shaped(state: &State, map: &Mapping) -> Result<Option<State>, WorldError> {
    let Some(crate::fluid_occlusion::Occlusion::Octants(mask)) =
        crate::fluid_occlusion::vanilla(state)?
    else {
        return Ok(None);
    };
    let target = octants(mask, map);
    let mut candidates = Vec::new();
    if state.name.ends_with("_stairs") {
        for half in ["bottom", "top"] {
            for facing in ["north", "east", "south", "west"] {
                for shape in [
                    "straight",
                    "inner_left",
                    "inner_right",
                    "outer_left",
                    "outer_right",
                ] {
                    if get(state, "shape").is_none() && shape != "straight" {
                        continue;
                    }
                    let mut value = state.clone();
                    set(&mut value, "half", half);
                    set(&mut value, "facing", facing);
                    if get(state, "shape").is_some() {
                        set(&mut value, "shape", shape);
                    }
                    if matches!(crate::fluid_occlusion::vanilla(&value)?,Some(crate::fluid_occlusion::Occlusion::Octants(v)) if v==target)
                    {
                        candidates.push(value);
                    }
                }
            }
        }
        if let Some(facing) = get(state, "facing") {
            let preferred = map_name(facing, map)?;
            if candidates
                .iter()
                .any(|c| get(c, "facing") == Some(preferred.as_str()))
            {
                candidates.retain(|c| get(c, "facing") == Some(preferred.as_str()));
            }
        }
    } else {
        for kind in ["bottom", "top", "double"] {
            let mut value = state.clone();
            set(&mut value, "type", kind);
            if matches!(crate::fluid_occlusion::vanilla(&value)?,Some(crate::fluid_occlusion::Occlusion::Octants(v)) if v==target)
            {
                candidates.push(value);
            }
        }
    }
    if candidates.len() != 1 {
        return Err(invalid(format!(
            "{} 的目标半格形状不能唯一表达；可关闭方块朝向变换后仅变换布局",
            state.name
        )));
    }
    let mut result = candidates.remove(0);
    result.properties.sort_unstable();
    Ok(Some(result))
}
fn six_facing(id: &str) -> bool {
    matches!(
        id,
        "minecraft:dispenser"
            | "minecraft:dropper"
            | "minecraft:piston"
            | "minecraft:sticky_piston"
            | "minecraft:piston_head"
            | "minecraft:observer"
            | "minecraft:command_block"
            | "minecraft:repeating_command_block"
            | "minecraft:chain_command_block"
            | "minecraft:end_rod"
            | "minecraft:lightning_rod"
            | "minecraft:amethyst_cluster"
            | "minecraft:small_amethyst_bud"
            | "minecraft:medium_amethyst_bud"
            | "minecraft:large_amethyst_bud"
    ) || matches!(
        id.strip_prefix("minecraft:")
            .and_then(|p| p.strip_suffix("shulker_box")),
        Some("")
            | Some("white_")
            | Some("orange_")
            | Some("magenta_")
            | Some("light_blue_")
            | Some("yellow_")
            | Some("lime_")
            | Some("pink_")
            | Some("gray_")
            | Some("light_gray_")
            | Some("cyan_")
            | Some("purple_")
            | Some("blue_")
            | Some("brown_")
            | Some("green_")
            | Some("red_")
            | Some("black_")
    )
}
pub(super) fn state(state: &State, map: &Mapping) -> Result<State, WorldError> {
    if map.orientation_identity() || state.properties.is_empty() {
        return Ok(state.clone());
    }
    let up = map.grid_direction([0, 1, 0]);
    // 复用现有旧新版本水平规则，包含铁轨、双箱、门铰链及 16 方向符号。
    if up == [0, 1, 0] {
        for mirrored in [false, true] {
            for turns in 0..4 {
                let operation = [H::Identity, H::Rotate90, H::Rotate180, H::Rotate270][turns];
                let apply = |mut p: [i32; 3]| {
                    if mirrored {
                        p[0] = -p[0];
                    }
                    for _ in 0..turns {
                        p = [-p[2], p[1], p[0]];
                    }
                    p
                };
                if apply([1, 0, 0]) == map.grid_direction([1, 0, 0])
                    && apply([0, 0, 1]) == map.grid_direction([0, 0, 1])
                {
                    let source = if mirrored {
                        reorient(state, H::MirrorX)?
                    } else {
                        state.clone()
                    };
                    return reorient(&source, operation);
                }
            }
        }
    }
    if !state.name.starts_with("minecraft:") {
        return Err(invalid(format!(
            "{} 尚无垂直朝向规则；可选择保留方块朝向",
            state.name
        )));
    }
    if let Some(value) = shaped(state, map)? {
        return Ok(value);
    }
    let mut fields: BTreeMap<String, String> = state.properties.iter().cloned().collect();
    let directional = state
        .properties
        .iter()
        .filter(|(k, _)| direction(k).is_some())
        .collect::<Vec<_>>();
    for (key, _) in &directional {
        fields.insert(
            (*key).clone(),
            if matches!(get(state, key), Some("none" | "low" | "tall")) {
                "none"
            } else {
                "false"
            }
            .into(),
        );
    }
    for (key, value) in &state.properties {
        if direction(key).is_some() {
            let target = map_name(key, map)?;
            if !fields.contains_key(&target)
                && !matches!(
                    state.name.as_str(),
                    "minecraft:glow_lichen" | "minecraft:sculk_vein"
                )
            {
                if !matches!(value.as_str(), "false" | "none") {
                    return Err(invalid(format!("{} 不支持朝 {target} 的连接", state.name)));
                }
            } else {
                fields.insert(target, value.clone());
            }
            continue;
        }
        let next = match key.as_str() {
            "axis" if value == "none" => value.clone(),
            "axis" => {
                let axis = match value.as_str() {
                    "x" => [1, 0, 0],
                    "y" => [0, 1, 0],
                    "z" => [0, 0, 1],
                    _ => return Err(invalid("方块 axis 无效")),
                };
                let target = map.grid_direction(axis);
                ["x", "y", "z"][target.iter().position(|v| *v != 0).expect("轴置换非零")].into()
            }
            "facing" => {
                let next = map_name(value, map)?;
                if matches!(next.as_str(), "up" | "down")
                    && !six_facing(&state.name)
                    && !(state.name == "minecraft:hopper" && next == "down")
                {
                    return Err(invalid(format!("{} 的 facing 无法表达 {next}", state.name)));
                }
                next
            }
            "vertical_direction" => {
                let next = map_name(value, map)?;
                if !matches!(next.as_str(), "up" | "down") {
                    return Err(invalid("该方块只有垂直方向，不能横放"));
                }
                next
            }
            "orientation"
                if matches!(
                    state.name.as_str(),
                    "minecraft:jigsaw" | "minecraft:crafter"
                ) =>
            {
                let (a, b) = value
                    .split_once('_')
                    .ok_or_else(|| invalid("复合方块方向无效"))?;
                let result = format!("{}_{}", map_name(a, map)?, map_name(b, map)?);
                if !matches!(
                    result.as_str(),
                    "down_east"
                        | "down_north"
                        | "down_south"
                        | "down_west"
                        | "up_east"
                        | "up_north"
                        | "up_south"
                        | "up_west"
                        | "west_up"
                        | "east_up"
                        | "north_up"
                        | "south_up"
                ) {
                    return Err(invalid(format!("{} 不支持目标方向 {result}", state.name)));
                }
                result
            }
            "half" if matches!(value.as_str(), "lower" | "upper") => {
                if up[1] == 0 {
                    return Err(invalid("上下双格方块不能横放"));
                }
                if up[1] < 0 {
                    if value == "lower" { "upper" } else { "lower" }.into()
                } else {
                    value.clone()
                }
            }
            "hinge" if up == [0, -1, 0] => match value.as_str() {
                "left" => "right".into(),
                "right" => "left".into(),
                _ => return Err(invalid("门铰链无效")),
            },
            "rotation" | "shape" | "face" | "half" | "part" | "type" | "tilt" => {
                return Err(invalid(format!(
                    "{} 的 {key} 暂无可表达的垂直变换，请选择保留朝向",
                    state.name
                )))
            }
            "waterlogged" | "lit" | "powered" | "age" | "level" | "open" | "hinge" | "distance"
            | "persistent" | "snowy" | "occupied" | "extended" | "triggered" | "conditional"
            | "delay" | "locked" | "power" | "stage" | "moisture" | "hatch" | "eggs" | "bites"
            | "layers" | "pickles" | "candles" | "charges" | "honey_level" | "note"
            | "instrument" | "attached" | "disarmed" | "in_wall" | "short" | "unstable"
            | "drag" | "enabled" | "signal_fire" | "berries" | "bloom" | "sculk_sensor_phase"
            | "thickness" | "mode" | "variant" | "seamless" | "decayable" | "check_decay" => {
                value.clone()
            }
            _ => return Err(invalid(format!("{} 的属性 {key} 尚无旋转规则", state.name))),
        };
        fields.insert(key.clone(), next);
    }
    Ok(State {
        name: state.name.clone(),
        properties: fields.into_iter().collect(),
    })
}
