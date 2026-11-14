//! 精确源流体的显示高度；只保存四角结果，不保留被遮挡体素或修改源状态。
use crate::{sections::State, WorldError};
use std::num::NonZeroU32;

/// 西北、东北、西南、东南四角的正高度；非零位模式允许 Option 复用空值编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FluidSurface {
    heights: [NonZeroU32; 4],
    /// 只供显示 UV 使用的水平单位流向；有限位模式参与源缓存的精确比较。
    flow: [u32; 2],
    kind: Kind,
    faces: u8,
    overlays: u8,
}
impl FluidSurface {
    fn new(heights: [f32; 4], flow: [f32; 2], kind: Kind) -> Result<Self, WorldError> {
        if heights
            .iter()
            .any(|h| !h.is_finite() || *h <= 0.0 || *h > 1.0)
        {
            return Err(WorldError::Invalid("流体四角高度超出 (0, 1]".into()));
        }
        if flow.iter().any(|v| !v.is_finite() || v.abs() > 1.0) {
            return Err(WorldError::Invalid("流体水平显示流向无效".into()));
        }
        Ok(Self {
            heights: heights.map(|h| NonZeroU32::new(h.to_bits()).expect("已验证正的有限高度")),
            flow: flow.map(|v| {
                if v == 0.0 {
                    0.0f32.to_bits()
                } else {
                    v.to_bits()
                }
            }),
            kind,
            faces: crate::surface::ALL_FACES,
            overlays: 0,
        })
    }
    pub fn height(self, east: bool, south: bool) -> f32 {
        f32::from_bits(self.heights[usize::from(south) * 2 + usize::from(east)].get())
    }
    pub fn maximum(self) -> f32 {
        self.heights
            .into_iter()
            .map(|h| f32::from_bits(h.get()))
            .fold(0.0, f32::max)
    }
    pub fn flow(self) -> [f32; 2] {
        self.flow.map(f32::from_bits)
    }
    pub fn kind(self) -> Kind {
        self.kind
    }
    pub fn faces(self) -> u8 {
        self.faces
    }
    pub fn overlay_faces(self) -> u8 {
        self.overlays
    }
    pub fn with_overlays(mut self, overlays: u8) -> Result<Self, WorldError> {
        if overlays & (!self.faces | crate::surface::FACE_TOP | crate::surface::FACE_BOTTOM) != 0 {
            return Err(WorldError::Invalid(
                "水 overlay 必须属于可见水平侧面".into(),
            ));
        }
        self.overlays = overlays;
        Ok(self)
    }
    pub fn with_faces(mut self, faces: u8) -> Result<Self, WorldError> {
        if faces & !crate::surface::ALL_FACES != 0 {
            return Err(WorldError::Invalid("流体可见面包含未知方向".into()));
        }
        self.faces = faces;
        self.overlays &= faces;
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Water,
    Lava,
}
/// 原版独立流体的命名状态；含水本体和无准确资产的数字色块不在此列。
pub fn is_named_fluid(state: &State) -> bool {
    matches!(
        state.name.as_str(),
        "minecraft:water"
            | "minecraft:flowing_water"
            | "minecraft:lava"
            | "minecraft:flowing_lava"
            | "minecraft:bubble_column"
    )
}
/// 原生状态是否携带流体；外部命名空间的同名属性不视为原版合同。
pub fn kind(state: &State) -> Option<Kind> {
    match state.name.as_str() {
        "minecraft:water"
        | "minecraft:flowing_water"
        | "numeric:8"
        | "numeric:9"
        | "minecraft:bubble_column"
        | "minecraft:kelp"
        | "minecraft:kelp_plant"
        | "minecraft:seagrass"
        | "minecraft:tall_seagrass" => Some(Kind::Water),
        "minecraft:lava" | "minecraft:flowing_lava" | "numeric:10" | "numeric:11" => {
            Some(Kind::Lava)
        }
        name if name.starts_with("minecraft:")
            && state
                .properties
                .iter()
                .any(|(key, value)| key == "waterlogged" && value == "true") =>
        {
            Some(Kind::Water)
        }
        _ => None,
    }
}
/// 只有原始空气提供零高度；未知格和未解释的运行时固体属性不当作空气。
#[derive(Clone, Copy)]
pub enum Sample {
    Unknown,
    Air,
    Other,
    Fluid(Kind, u8),
}
impl Sample {
    pub fn vanilla(state: &State) -> Result<Self, WorldError> {
        let (kind, property) = match state.name.as_str() {
            "minecraft:water" | "minecraft:flowing_water" => (Kind::Water, "level"),
            "minecraft:lava" | "minecraft:flowing_lava" => (Kind::Lava, "level"),
            // 已核对五份原 BlockDumper 表：这四个 ID 的 data 与 level 逐值一致。
            "numeric:8" | "numeric:9" => (Kind::Water, "data"),
            "numeric:10" | "numeric:11" => (Kind::Lava, "data"),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "numeric:0" => {
                return Ok(Self::Air)
            }
            _ => {
                if state.name.starts_with("minecraft:") {
                    if state.properties.iter().any(|(key, value)| {
                        key == "waterlogged" && !matches!(value.as_str(), "true" | "false")
                    }) {
                        return Err(WorldError::Invalid(format!(
                            "原生 waterlogged 属性无效：{state:?}"
                        )));
                    }
                    if kind(state).is_some() {
                        return Ok(Self::Fluid(Kind::Water, 0));
                    }
                }
                return Ok(Self::Other);
            }
        };
        let level = state
            .properties
            .iter()
            .find(|(key, _)| key == property)
            .map(|(_, value)| value.parse::<u8>().ok().filter(|level| *level <= 15))
            .unwrap_or(Some(0))
            .ok_or_else(|| WorldError::Invalid(format!("流体 level 无效：{state:?}")))?;
        Ok(Self::Fluid(kind, level))
    }
}

/// 基于真实本格、八个水平邻格及其上方格求角点；查询只在本次后台读取中发生。
pub fn inspect(
    source: Sample,
    position: (i32, i32, i32),
    query: &mut dyn FnMut((i32, i32, i32)) -> Result<Sample, WorldError>,
) -> Result<Option<FluidSurface>, WorldError> {
    let Sample::Fluid(kind, own_level) = source else {
        return Ok(None);
    };
    let own_height = level_height(own_level)?;
    let mut samples = [[None; 3]; 3];
    let mut states = [[Sample::Unknown; 3]; 3];
    for (iz, dz) in (-1..=1).enumerate() {
        for (ix, dx) in (-1..=1).enumerate() {
            let (Some(x), Some(z), Some(above)) = (
                position.0.checked_add(dx),
                position.2.checked_add(dz),
                position.1.checked_add(1),
            ) else {
                continue;
            };
            let state = if dx == 0 && dz == 0 {
                source
            } else {
                query((x, position.1, z))?
            };
            states[iz][ix] = state;
            samples[iz][ix] = match state {
                Sample::Fluid(other, level) if other == kind => {
                    if matches!(query((x, above, z))?, Sample::Fluid(other, _) if other == kind) {
                        Some(1.0)
                    } else {
                        Some(level_height(level)?)
                    }
                }
                Sample::Air => Some(0.0),
                _ => None,
            };
        }
    }
    let own = samples[1][1].unwrap_or(own_height);
    let mut corners = [own; 4];
    for (i, (x, z)) in [(0, 0), (2, 0), (0, 2), (2, 2)].into_iter().enumerate() {
        let a = samples[1][x];
        let b = samples[z][1];
        let diagonal = if a.is_some_and(|h| h > 0.0) || b.is_some_and(|h| h > 0.0) {
            samples[z][x]
        } else {
            None
        };
        let values = [Some(own), a, b, diagonal];
        if values.into_iter().flatten().any(|h| h >= 1.0) {
            corners[i] = 1.0;
            continue;
        }
        let (mut height, mut weights) = (0.0, 0.0);
        for sample in values.into_iter().flatten() {
            let weight = if sample >= 0.8 { 10.0 } else { 1.0 };
            height += sample * weight;
            weights += weight;
        }
        corners[i] = height / weights;
    }
    // 1.21.1 FlowingFluid#getFlow 的水平分量：使用原始 ownHeight，
    // 已知空气且下方有同类流体时计入 8/9 的落差；未知格不伪装为可流通空气。
    let mut flow = [0.0f64; 2];
    for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
        let neighbor = states[(dz + 1) as usize][(dx + 1) as usize];
        let difference = match neighbor {
            Sample::Fluid(other, level) if other == kind => own_height - level_height(level)?,
            Sample::Air => {
                let (Some(x), Some(y), Some(z)) = (
                    position.0.checked_add(dx),
                    position.1.checked_sub(1),
                    position.2.checked_add(dz),
                ) else {
                    continue;
                };
                match query((x, y, z))? {
                    Sample::Fluid(other, level) if other == kind => {
                        own_height - (level_height(level)? - 8.0 / 9.0)
                    }
                    _ => 0.0,
                }
            }
            _ => 0.0,
        };
        flow[0] += f64::from(dx as f32 * difference);
        flow[1] += f64::from(dz as f32 * difference);
    }
    let length = (flow[0] * flow[0] + flow[1] * flow[1]).sqrt();
    let flow = if length < 0.0001 {
        [0.0; 2]
    } else {
        flow.map(|v| (v / length) as f32)
    };
    FluidSurface::new(corners, flow, kind).map(Some)
}
fn level_height(level: u8) -> Result<f32, WorldError> {
    if level > 15 {
        return Err(WorldError::Invalid("流体 level 超出 0..=15".into()));
    }
    Ok(if level >= 8 {
        8.0 / 9.0
    } else {
        f32::from(8 - level) / 9.0
    })
}
