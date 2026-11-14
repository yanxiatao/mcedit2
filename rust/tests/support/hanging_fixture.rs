//! 世界与桌面类别共用完整悬挂实体载荷，未知业务字段留在原始 Compound 中。
use mcedit2_world::{
    entity_catalog::EntityVisual,
    hanging,
    nbt_document::{Field, List, Value},
    painting::Variant,
};
fn field(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
fn base(
    version: i32,
    id: &str,
    position: [f64; 3],
    facing: u8,
    index: i32,
    tile: [i32; 3],
) -> Vec<Field> {
    let mut out = vec![
        field("id", Value::String(id.into())),
        field(
            "Pos",
            Value::List(List {
                element_type: 6,
                items: position.map(Value::Double).to_vec(),
            }),
        ),
        field(
            "Rotation",
            Value::List(List {
                element_type: 5,
                items: hanging::angles(facing).map(Value::Float).to_vec(),
            }),
        ),
        field("TileX", Value::Int(tile[0])),
        field("TileY", Value::Int(tile[1])),
        field("TileZ", Value::Int(tile[2])),
        field("Opaque", Value::ByteArray(vec![-128, -1, 0, 127])),
        field(
            "Motion",
            Value::List(List {
                element_type: 6,
                items: [0., 0., 0.].map(Value::Double).to_vec(),
            }),
        ),
    ];
    if version == 1343 {
        out.push(field("UUIDMost", Value::Long(42)));
        out.push(field("UUIDLeast", Value::Long(i64::from(index))));
    } else {
        out.push(field("UUID", Value::IntArray(vec![0, 42, 0, index])));
    }
    out
}
pub fn frame(
    version: i32,
    facing: u8,
    rotation: u8,
    map: bool,
    index: i32,
    tile: [i32; 3],
) -> Value {
    let mut out = base(
        version,
        "minecraft:item_frame",
        hanging::anchor_center(tile, facing),
        facing,
        index,
        tile,
    );
    out.push(field(
        "Facing",
        Value::Byte(hanging::encode_facing(facing, version, false).unwrap()),
    ));
    out.push(field("ItemRotation", Value::Byte(rotation as i8)));
    out.push(field("Invisible", Value::Byte(0)));
    let mut item = vec![
        field(
            "id",
            if version == 1343 && map {
                Value::Short(358)
            } else {
                Value::String(
                    if map {
                        "minecraft:filled_map"
                    } else {
                        "minecraft:diamond_sword"
                    }
                    .into(),
                )
            },
        ),
        field(
            if version == 3955 { "count" } else { "Count" },
            if version == 3955 {
                Value::Int(1)
            } else {
                Value::Byte(1)
            },
        ),
        field("OpaqueItem", Value::LongArray(vec![i64::MIN, i64::MAX])),
    ];
    if version == 1343 {
        item.push(field("Damage", Value::Short(if map { 7 } else { 0 })));
    } else if map {
        item.push(field(
            if version == 3955 { "components" } else { "tag" },
            Value::Compound(vec![field(
                if version == 3955 {
                    "minecraft:map_id"
                } else {
                    "map"
                },
                Value::Int(7),
            )]),
        ));
    }
    out.push(field("Item", Value::Compound(item)));
    Value::Compound(out)
}
pub fn painting(
    version: i32,
    id: &str,
    art: &Variant,
    facing: u8,
    index: i32,
    tile: [i32; 3],
) -> Value {
    let source = mcedit2_world::painting::Source {
        variant: art.clone(),
        facing,
        tile,
    };
    let canonical = hanging::HORIZONTAL[facing as usize];
    let mut out = base(
        version,
        "minecraft:painting",
        source.center(),
        canonical,
        index,
        tile,
    );
    out.push(field(
        if version == 1343 { "Facing" } else { "facing" },
        Value::Byte(facing as i8),
    ));
    out.push(if version == 1343 {
        field("Motive", Value::String(art.motive.clone().unwrap()))
    } else {
        field("variant", Value::String(id.into()))
    });
    Value::Compound(out)
}
pub fn visual(value: &Value, version: i32) -> EntityVisual {
    EntityVisual::parse_block_entity(&value.payload().unwrap(), Some(version)).unwrap()
}
