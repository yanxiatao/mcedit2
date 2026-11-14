//! 地图源和原生客户端数值参照，供既有 NBT / 渲染类别共用。
use mcedit2_world::nbt_document::{Document, Field, List, Value};

pub fn reference(version: i32) -> serde_json::Value {
    serde_json::from_str(match version {
        1343 => include_str!("../../crates/mcedit2-world/tests/fixtures/map-reference-1.12.2.json"),
        3465 => include_str!("../../crates/mcedit2-world/tests/fixtures/map-reference-1.20.1.json"),
        3955 => include_str!("../../crates/mcedit2-world/tests/fixtures/map-reference-1.21.1.json"),
        _ => unreachable!(),
    })
    .unwrap()
}
pub fn payload(value: &serde_json::Value) -> Value {
    let mut bytes = vec![10, 0, 0];
    bytes.extend(
        value["payload"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as u8),
    );
    Document::new(bytes).unwrap().inspect("").unwrap()
}
pub fn field(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
pub fn change(value: &mut Value, name: &str, new: Value) {
    let Value::Compound(fields) = value else {
        unreachable!()
    };
    if let Some(f) = fields.iter_mut().find(|f| f.name == name) {
        f.value = new;
    } else {
        fields.push(field(name, new));
    }
}
pub fn list(items: Vec<Value>) -> Value {
    Value::List(List {
        element_type: 10,
        items,
    })
}
pub fn data(version: i32) -> Value {
    let mut fields = vec![
        field("scale", Value::Byte(0)),
        field("xCenter", Value::Int(0)),
        field("zCenter", Value::Int(0)),
        field(
            "dimension",
            if version == 1343 {
                Value::Byte(0)
            } else {
                Value::String("minecraft:overworld".into())
            },
        ),
        field("colors", Value::ByteArray(vec![18; 128 * 128])),
    ];
    if version != 1343 {
        let reference = reference(version);
        fields.push(field(
            "banners",
            list(
                reference["banners"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        let mut banner = payload(b);
                        if version == 3955 {
                            change(
                                &mut banner,
                                "pos",
                                Value::IntArray(vec![-19 + i as i32, 72, -17]),
                            );
                        } else {
                            change(
                                &mut banner,
                                "Pos",
                                Value::Compound(vec![
                                    field("X", Value::Int(-19 + i as i32)),
                                    field("Y", Value::Int(72)),
                                    field("Z", Value::Int(-17)),
                                ]),
                            );
                        }
                        banner
                    })
                    .collect(),
            ),
        ));
        let sample = payload(&reference["samples"][4]);
        let Value::Compound(sample) = sample else {
            unreachable!()
        };
        fields.push(sample.into_iter().find(|f| f.name == "frames").unwrap());
    }
    Value::Compound(fields)
}
pub fn write(root: &std::path::Path, version: i32, data: Value) {
    let mut fields = vec![field("data", data)];
    if version != 1343 {
        fields.push(field("DataVersion", Value::Int(version)));
    }
    let mut bytes = vec![10, 0, 0];
    bytes.extend(Value::Compound(fields).payload().unwrap());
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(root.join("data/map_7.dat"), bytes).unwrap();
}
pub fn decorated_item(version: i32, type_name: &str, key: &str) -> Value {
    let mut fields = vec![field("id", Value::String("minecraft:filled_map".into()))];
    let mut decoration = vec![
        field("x", Value::Double(10.)),
        field("z", Value::Double(-10.)),
    ];
    if version == 3955 {
        fields.push(field("count", Value::Int(1)));
        decoration.extend([
            field("type", Value::String(type_name.into())),
            field("rotation", Value::Float(-90.)),
        ]);
        fields.push(field(
            "components",
            Value::Compound(vec![
                field("minecraft:map_id", Value::Int(7)),
                field(
                    "minecraft:map_decorations",
                    Value::Compound(vec![field(key, Value::Compound(decoration))]),
                ),
            ]),
        ));
    } else {
        let definition = &mcedit2_world::map_decorations::catalog(version).unwrap()[type_name];
        fields.push(field("Count", Value::Byte(1)));
        decoration.extend([
            field("id", Value::String(key.into())),
            field("type", Value::Byte(definition.legacy_icon.unwrap() as i8)),
            field("rot", Value::Double(-90.)),
        ]);
        let mut tag = vec![field(
            "Decorations",
            list(vec![Value::Compound(decoration)]),
        )];
        if version == 1343 {
            fields.push(field("Damage", Value::Short(7)));
        } else {
            tag.push(field("map", Value::Int(7)));
        }
        fields.push(field("tag", Value::Compound(tag)));
    }
    Value::Compound(fields)
}

pub fn item_decoration(item: &mut Value, version: i32) -> &mut Value {
    let Value::Compound(fields) = item else {
        unreachable!()
    };
    let Value::Compound(data) = &mut fields
        .iter_mut()
        .find(|f| f.name == if version == 3955 { "components" } else { "tag" })
        .unwrap()
        .value
    else {
        unreachable!()
    };
    let entry = &mut data
        .iter_mut()
        .find(|f| {
            f.name
                == if version == 3955 {
                    "minecraft:map_decorations"
                } else {
                    "Decorations"
                }
        })
        .unwrap()
        .value;
    if version == 3955 {
        let Value::Compound(entries) = entry else {
            unreachable!()
        };
        &mut entries[0].value
    } else {
        let Value::List(list) = entry else {
            unreachable!()
        };
        &mut list.items[0]
    }
}
