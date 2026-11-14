//! 展示框物品的静态世界上下文；读取保存时间与实际维度规则，不驱动世界模拟。
use crate::{item_icons::field, java_random::LegacyRandom};
use mcedit2_world::{hanging, nbt_document::Value, world_data, WorldSummary};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone)]
pub(crate) struct Environment {
    pub data: Arc<world_data::Snapshot>,
    pub dimension: String,
    pub version: Option<i32>,
}
impl PartialEq for Environment {
    fn eq(&self, other: &Self) -> bool {
        self.data.origin == other.data.origin
            && self.dimension == other.dimension
            && self.version == other.version
    }
}
impl Environment {
    pub fn new(world: &WorldSummary, dimension: &str) -> Self {
        Self {
            data: Arc::clone(&world.world_data),
            dimension: dimension.into(),
            version: world.data_version,
        }
    }
}
/// 原版展示框视觉朝向。旧指南针不计物品旋转；现代垂直框还有轴方向修正。
pub(crate) fn frame_yaw(frame: &hanging::Frame, version: i32) -> f32 {
    let index = match frame.facing {
        0 | 1 => -1,
        2 => 2,
        3 => 0,
        4 => 1,
        5 => 3,
        _ => unreachable!("方向已校验"),
    };
    let extra = if version == 1343 {
        0
    } else {
        i32::from(frame.rotation) * 45
            + match frame.facing {
                0 => -90,
                1 => 90,
                _ => 0,
            }
    };
    let angle = (180 + index * 90 + extra).rem_euclid(360);
    (if angle >= 180 { angle - 360 } else { angle }) as f32
}
/// 原版已稳定的时钟目标；旧版使用 float 与 partialTick=1，现代采用 DimensionType 双精度运算。
pub(crate) fn time(day: i64, fixed: Option<i64>, version: i32) -> f32 {
    if version == 1343 {
        let mut value = ((day % 24000) as f32 + 1.) / 24000. - 0.25;
        if value < 0. {
            value += 1.;
        }
        if value > 1. {
            value -= 1.;
        }
        let smoothed = 1. - (((f64::from(value) * std::f64::consts::PI).cos() + 1.) / 2.) as f32;
        value + (smoothed - value) / 3.
    } else {
        let day = fixed.unwrap_or(day) as f64 / 24000. - 0.25;
        let fraction = day - day.floor();
        let eased = 0.5 - (fraction * std::f64::consts::PI).cos() / 2.;
        (fraction * 2. + eased) as f32 / 3.
    }
}
pub(crate) fn angle(position: [f64; 3], yaw: f32, target: [i32; 3], version: i32) -> f32 {
    let offset = if version == 1343 { 0. } else { 0.5 };
    let dx = f64::from(target[0]) + offset - position[0];
    let dz = f64::from(target[2]) + offset - position[2];
    let bearing = dz.atan2(dx) / f64::from(std::f32::consts::TAU);
    let rotation = if version == 1343 {
        f64::from(yaw) / 360.
    } else {
        f64::from(yaw / 360.)
    };
    // Java positiveModulo 在 cast 后执行；不能先在 double 上折回。
    let value = (0.5 - (rotation.rem_euclid(1.) - 0.25 - bearing)) as f32;
    (value % 1. + 1.) % 1.
}
fn text(value: &Value, name: &str) -> Result<String, String> {
    match field(Some(value), name) {
        Some(Value::String(s)) => Ok(if s.contains(':') {
            s.clone()
        } else {
            format!("minecraft:{s}")
        }),
        _ => Err(format!("指南针目标 {name} 必须为资源名称")),
    }
}
fn position(value: &Value, modern: bool) -> Result<[i32; 3], String> {
    if modern {
        match value {
            Value::IntArray(v) if v.len() == 3 => Ok([v[0], v[1], v[2]]),
            _ => Err("指南针目标 pos 必须为三个 Int 的数组".into()),
        }
    } else {
        let mut result = [0; 3];
        for (i, key) in ["X", "Y", "Z"].into_iter().enumerate() {
            result[i] = match field(Some(value), key) {
                Some(Value::Int(n)) => *n,
                _ => return Err(format!("磁石位置 {key} 必须为 Int")),
            };
        }
        Ok(result)
    }
}
enum Target {
    Spawn,
    Lodestone(Option<(String, [i32; 3])>),
    None,
}
fn target(id: &str, value: &Value, version: i32) -> Result<Target, String> {
    // RecoveryCompass 的原版提供器只读取 Player.lastDeathLocation；展示框不是玩家。
    if id == "minecraft:recovery_compass" {
        return Ok(Target::None);
    }
    if version == 1343 {
        return Ok(Target::Spawn);
    }
    if version == 3955 {
        let Some(tracker) = field(
            field(Some(value), "components"),
            "minecraft:lodestone_tracker",
        ) else {
            return Ok(Target::Spawn);
        };
        if !matches!(tracker, Value::Compound(_)) {
            return Err("磁石追踪组件必须为 Compound".into());
        }
        let Some(target) = field(Some(tracker), "target") else {
            return Ok(Target::Lodestone(None));
        };
        return Ok(Target::Lodestone(Some((
            text(target, "dimension")?,
            position(field(Some(target), "pos").ok_or("磁石目标缺少 pos")?, true)?,
        ))));
    }
    let tag = field(Some(value), "tag");
    let dim = field(tag, "LodestoneDimension");
    let pos = field(tag, "LodestonePos");
    match (dim, pos) {
        (None, None) => Ok(Target::Spawn),
        (Some(Value::String(dim)), Some(pos)) => Ok(Target::Lodestone(Some((
            if dim.contains(':') {
                dim.clone()
            } else {
                format!("minecraft:{dim}")
            },
            position(pos, false)?,
        )))),
        (None, _) | (_, None) => Ok(Target::Lodestone(None)),
        _ => Err("LodestoneDimension 必须为字符串".into()),
    }
}
/// 没有有效目标的原版属性是随机指针；静态编辑器在同一保存快照保持一份可复现采样。
/// 不将它误作指向出生点，也不声称播放原客户端的逐 tick 阻尼动画。
fn spin(environment: &Environment, position: [f64; 3]) -> Result<f32, String> {
    let mut seed = environment.data.saved_time.ok_or("随机指针缺少世界 Time")?;
    for n in position {
        seed = seed.rotate_left(17) ^ n.to_bits() as i64;
    }
    Ok(LegacyRandom::new(seed).double() as f32)
}
pub(crate) fn apply(
    id: &str,
    value: &Value,
    frame: &hanging::Frame,
    version: i32,
    environment: Option<&Environment>,
    world_position: [f64; 3],
    properties: &mut BTreeMap<String, f32>,
) -> Result<(), String> {
    apply_at(
        id,
        value,
        frame_yaw(frame, version),
        version,
        environment,
        world_position,
        properties,
        None,
    )
}
pub(crate) fn apply_dropped(
    id: &str,
    value: &Value,
    yaw: f32,
    version: i32,
    environment: Option<&Environment>,
    world_position: [f64; 3],
    properties: &mut BTreeMap<String, f32>,
) -> Result<(), String> {
    // 旧 ItemCompass 在没有持有者 / 展示框时直接返回 0；现代掉落物保留实体表示。
    if version == 1343 && id == "minecraft:compass" {
        properties.insert("angle".into(), 0.);
        return Ok(());
    }
    apply_at(
        id,
        value,
        yaw,
        version,
        environment,
        world_position,
        properties,
        None,
    )
}
/// 手持模型使用实际持有者；恢复指南针只解释已保存的玩家死亡位置。
pub(crate) fn apply_held(
    id: &str,
    value: &Value,
    yaw: f32,
    version: i32,
    environment: Option<&Environment>,
    world_position: [f64; 3],
    player: Option<&mcedit2_world::entity_catalog::EntityVisual>,
    properties: &mut BTreeMap<String, f32>,
) -> Result<(), String> {
    let override_target = if id == "minecraft:recovery_compass" {
        Some(
            match player.and_then(|p| p.appearance.get("LastDeathLocation")) {
                None => Target::None,
                Some(value) => Target::Lodestone(Some((
                    text(value, "dimension")?,
                    position(
                        field(Some(value), "pos").ok_or("玩家死亡位置缺少 pos")?,
                        true,
                    )?,
                ))),
            },
        )
    } else {
        None
    };
    apply_at(
        id,
        value,
        yaw,
        version,
        environment,
        world_position,
        properties,
        override_target,
    )
}
fn apply_at(
    id: &str,
    value: &Value,
    yaw: f32,
    version: i32,
    environment: Option<&Environment>,
    world_position: [f64; 3],
    properties: &mut BTreeMap<String, f32>,
    override_target: Option<Target>,
) -> Result<(), String> {
    let clock = id == "minecraft:clock";
    let compass =
        id == "minecraft:compass" || id == "minecraft:recovery_compass" && version != 1343;
    if !clock && !compass {
        return Ok(());
    }
    let environment = environment.ok_or("世界物品缺少实际维度上下文")?;
    if environment.version != Some(version) {
        return Err("物品与世界上下文版本不同".into());
    }
    if clock {
        let rules = environment
            .data
            .celestial(&environment.dimension, version)
            .map_err(|e| e.to_string())?;
        let value = if rules.natural {
            let day = rules
                .fixed_time
                .or(environment.data.day_time)
                .ok_or("时钟缺少保存的 DayTime")?;
            time(day, None, version)
        } else {
            spin(environment, world_position)?
        };
        properties.insert("time".into(), value);
    }
    if compass {
        let target = match match override_target {
            Some(target) => target,
            None => target(id, value, version)?,
        } {
            Target::None | Target::Lodestone(None) => None,
            Target::Lodestone(Some((dim, pos))) => (dim == environment.dimension).then_some(pos),
            Target::Spawn => {
                let rules = environment
                    .data
                    .celestial(&environment.dimension, version)
                    .map_err(|e| e.to_string())?;
                if rules.natural {
                    Some(environment.data.spawn.ok_or("指南针缺少世界出生点")?)
                } else {
                    None
                }
            }
        };
        let target = target.filter(|p| {
            version == 1343
                || (0..3)
                    .map(|i| (f64::from(p[i]) + 0.5 - world_position[i]).powi(2))
                    .sum::<f64>()
                    >= f64::from(0.00001_f32)
        });
        let value = match target {
            Some(target) => angle(world_position, yaw, target, version),
            None => spin(environment, world_position)?,
        };
        properties.insert("angle".into(), value);
    }
    Ok(())
}
