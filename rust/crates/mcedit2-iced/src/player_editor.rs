use crate::nbt_editor::Action;
use iced::{
    widget::{button, column, row, text, text_input},
    Element,
};
use mcedit2_world::nbt_document::{Document, Value};
use std::collections::BTreeMap;

pub(crate) fn view<'a>(
    doc: &'a Document,
    input: &'a BTreeMap<String, String>,
    prefix: &str,
) -> Element<'a, Action> {
    let mut out = column![text("玩家属性（使用当前真实玩家源）").size(18)].spacing(4);
    for (label, name) in [
        ("位置 X", "Pos/0"),
        ("位置 Y", "Pos/1"),
        ("位置 Z", "Pos/2"),
        ("朝向 yaw", "Rotation/0"),
        ("朝向 pitch", "Rotation/1"),
        ("生命", "Health"),
        ("旧版生命", "HealF"),
        ("伤害吸收", "AbsorptionAmount"),
        ("氧气", "Air"),
        ("死亡时间", "DeathTime"),
        ("跌落距离", "FallDistance"),
        ("着火时间", "Fire"),
        ("上次受伤时间", "HurtByTimestamp"),
        ("受伤时间", "HurtTime"),
        ("无敌 0/1", "Invulnerable"),
        ("在地面 0/1", "OnGround"),
        ("传送门冷却", "PortalCooldown"),
        ("分数", "Score"),
        ("选中快捷栏 0–8", "SelectedItemSlot"),
        ("睡眠 0/1", "Sleeping"),
        ("睡眠时间", "SleepTimer"),
        ("消耗度", "foodExhaustionLevel"),
        ("饱和度", "foodSaturationLevel"),
        ("饥饿计时", "foodTickTimer"),
        ("经验随机种子", "XpSeed"),
        ("总经验", "XpTotal"),
        ("出生点 X", "SpawnX"),
        ("出生点 Y", "SpawnY"),
        ("出生点 Z", "SpawnZ"),
        ("出生点维度", "SpawnDimension"),
        ("强制出生点 0/1", "SpawnForced"),
        ("饥饿", "foodLevel"),
        ("经验等级", "XpLevel"),
        ("经验进度", "XpP"),
        ("玩家模式", "playerGameType"),
        ("维度", "Dimension"),
    ] {
        let path = format!("{prefix}/{name}");
        if let Ok(value) = doc.inspect(&path) {
            if let Some(current) = crate::world_info::as_text(&value) {
                let shown = input.get(&path).cloned().unwrap_or(current);
                let a = path.clone();
                let b = path.clone();
                out = out.push(
                    row![
                        text(label).width(120),
                        text_input("值", &shown)
                            .on_input(move |s| Action::TypedInput(a.clone(), s)),
                        button("暂存").on_press(Action::TypedApply(b))
                    ]
                    .spacing(4),
                );
                if let Some(choices) = choice_row(name, &path, &value) {
                    out = out.push(choices);
                }
            }
        }
    }
    if let Ok(Value::Compound(abilities)) = doc.inspect(&format!("{prefix}/abilities")) {
        out = out.push(text("玩家能力"));
        for field in abilities {
            if let Some(current) = crate::world_info::as_text(&field.value) {
                let path = format!(
                    "{prefix}/abilities/{}",
                    field.name.replace('~', "~0").replace('/', "~1")
                );
                let shown = input.get(&path).cloned().unwrap_or(current);
                let change = path.clone();
                out = out.push(
                    row![
                        text(field.name.clone()),
                        text_input("值", &shown)
                            .on_input(move |value| Action::TypedInput(change.clone(), value)),
                        button("暂存").on_press(Action::TypedApply(path.clone()))
                    ]
                    .spacing(4),
                );
                if let Some(choices) =
                    choice_row(&format!("abilities/{}", field.name), &path, &field.value)
                {
                    out = out.push(choices);
                }
            }
        }
    }
    for field_name in ["attributes", "Attributes"] {
        let path = format!("{prefix}/{field_name}");
        if let Ok(Value::List(list)) = doc.inspect(&path) {
            out = out.push(text("玩家属性"));
            for (index, item) in list.items.iter().enumerate() {
                if let Value::Compound(fields) = item {
                    let name = fields
                        .iter()
                        .find(|f| f.name == "id" || f.name == "Name")
                        .and_then(|f| {
                            if let Value::String(v) = &f.value {
                                Some(v.as_str())
                            } else {
                                None
                            }
                        })
                        .unwrap_or("未知属性");
                    if let Some(base) = fields.iter().find(|f| f.name == "base" || f.name == "Base")
                    {
                        let key = format!("{path}/{index}/{}", base.name);
                        if let Some(current) = crate::world_info::as_text(&base.value) {
                            let value = input.get(&key).cloned().unwrap_or(current);
                            let change = key.clone();
                            out = out.push(
                                row![
                                    text(name.to_owned()),
                                    text_input("基础值", &value)
                                        .on_input(move |v| Action::TypedInput(change.clone(), v)),
                                    button("暂存基础值").on_press(Action::TypedApply(key)),
                                    button("修饰符/NBT").on_press(Action::Tree(
                                        crate::nbt_tree::Action::Select(
                                            format!("{path}/{index}"),
                                            10
                                        )
                                    ))
                                ]
                                .spacing(4),
                            );
                        }
                    }
                }
            }
        }
    }
    out.into()
}
fn choice_row<'a>(field: &str, path: &str, value: &Value) -> Option<Element<'a, Action>> {
    let current = match value {
        Value::Byte(value) => i32::from(*value),
        Value::Int(value) => *value,
        _ => return None,
    };
    let choices: &[(i32, &str)] = if boolean(field) {
        &[(0, "关闭"), (1, "开启")]
    } else {
        match field {
            "playerGameType" => &[(0, "生存"), (1, "创造"), (2, "冒险"), (3, "旁观")],
            "SelectedItemSlot" => &[
                (0, "槽 1"),
                (1, "槽 2"),
                (2, "槽 3"),
                (3, "槽 4"),
                (4, "槽 5"),
                (5, "槽 6"),
                (6, "槽 7"),
                (7, "槽 8"),
                (8, "槽 9"),
            ],
            _ => &[],
        }
    };
    if choices.is_empty() {
        return None;
    }
    let mut buttons = row![].spacing(4);
    for (value, label) in choices {
        buttons = buttons.push(
            button(*label)
                .on_press(Action::TypedSet(path.into(), value.to_string()))
                .style(if *value == current {
                    button::primary
                } else {
                    button::secondary
                }),
        );
    }
    Some(buttons.wrap().into())
}
fn boolean(field: &str) -> bool {
    matches!(
        field,
        "Invulnerable"
            | "OnGround"
            | "Sleeping"
            | "SpawnForced"
            | "abilities/flying"
            | "abilities/mayfly"
            | "abilities/instabuild"
            | "abilities/invulnerable"
            | "abilities/mayBuild"
    )
}

