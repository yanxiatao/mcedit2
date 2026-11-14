//! 从同代实际方块读取鱼类的水接触；这是渲染上下文，不是新的存档标签。
use crate::world_data::RegistryLayer;
use crate::{
    entity_catalog::EntityVisual,
    fluid_surface::{Kind, Sample},
    nbt_document::Value,
    sections::State,
    WorldError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug)]
pub struct Tags {
    water: BTreeSet<String>,
}
impl Tags {
    /// 原生层之后按真实数据包顺序应用过滤、追加、replace 和嵌套标签。
    pub fn with_layers(version: i32, layers: &[RegistryLayer]) -> Result<Self, String> {
        let (registry, source) = match version {
            3465 => (
                "tags/fluids",
                include_str!("../assets/fluid-tags-1.20.1.json"),
            ),
            3955 => (
                "tags/fluid",
                include_str!("../assets/fluid-tags-1.21.1.json"),
            ),
            _ => return Err(format!("尚未核实 DataVersion={version} 的流体标签")),
        };
        let base: BTreeMap<String, serde_json::Value> =
            serde_json::from_str(source).map_err(|e| e.to_string())?;
        let mut visible: BTreeMap<String, Vec<Arc<[u8]>>> = base
            .into_iter()
            .map(|(key, value)| {
                serde_json::to_vec(&value)
                    .map(|bytes| (key, vec![Arc::from(bytes)]))
                    .map_err(|e| e.to_string())
            })
            .collect::<Result<_, _>>()?;
        for layer in layers {
            visible.retain(|id, _| {
                !layer
                    .filters
                    .iter()
                    .any(|filter| filter.blocks(id, registry))
            });
            for (id, bytes) in &layer.definitions {
                visible
                    .entry(id.clone())
                    .or_default()
                    .push(Arc::clone(bytes));
            }
        }
        Ok(Self {
            water: crate::registry_tags::resolve(
                "minecraft:water",
                true,
                &visible,
                &mut BTreeSet::new(),
                &mut BTreeMap::new(),
            )?,
        })
    }
}

const KEY: &str = "mcedit2:in_water";
pub fn supports(id: &str) -> bool {
    matches!(
        id,
        "minecraft:cod" | "minecraft:salmon" | "minecraft:pufferfish" | "minecraft:tropical_fish"
    )
}
/// 原生河豚的 renderer 不读取 isInWater，不为它准备无关的场景探针。
pub fn needs_contact(id: &str) -> bool {
    matches!(
        id,
        "minecraft:cod" | "minecraft:salmon" | "minecraft:tropical_fish"
    )
}
fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

fn fluid(state: &State) -> Result<(Option<Kind>, u8, &'static str), WorldError> {
    if !state.name.starts_with("minecraft:") {
        return Err(invalid(format!(
            "实体流体接触缺少方块 {} 的流体定义",
            state.name
        )));
    }
    Ok(match Sample::vanilla(state)? {
        Sample::Fluid(kind, level) => (
            Some(kind),
            level,
            match (kind, level == 0) {
                (Kind::Water, true) => "minecraft:water",
                (Kind::Water, false) => "minecraft:flowing_water",
                (Kind::Lava, true) => "minecraft:lava",
                (Kind::Lava, false) => "minecraft:flowing_lava",
            },
        ),
        _ => (None, 0, "minecraft:empty"),
    })
}

/// 两版原生实体的 AABB 向内 .001、floor(min)..ceil(max) 与流体表面比较。
/// 不使用渲染模型的体积，不把源未知格当作空气，也不推进流体或实体运动。
pub fn inspect(
    id: &str,
    visual: &EntityVisual,
    position: [f64; 3],
    tags: &Tags,
    query: &mut impl FnMut([i32; 3]) -> Result<State, WorldError>,
) -> Result<bool, WorldError> {
    if !supports(id) || !matches!(visual.source_version, Some(3465 | 3955)) {
        return Err(invalid(format!(
            "尚未核实 {id} 在 {:?} 的水接触",
            visual.source_version
        )));
    }
    let body = crate::entity_physics::read(id, visual).map_err(invalid)?;
    // EntityDimensions.makeBoundingBox 先以 float 计算半宽，再转换为 double。
    let half = f64::from(body.dimensions[0] / 2.0_f32);
    let minimum = [
        position[0] - half + 0.001,
        position[1] + 0.001,
        position[2] - half + 0.001,
    ];
    let maximum = [
        position[0] + half - 0.001,
        position[1] + f64::from(body.dimensions[1]) - 0.001,
        position[2] + half - 0.001,
    ];
    let coordinate = |value: f64| -> Result<i32, WorldError> {
        if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
            return Err(invalid("实体水接触范围超出可表示坐标"));
        }
        Ok(value as i32)
    };
    let mut start = [0; 3];
    let mut end = [0; 3];
    for i in 0..3 {
        start[i] = coordinate(minimum[i].floor())?;
        end[i] = coordinate(maximum[i].ceil())?;
    }
    let mut contact = false;
    for x in start[0]..end[0] {
        for y in start[1]..end[1] {
            for z in start[2]..end[2] {
                let (kind, level, id) = fluid(&query([x, y, z])?)?;
                if !tags.water.contains(id) {
                    continue;
                }
                let height = if let Some(kind) = kind {
                    let above = y
                        .checked_add(1)
                        .ok_or_else(|| invalid("流体上方坐标溢出"))?;
                    if fluid(&query([x, above, z])?)?.0 == Some(kind) {
                        1.0_f32
                    } else {
                        let amount = if level == 0 || level >= 8 {
                            8
                        } else {
                            8 - level
                        };
                        f32::from(amount) / 9.0_f32
                    }
                } else {
                    0.
                };
                // Entity 的 y + fluidHeight 先用 float 相加，再提升到 double。
                contact |= f64::from(y as f32 + height) >= minimum[1];
            }
        }
    }
    Ok(contact)
}

/// 仅写入不可变场景外观副本的运行时上下文，源 NBT / 撤销 / 保存载荷保持原样。
pub fn set(visual: &mut EntityVisual, in_water: bool) {
    visual
        .appearance
        .insert(KEY.into(), Value::Byte(if in_water { 1 } else { 0 }));
}
pub fn read(visual: &EntityVisual) -> Result<bool, String> {
    match visual.appearance.get(KEY) {
        Some(Value::Byte(0)) => Ok(false),
        Some(Value::Byte(1)) => Ok(true),
        _ => Err("鱼类外观缺少同代实际水接触上下文".into()),
    }
}
