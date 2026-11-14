//! 结构变换中的实体公共坐标与完整乘客树；非位置载荷逐字节保留。
use super::{invalid, Mapping};
use crate::{
    nbt_document::{patch_batch, List, Operation, Value},
    sections::{parse_root, Tag},
    WorldError,
};
fn triples(node: &Tag<'_>, key: &str) -> Result<[f64; 3], WorldError> {
    let tag = node.require(key, 9).map_err(invalid)?;
    if tag.items.len() != 3 || tag.items.iter().any(|v| v.kind != 6 || v.raw.len() != 8) {
        return Err(invalid(format!("实体 {key} 必须为三个 Double")));
    }
    let values = std::array::from_fn(|i| {
        f64::from_be_bytes(tag.items[i].raw.try_into().expect("Double 宽度已核实"))
    });
    if !values.into_iter().all(f64::is_finite) {
        return Err(invalid(format!("实体 {key} 含非有限值")));
    }
    Ok(values)
}
fn list(values: [f64; 3]) -> Value {
    Value::List(List {
        element_type: 6,
        items: values.into_iter().map(Value::Double).collect(),
    })
}
/// 几何变换可固定某个实体；仅记录实际字节变化，保留 NBT 文档本身拒绝空编辑的约定。
pub(super) fn put(
    node: &Tag<'_>,
    path: &str,
    key: &str,
    value: Value,
    out: &mut Vec<Operation>,
) -> Result<(), WorldError> {
    if let Some(current) = node.field(key) {
        if current.raw == value.payload()?.as_slice() {
            return Ok(());
        }
        out.push(Operation::Replace {
            path: format!("{path}/{key}"),
            value,
        });
    } else {
        out.push(Operation::InsertField {
            path: path.into(),
            name: key.into(),
            value,
        });
    }
    Ok(())
}
pub(super) fn transform(
    compound: &[u8],
    origin: [i32; 3],
    map: &Mapping,
    rotate: bool,
    version: Option<i32>,
    variants: &std::collections::BTreeMap<String, crate::painting::Variant>,
    dimensions: &crate::entity_editing::DimensionChange,
) -> Result<Vec<u8>, WorldError> {
    fn walk(
        node: &Tag<'_>,
        path: &str,
        origin: [i32; 3],
        map: &Mapping,
        rotate: bool,
        version: Option<i32>,
        variants: &std::collections::BTreeMap<String, crate::painting::Variant>,
        dimensions: &crate::entity_editing::DimensionChange,
        out: &mut Vec<Operation>,
    ) -> Result<(), WorldError> {
        if node.kind != 10 {
            return Err(invalid("实体树节点必须为 Compound"));
        }
        let point = |p: [f64; 3]| {
            let result = map.point(std::array::from_fn(|i| p[i] - f64::from(origin[i])));
            std::array::from_fn(|i| result[i] + f64::from(origin[i]))
        };
        let position = triples(node, "Pos")?;
        let hanging =
            super::hanging::transform(node, path, version, variants, origin, map, rotate, out)?;
        if !hanging {
            put(node, path, "Pos", list(point(position)), out)?;
        }
        if rotate {
            crate::entity_references::orientation(node, path, version, |p| map.direction(p), out)?;
            let shulker_face = crate::entity_spatial::shulker_face(node, version)?;
            if let Some(face) = shulker_face {
                let direction =
                    map.direction(crate::hanging::DIRECTIONS[face as usize].map(f64::from));
                let target = crate::hanging::DIRECTIONS
                    .iter()
                    .position(|normal| normal.map(f64::from) == direction)
                    .ok_or_else(|| {
                        invalid("潜影贝 AttachFace 无法表示该朝向变换，请保留朝向或取消复制实体")
                    })?;
                if target != usize::from(face) {
                    put(node, path, "AttachFace", Value::Byte(target as i8), out)?;
                }
            }
            for key in ["Facing", "facing", "Direction", "AttachFace"] {
                if node.field(key).is_some()
                    && !map.orientation_identity()
                    && (!hanging || matches!(key, "Direction" | "AttachFace"))
                    && !(key == "AttachFace" && shulker_face.is_some())
                {
                    return Err(invalid(format!(
                        "实体 {key} 的此方向变换尚无已核实编码，请保留朝向或取消复制实体"
                    )));
                }
            }
            if node.field("Motion").is_some() {
                put(
                    node,
                    path,
                    "Motion",
                    list(map.direction(triples(node, "Motion")?)),
                    out,
                )?;
            }
            if let Some(rotation) = node.field("Rotation").filter(|_| !hanging) {
                if rotation.kind != 9
                    || rotation.items.len() != 2
                    || rotation
                        .items
                        .iter()
                        .any(|v| v.kind != 5 || v.raw.len() != 4)
                {
                    return Err(invalid("实体 Rotation 必须为两个 Float"));
                }
                let angles: [f32; 2] = std::array::from_fn(|i| {
                    f32::from_be_bytes(rotation.items[i].raw.try_into().expect("Float 宽度已核实"))
                });
                if !angles.into_iter().all(f32::is_finite) {
                    return Err(invalid("实体角度不是有限数"));
                }
                let yaw = f64::from(angles[0]).to_radians();
                let pitch = f64::from(angles[1]).to_radians();
                let look = map.direction([
                    -yaw.sin() * pitch.cos(),
                    -pitch.sin(),
                    yaw.cos() * pitch.cos(),
                ]);
                let horizontal = look[0].hypot(look[2]);
                let result = [
                    (-look[0]).atan2(look[2]).to_degrees() as f32,
                    (-look[1]).atan2(horizontal).to_degrees() as f32,
                ];
                put(
                    node,
                    path,
                    "Rotation",
                    Value::List(List {
                        element_type: 5,
                        items: result.into_iter().map(Value::Float).collect(),
                    }),
                    out,
                )?;
            }
        }
        let tile = ["TileX", "TileY", "TileZ"];
        let count = tile.iter().filter(|key| node.field(key).is_some()).count();
        if count != 0 && !hanging {
            if count != 3 {
                return Err(invalid("悬挂实体 Tile 坐标不完整"));
            }
            let mut position = [0.; 3];
            for i in 0..3 {
                position[i] = f64::from(
                    node.require(tile[i], 3)
                        .and_then(Tag::i32)
                        .map_err(invalid)?,
                ) + 0.5;
            }
            let position = point(position);
            for i in 0..3 {
                if position[i] < f64::from(i32::MIN) || position[i] >= f64::from(i32::MAX) {
                    return Err(invalid("悬挂实体 Tile 变换越界"));
                }
                put(
                    node,
                    path,
                    tile[i],
                    Value::Int(position[i].floor() as i32),
                    out,
                )?;
            }
        }
        crate::entity_spatial::positions(
            node,
            path,
            version,
            dimensions,
            |anchor| {
                let mapped = point(anchor.map(|v| f64::from(v) + 0.5));
                let mut target = [0; 3];
                for axis in 0..3 {
                    let value = mapped[axis].floor();
                    if !value.is_finite()
                        || value < f64::from(i32::MIN)
                        || value > f64::from(i32::MAX)
                    {
                        return Err(invalid("实体空间坐标变换越界"));
                    }
                    target[axis] = value as i32;
                }
                Ok(target)
            },
            out,
        )?;
        if let Some(passengers) = node.field("Passengers") {
            if passengers.kind != 9 {
                return Err(invalid("Passengers 必须为列表"));
            }
            for (i, child) in passengers.items.iter().enumerate() {
                walk(
                    child,
                    &format!("{path}/Passengers/{i}"),
                    origin,
                    map,
                    rotate,
                    version,
                    variants,
                    dimensions,
                    out,
                )?;
            }
        }
        if let Some(child) = node.field("Riding") {
            walk(
                child,
                &format!("{path}/Riding"),
                origin,
                map,
                rotate,
                version,
                variants,
                dimensions,
                out,
            )?;
        }
        Ok(())
    }
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    let node = parse_root(&raw).map_err(invalid)?;
    let mut operations = Vec::new();
    walk(
        &node,
        "",
        origin,
        map,
        rotate,
        version,
        variants,
        dimensions,
        &mut operations,
    )?;
    if operations.is_empty() {
        return Ok(compound.to_vec());
    }
    let result = patch_batch(&raw, &operations)?;
    Ok(result[3..].to_vec())
}
pub(super) fn block_entity(
    compound: &[u8],
    position: (i32, i32, i32),
    map: &Mapping,
    rotate: bool,
) -> Result<Vec<u8>, WorldError> {
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    let mut operations = Vec::new();
    if rotate && !map.orientation_identity() {
        let node = parse_root(&raw).map_err(invalid)?;
        let id = node
            .require("id", 8)
            .and_then(Tag::string)
            .map_err(invalid)?;
        if !matches!(
            id.as_str(),
            "minecraft:chest"
                | "minecraft:trapped_chest"
                | "minecraft:barrel"
                | "minecraft:furnace"
                | "minecraft:blast_furnace"
                | "minecraft:smoker"
                | "minecraft:hopper"
                | "minecraft:dropper"
                | "minecraft:dispenser"
                | "minecraft:brewing_stand"
                | "minecraft:enchanting_table"
                | "minecraft:sign"
                | "minecraft:hanging_sign"
                | "minecraft:shulker_box"
                | "minecraft:bed"
                | "minecraft:banner"
                | "minecraft:skull"
                | "minecraft:lectern"
                | "minecraft:jukebox"
                | "minecraft:chiseled_bookshelf"
                | "minecraft:decorated_pot"
        ) {
            return Err(invalid(format!(
                "方块实体 {id} 尚无载荷朝向规则；可选择保留朝向"
            )));
        }
        if let Some(rot) = node.field("Rot") {
            let value = rot.byte().map_err(invalid)?;
            if !(0..16).contains(&value) || map.grid_direction([0, 1, 0]) != [0, 1, 0] {
                return Err(invalid("旧版头颅 Rot 不能表示该空间变换"));
            }
            let angle = f64::from(value) * std::f64::consts::TAU / 16.;
            let q = super::vector(map.basis, [-angle.sin(), 0., angle.cos()]);
            let rotation = ((-q[0]).atan2(q[2]) * 16. / std::f64::consts::TAU)
                .round()
                .rem_euclid(16.) as i8;
            operations.push(Operation::Replace {
                path: "/Rot".into(),
                value: Value::Byte(rotation),
            });
        }
    }
    if !operations.is_empty() {
        raw = patch_batch(&raw, &operations)?;
    }
    crate::be_editing::relocate_compound(&raw[3..], position)
}