/// 快捷属性只约束已打开玩家根下的原版槽位、模式和布尔字段，不解释模组自定义字段。
pub(crate) fn validate_property(prefix: &str, path: &str, value: &Value) -> Result<(), String> {
    let Some(field) = path.strip_prefix(&format!("{prefix}/")) else {
        return Ok(());
    };
    let n = match value {
        Value::Byte(n) => Some(i32::from(*n)),
        Value::Int(n) => Some(*n),
        _ => None,
    };
    if boolean(field) && !matches!(n, Some(0 | 1)) {
        return Err("玩家开关必须为 0 或 1".into());
    }
    if field == "SelectedItemSlot" && !n.is_some_and(|n| (0..=8).contains(&n)) {
        return Err("选中快捷栏槽位必须为 0..8".into());
    }
    if field == "playerGameType" && !n.is_some_and(|n| (0..=3).contains(&n)) {
        return Err("玩家游戏模式必须为 0..3".into());
    }
    Ok(())
}

/// 保留发起时的世界相机位置 / 朝向 / 维度，异步打开文件后不改用别的镜头。
#[derive(Clone, Debug)]
pub(crate) struct CameraPose {
    position: Option<[f64; 3]>,
    rotation: [f32; 2],
    dimension: String,
}
/// 玩家属性页覆盖画布时仍记住最近的实际投影；切片 / 四视图不能采用旧自由相机。
pub(crate) fn remember_camera_view(app: &mut crate::App) {
    let Some(key) = app.current_key() else {
        app.player_camera_view = None;
        return;
    };
    if app
        .player_camera_view
        .is_some_and(|(owner, _)| owner != key)
    {
        app.player_camera_view = None;
    }
    if app.structures.active.is_some() || app.desktop.show_recent {
        return;
    }
    if !app.show_advanced || app.advanced_section == crate::AdvancedSection::Viewer {
        app.player_camera_view = Some((key, app.spectator && !app.show_top_down));
    } else if matches!(
        app.advanced_section,
        crate::AdvancedSection::Maps | crate::AdvancedSection::Quad
    ) {
        app.player_camera_view = Some((key, false));
    }
}
pub(crate) fn capture_camera(app: &crate::App, moving: bool) -> Result<CameraPose, String> {
    let (world, dimension) = app.current_key().ok_or("请先选择实际世界维度")?;
    if app.structures.active.is_some()
        || !app.spectator
        || app.show_top_down
        || app.player_camera_view != Some(((world, dimension), true))
    {
        return Err("请先切换到世界相机视图，再采用相机位置或朝向".into());
    }
    Ok(CameraPose {
        position: moving.then_some(app.spectator_camera.position),
        rotation: [app.spectator_camera.yaw, app.spectator_camera.pitch],
        dimension: app.worlds[world].dimensions[dimension].name.clone(),
    })
}
/// 文档打开后才解释 Dimension 的实际类型；三个字段同次发布且只产生一次撤销。
pub(crate) fn camera_operations(
    doc: &Document,
    prefix: &str,
    pose: &CameraPose,
) -> Result<Vec<mcedit2_world::nbt_document::Operation>, String> {
    use mcedit2_world::{nbt_document::Operation, player_catalog};
    let rotation = doc
        .inspect_field(prefix, "Rotation")
        .map_err(|e| e.to_string())?
        .ok_or("玩家缺少 Rotation")?;
    if !matches!(rotation, Value::List(ref list) if list.element_type == 5 && list.items.len() == 2 && list.items.iter().all(|v| matches!(v, Value::Float(_))))
    {
        return Err("玩家 Rotation 必须为两项 Float".into());
    }
    let mut operations = camera_pose(prefix, pose.position, pose.rotation)?;
    if pose.position.is_some() {
        // 位置 / 维度按当前文档读取，包含此前已经暂存的编辑。
        player_catalog::document_location(doc, prefix).map_err(|e| e.to_string())?;
        let template = doc
            .inspect_field(prefix, "Dimension")
            .map_err(|e| e.to_string())?
            .ok_or("玩家缺少 Dimension")?;
        operations.push(Operation::Replace {
            path: format!("{prefix}/Dimension"),
            value: player_catalog::dimension_value(&template, &pose.dimension)
                .map_err(|e| e.to_string())?,
        });
    }
    Ok(operations)
}

pub(crate) fn camera_pose(
    prefix: &str,
    position: Option<[f64; 3]>,
    rotation: [f32; 2],
) -> Result<Vec<mcedit2_world::nbt_document::Operation>, String> {
    use mcedit2_world::nbt_document::{List, Operation};
    if position.is_some_and(|position| position.into_iter().any(|value| !value.is_finite()))
        || rotation.into_iter().any(|value| !value.is_finite())
    {
        return Err("镜头位置和朝向必须为有限数".into());
    }
    let mut operations = Vec::new();
    if let Some(position) = position {
        operations.push(Operation::Replace {
            path: format!("{prefix}/Pos"),
            value: Value::List(List {
                element_type: 6,
                items: position.into_iter().map(Value::Double).collect(),
            }),
        });
    }
    operations.push(Operation::Replace {
        path: format!("{prefix}/Rotation"),
        value: Value::List(List {
            element_type: 5,
            items: rotation.into_iter().map(Value::Float).collect(),
        }),
    });
    Ok(operations)
}
