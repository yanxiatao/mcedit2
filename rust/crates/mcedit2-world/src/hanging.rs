//! 悬挂实体的版本化方向和原版坐标基底；地图显示、源范围与结构变换共同使用。
use crate::{entity_catalog::EntityVisual, nbt_document::Value};

/// 原版六向序号：下、上、北、南、西、东。
pub const DIRECTIONS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];
pub const HORIZONTAL: [u8; 4] = [3, 4, 2, 5];
pub fn is_frame(id: &str) -> bool {
    matches!(
        id,
        "ItemFrame" | "minecraft:item_frame" | "minecraft:glow_item_frame"
    )
}
pub fn is_hanging(id: &str) -> bool {
    is_frame(id) || crate::painting::is_painting(id)
}
pub fn is_map_item(item: &Value, version: i32) -> bool {
    let Value::Compound(fields) = item else {
        return false;
    };
    fields
        .iter()
        .find(|f| f.name == "id")
        .is_some_and(|f| match &f.value {
            Value::String(id) => matches!(id.as_str(), "filled_map" | "minecraft:filled_map"),
            Value::Short(358) | Value::Int(358) => version == 1343,
            _ => false,
        })
}
pub fn decode_facing(value: i8, version: i32, painting: bool) -> Result<u8, String> {
    if !matches!(version, 1343 | 3465 | 3955) {
        return Err(format!("尚未核对 DataVersion {version} 的悬挂方向"));
    }
    if painting || version == 1343 {
        HORIZONTAL
            .get(value as usize)
            .copied()
            .ok_or_else(|| "悬挂实体水平方向必须为 0 至 3".into())
    } else {
        if !(0..6).contains(&value) {
            return Err("展示框方向必须为 0 至 5".into());
        }
        Ok(value as u8)
    }
}
pub fn encode_facing(direction: u8, version: i32, painting: bool) -> Result<i8, String> {
    if !matches!(version, 1343 | 3465 | 3955) {
        return Err(format!("尚未核对 DataVersion {version} 的悬挂方向"));
    }
    if painting || version == 1343 {
        HORIZONTAL
            .iter()
            .position(|n| *n == direction)
            .map(|n| n as i8)
            .ok_or_else(|| "此版本的悬挂实体不能朝向地面或天花板".into())
    } else if matches!(version, 3465 | 3955) && direction < 6 {
        Ok(direction as i8)
    } else {
        Err("没有对应的悬挂方向编码".into())
    }
}
/// ItemFrameRenderer 的 Rx(pitch) × Ry(180-yaw)，保持精确直角整数基底。
pub fn frame_basis(facing: u8, [x, y, z]: [i32; 3]) -> [i32; 3] {
    match facing {
        0 => [-x, z, y],
        1 => [-x, -z, -y],
        2 => [x, y, z],
        3 => [-x, y, -z],
        4 => [z, y, -x],
        5 => [-z, y, x],
        _ => unreachable!("方向已经校验"),
    }
}
pub fn frame_up(facing: u8, rotation: u8, map: bool) -> [i32; 3] {
    let octant = if map {
        (rotation % 4) * 2
    } else {
        rotation % 8
    };
    let [x, y] = [
        [0, 1],
        [-1, 1],
        [-1, 0],
        [-1, -1],
        [0, -1],
        [1, -1],
        [1, 0],
        [1, 1],
    ][octant as usize];
    frame_basis(facing, [x, y, 0])
}
pub fn angles(facing: u8) -> [f32; 2] {
    // NBT Rotation 为 yaw / pitch；垂直展示框的 yaw 固定为零。
    match facing {
        0 => [0., 90.],
        1 => [0., -90.],
        2 => [180., 0.],
        3 => [0., 0.],
        4 => [90., 0.],
        5 => [270., 0.],
        _ => unreachable!("方向已经校验"),
    }
}
pub fn anchor_center(tile: [i32; 3], facing: u8) -> [f64; 3] {
    std::array::from_fn(|i| {
        f64::from(tile[i]) + 0.5 - f64::from(DIRECTIONS[facing as usize][i]) * 0.46875
    })
}
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub tile: [i32; 3],
    pub facing: u8,
    pub rotation: u8,
    pub invisible: bool,
}
impl Frame {
    pub fn read(visual: &EntityVisual) -> Result<Self, String> {
        let version = visual.source_version.ok_or("展示框缺少源 DataVersion")?;
        let facing = match visual.source_field_optional("Facing")? {
            None => 0,
            Some(Value::Byte(n)) => n,
            _ => return Err("展示框 Facing 必须为 Byte".into()),
        };
        let facing = decode_facing(facing, version, false)?;
        let mut tile = [0; 3];
        for (i, key) in ["TileX", "TileY", "TileZ"].iter().enumerate() {
            let Value::Int(n) = visual.source_field(key)? else {
                return Err(format!("展示框 {key} 必须为 Int"));
            };
            tile[i] = n;
        }
        let rotation = match visual.source_field_optional("ItemRotation")? {
            None => 0,
            Some(Value::Byte(n)) if (0..8).contains(&n) => n as u8,
            _ => return Err("展示框 ItemRotation 必须为 0 至 7 的 Byte".into()),
        };
        // 1.12.2 的 Entity / HangingEntity / ItemFrame 均不读写此 NBT 字段。
        let invisible = if version == 1343 {
            false
        } else {
            match visual.source_field_optional("Invisible")? {
                None => false,
                Some(Value::Byte(n)) => n != 0,
                _ => return Err("展示框 Invisible 必须为 Byte".into()),
            }
        };
        Ok(Self {
            tile,
            facing,
            rotation,
            invisible,
        })
    }
    pub fn center(&self) -> [f64; 3] {
        anchor_center(self.tile, self.facing)
    }
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let center = self.center();
        let normal = DIRECTIONS[self.facing as usize];
        let half = normal.map(|n| if n == 0 { 0.375 } else { 0.03125 });
        (
            std::array::from_fn(|i| center[i] - half[i]),
            std::array::from_fn(|i| center[i] + half[i]),
        )
    }
    pub fn render_bounds(&self, map: bool) -> ([f64; 3], [f64; 3]) {
        let (mut min, mut max) = self.bounds();
        if map {
            let center = self.center();
            let normal = DIRECTIONS[self.facing as usize];
            let depth = if self.invisible { 0.5 } else { 0.4375 } - 1.01 / 128.;
            let plane = std::array::from_fn::<_, 3, _>(|i| {
                center[i] + f64::from(normal[i]) * (0.46875 - depth)
            });
            for i in 0..3 {
                let half = if normal[i] == 0 { 0.5 } else { 0. };
                min[i] = min[i].min(plane[i] - half);
                max[i] = max[i].max(plane[i] + half);
            }
        }
        (min, max)
    }
}
