//! 悬挂实体按真实锚点重建位置；固定尺寸不随体素缩放，未知业务载荷保持原字节。
use super::{entity::put, invalid, Mapping};
use crate::{
    hanging::{self, DIRECTIONS},
    nbt_document::{List, Operation, Value},
    sections::Tag,
    WorldError,
};
fn byte(node: &Tag<'_>, key: &str) -> Result<i8, WorldError> {
    node.field(key)
        .map(Tag::byte)
        .transpose()
        .map_err(invalid)
        .map(|n| n.unwrap_or(0))
}
/// 原版 structure 的画作 blockPos 是 Tile 锚点，其他实体是连续 Pos 所在格。
pub(crate) fn block_position(
    node: &Tag<'_>,
    origin: [i32; 3],
    position: [f64; 3],
) -> Result<Option<[i32; 3]>, WorldError> {
    let Some(id) = node.field("id") else {
        return Ok(None);
    };
    let id = id.string().map_err(invalid)?;
    if crate::painting::is_painting(&id) {
        let mut out = [0; 3];
        for (i, key) in ["TileX", "TileY", "TileZ"].iter().enumerate() {
            out[i] = node
                .require(key, 3)
                .and_then(Tag::i32)
                .map_err(invalid)?
                .checked_sub(origin[i])
                .ok_or_else(|| invalid("结构画作 blockPos 越界"))?;
        }
        Ok(Some(out))
    } else if hanging::is_frame(&id) {
        Ok(Some(position.map(|n| n.floor() as i32)))
    } else {
        Ok(None)
    }
}
fn map_item(node: &Tag<'_>, version: i32) -> Result<bool, WorldError> {
    let Some(item) = node.field("Item") else {
        return Ok(false);
    };
    if item.kind != 10 {
        return Err(invalid("展示框 Item 必须为 Compound"));
    }
    let Some(id) = item.field("id") else {
        return Ok(false);
    };
    Ok(match id.kind {
        8 => matches!(
            id.string().map_err(invalid)?.as_str(),
            "filled_map" | "minecraft:filled_map"
        ),
        2 if version == 1343 => i16::from_be_bytes(id.raw.try_into().map_err(invalid)?) == 358,
        3 if version == 1343 => id.i32().map_err(invalid)? == 358,
        _ => false,
    })
}
pub(super) fn transform(
    node: &Tag<'_>,
    path: &str,
    version: Option<i32>,
    variants: &std::collections::BTreeMap<String, crate::painting::Variant>,
    origin: [i32; 3],
    map: &Mapping,
    rotate: bool,
    out: &mut Vec<Operation>,
) -> Result<bool, WorldError> {
    let Some(id) = node.field("id") else {
        return Ok(false);
    };
    let id = id.string().map_err(invalid)?;
    if !hanging::is_hanging(&id) {
        return Ok(false);
    }
    let version = version.ok_or_else(|| invalid("悬挂实体缺少源 DataVersion，不能推测方向编码"))?;
    let painting = crate::painting::is_painting(&id);
    let key = if painting && version != 1343 {
        "facing"
    } else {
        "Facing"
    };
    let raw_facing = byte(node, key)?;
    let facing = hanging::decode_facing(raw_facing, version, painting).map_err(invalid)?;
    let mut tile = [0; 3];
    for (i, key) in ["TileX", "TileY", "TileZ"].iter().enumerate() {
        tile[i] = node.require(key, 3).and_then(Tag::i32).map_err(invalid)?;
    }
    let normal = DIRECTIONS[facing as usize];
    let tangent = [normal[2], 0, -normal[0]];
    let parity = if painting {
        let builtin = crate::painting::Catalog::builtin(version).map_err(invalid)?;
        let visual =
            crate::entity_catalog::EntityVisual::parse_block_entity(node.raw, Some(version))
                .map_err(invalid)?;
        let source =
            crate::painting::Source::read_bound(&visual, &builtin, variants).map_err(invalid)?;
        [
            if source.variant.width % 2 == 0 {
                0.5
            } else {
                0.
            },
            if source.variant.height % 2 == 0 {
                0.5
            } else {
                0.
            },
        ]
    } else {
        [0.; 2]
    };
    let old_offset = std::array::from_fn::<_, 3, _>(|i| {
        f64::from(tangent[i]) * parity[0] + if i == 1 { parity[1] } else { 0. }
    });
    let direction = if rotate {
        map.grid_direction(normal)
    } else {
        normal
    };
    let target_facing = DIRECTIONS
        .iter()
        .position(|n| *n == direction)
        .ok_or_else(|| invalid("悬挂方向不能表示该变换"))? as u8;
    let target_raw = hanging::encode_facing(target_facing, version, painting).map_err(invalid)?;
    if painting && rotate && map.grid_direction([0, 1, 0]) != [0, 1, 0] {
        return Err(invalid(
            "挂画 NBT 无法表达画面的倾斜或上下翻转；请保留朝向或关闭复制实体",
        ));
    }
    let target_tangent = [direction[2], 0, -direction[0]];
    let target_offset = std::array::from_fn::<_, 3, _>(|i| {
        f64::from(target_tangent[i]) * parity[0] + if i == 1 { parity[1] } else { 0. }
    });
    let rotated_offset = if rotate {
        map.grid_direction(old_offset.map(|n| (n * 2.) as i32))
            .map(|n| f64::from(n) * 0.5)
    } else {
        old_offset
    };
    let anchor = map.point(std::array::from_fn(|i| {
        f64::from(tile[i]) + 0.5 - f64::from(origin[i])
    }));
    let mut new_tile = [0; 3];
    for i in 0..3 {
        let value =
            (anchor[i] + f64::from(origin[i]) + rotated_offset[i] - target_offset[i]).floor();
        if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
            return Err(invalid("悬挂实体锚点变换越界"));
        }
        new_tile[i] = value as i32;
        put(
            node,
            path,
            ["TileX", "TileY", "TileZ"][i],
            Value::Int(new_tile[i]),
            out,
        )?;
    }
    let center = hanging::anchor_center(new_tile, target_facing);
    put(
        node,
        path,
        "Pos",
        Value::List(List {
            element_type: 6,
            items: std::array::from_fn::<_, 3, _>(|i| Value::Double(center[i] + target_offset[i]))
                .to_vec(),
        }),
        out,
    )?;
    if rotate {
        if target_raw != raw_facing {
            put(node, path, key, Value::Byte(target_raw), out)?;
        }
        put(
            node,
            path,
            "Rotation",
            Value::List(List {
                element_type: 5,
                items: hanging::angles(target_facing).map(Value::Float).to_vec(),
            }),
            out,
        )?;
        if !painting {
            let rotation = byte(node, "ItemRotation")?;
            if !(0..8).contains(&rotation) {
                return Err(invalid("展示框 ItemRotation 必须为 0 至 7"));
            }
            let is_map = map_item(node, version)?;
            let up = map.grid_direction(hanging::frame_up(facing, rotation as u8, is_map));
            let target = (0..if is_map { 4 } else { 8 })
                .find(|r| hanging::frame_up(target_facing, *r, is_map) == up)
                .ok_or_else(|| invalid("展示框内容角度不能表示该变换"))?;
            let target = if is_map {
                target + (rotation as u8 / 4) * 4
            } else {
                target
            };
            if target != rotation as u8 {
                put(node, path, "ItemRotation", Value::Byte(target as i8), out)?;
            }
        }
    }
    Ok(true)
}
