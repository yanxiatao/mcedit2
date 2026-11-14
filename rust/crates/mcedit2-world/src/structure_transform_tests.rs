//! 在既有结构交换大类中验证完整网格、离散朝向、NBT 载荷及重采样。
use super::*;
use crate::{
    clipboard::{OpaqueBlockEntity, OpaqueEntity},
    nbt_document::{Field, List, Operation, Value},
    sections::parse_root,
};
fn field(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
fn compound(fields: Vec<Field>) -> Vec<u8> {
    let raw = crate::nbt_document::patch(
        &[10, 0, 0, 0],
        &Operation::InsertField {
            path: String::new(),
            name: "value".into(),
            value: Value::Compound(fields),
        },
    )
    .unwrap();
    parse_root(&raw)
        .unwrap()
        .field("value")
        .unwrap()
        .raw
        .to_vec()
}
fn document(bytes: &[u8]) -> crate::nbt_document::Document {
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(bytes);
    crate::nbt_document::Document::new(raw).unwrap()
}
fn entity(id: &str, position: [f64; 3], children: Vec<Vec<u8>>) -> Vec<u8> {
    static NEXT_ENTITY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);
    let identity = NEXT_ENTITY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut fields = vec![
        field("id", Value::String(id.into())),
        field("UUID", Value::IntArray(vec![1, 2, 3, identity])),
        field(
            "Pos",
            Value::List(List {
                element_type: 6,
                items: position.into_iter().map(Value::Double).collect(),
            }),
        ),
        field(
            "Motion",
            Value::List(List {
                element_type: 6,
                items: [1., 0., 0.].into_iter().map(Value::Double).collect(),
            }),
        ),
        field(
            "Rotation",
            Value::List(List {
                element_type: 5,
                items: vec![Value::Float(0.), Value::Float(0.)],
            }),
        ),
        field("Opaque", Value::ByteArray(vec![-11; 128])),
    ];
    if !children.is_empty() {
        fields.push(field(
            "Passengers",
            Value::List(List {
                element_type: 10,
                items: children
                    .into_iter()
                    .map(|b| document(&b).inspect("").unwrap())
                    .collect(),
            }),
        ));
    }
    compound(fields)
}
pub(super) fn structure_transform_pipeline() {
    let world = crate::WorldSummary {
        root: "structure-transform-world".into(),
        world_data: Default::default(),
        seed: None,
        level_name: None,
        data_version: Some(3955),
        storage_format: Some(19133),
        version_name: Some("1.21.1".into()),
        dimensions: vec![],
        warnings: vec![],
    };
    let ids = [
        "stone",
        "dirt",
        "sand",
        "glass",
        "granite",
        "andesite",
        "diamond_block",
        "oak_planks",
    ];
    let source = crate::clipboard::new(
        &world,
        "minecraft:overworld",
        false,
        [0, -64, 0],
        [2; 3],
        ids.map(|id| crate::editing::parse_state(&format!("minecraft:{id}")).unwrap())
            .to_vec(),
    )
    .unwrap();
    for axis in 0..3 {
        let mut transform = Transform::default();
        transform.rotation[axis] = 90.;
        let target = source.resampled_cancellable(transform, || false).unwrap();
        assert_eq!(target.size, [2; 3]);
        assert_eq!(target.blocks[0], source.blocks[[2, 1, 4][axis]]);
        let mut restored = target;
        for _ in 0..3 {
            restored = restored.resampled_cancellable(transform, || false).unwrap();
        }
        assert_eq!(restored.blocks, source.blocks);
        assert_eq!(restored.offset, source.offset);
    }
    for x in [0., 90., 180., 270.] {
        for y in [0., 90., 180., 270.] {
            for z in [0., 90., 180., 270.] {
                let t = Transform {
                    rotation: [x, y, z],
                    ..Default::default()
                };
                let rotated = source.resampled_cancellable(t, || false).unwrap();
                let restored = rotated
                    .resampled_cancellable(
                        Transform {
                            rotation: [0., 0., -z],
                            ..Default::default()
                        },
                        || false,
                    )
                    .unwrap()
                    .resampled_cancellable(
                        Transform {
                            rotation: [0., -y, 0.],
                            ..Default::default()
                        },
                        || false,
                    )
                    .unwrap()
                    .resampled_cancellable(
                        Transform {
                            rotation: [-x, 0., 0.],
                            ..Default::default()
                        },
                        || false,
                    )
                    .unwrap();
                assert_eq!(restored.blocks, source.blocks, "XYZ {x}/{y}/{z}");
            }
        }
    }
    let mut payload = source.clone();
    payload.blocks[0] =
        crate::editing::parse_state("minecraft:chest[facing=north,type=single,waterlogged=false]")
            .unwrap();
    let chest = compound(vec![
        field("id", Value::String("minecraft:chest".into())),
        field("x", Value::Int(0)),
        field("y", Value::Int(-64)),
        field("z", Value::Int(0)),
        field(
            "Items",
            Value::List(List {
                element_type: 10,
                items: vec![Value::Compound(vec![
                    field("id", Value::String("minecraft:diamond".into())),
                    field("count", Value::Int(64)),
                    field("Slot", Value::Byte(0)),
                ])],
            }),
        ),
        field("Opaque", Value::ByteArray(vec![73; 256])),
    ]);
    payload.block_entities.push(OpaqueBlockEntity {
        position: [0, 0, 0],
        compound: chest.clone(),
    });
    payload.entities.push(OpaqueEntity {
        position: [0.25, 0.5, 0.75],
        block_position: None,
        compound: entity(
            "minecraft:pig",
            [0.25, -63.5, 0.75],
            vec![entity("minecraft:chicken", [0.5, -63., 1.5], vec![])],
        ),
    });
    payload.biomes = Some(BiomeVolume {
        resolution: 1,
        size: [2; 3],
        cells: vec!["minecraft:plains".into(); 8],
    });
    payload.validate(&world, "minecraft:overworld").unwrap();
    let grown = payload
        .resampled_cancellable(
            Transform {
                scale: [2.; 3],
                ..Default::default()
            },
            || false,
        )
        .unwrap();
    assert_eq!(grown.size, [4; 3]);
    assert_eq!(grown.block_entities.len(), 8);
    assert_eq!(grown.entities.len(), 1);
    grown.validate(&world, "minecraft:overworld").unwrap();
    for be in &grown.block_entities {
        let doc = document(&be.compound);
        assert_eq!(
            doc.inspect("/Items").unwrap(),
            document(&chest).inspect("/Items").unwrap()
        );
        assert_eq!(
            doc.inspect("/Opaque").unwrap(),
            Value::ByteArray(vec![73; 256])
        );
    }
    let shrunk = grown
        .resampled_cancellable(
            Transform {
                scale: [0.5; 3],
                ..Default::default()
            },
            || false,
        )
        .unwrap();
    shrunk.validate(&world, "minecraft:overworld").unwrap();
    assert_eq!(shrunk.blocks, payload.blocks);
    assert_eq!(shrunk.entities[0].position, payload.entities[0].position);
    assert_eq!(shrunk.block_entities, payload.block_entities);
    assert_eq!(
        document(&shrunk.entities[0].compound)
            .inspect("/Passengers/0/Pos")
            .unwrap(),
        document(&payload.entities[0].compound)
            .inspect("/Passengers/0/Pos")
            .unwrap()
    );
    let arbitrary = source
        .resampled_cancellable(
            Transform {
                rotation: [25., 38., 17.],
                ..Default::default()
            },
            || false,
        )
        .unwrap();
    arbitrary.validate(&world, "minecraft:overworld").unwrap();
    assert!(arbitrary.mask.as_ref().unwrap().iter().any(|v| !*v));
    for (selected, block) in arbitrary
        .mask
        .as_ref()
        .unwrap()
        .iter()
        .zip(&arbitrary.blocks)
    {
        if *selected {
            assert!(source.blocks.contains(block));
        }
    }
    for state in [
        "minecraft:observer[facing=north,powered=false]",
        "minecraft:oak_log[axis=y]",
        "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]",
    ] {
        let mut value = source.clone();
        value
            .blocks
            .fill(crate::editing::parse_state(state).unwrap());
        let mapped = value
            .resampled_cancellable(
                Transform {
                    rotation: [90., 0., 0.],
                    ..Default::default()
                },
                || false,
            )
            .unwrap();
        let restored = mapped
            .resampled_cancellable(
                Transform {
                    rotation: [-90., 0., 0.],
                    ..Default::default()
                },
                || false,
            )
            .unwrap();
        assert_eq!(restored.blocks, value.blocks);
    }
    let mut modded = source.clone();
    modded
        .blocks
        .fill(crate::editing::parse_state("custom:machine[facing=north]").unwrap());
    assert!(modded
        .resampled_cancellable(
            Transform {
                rotation: [90., 0., 0.],
                ..Default::default()
            },
            || false
        )
        .is_err());
    let preserved = modded
        .resampled_cancellable(
            Transform {
                rotation: [90., 0., 0.],
                rotate_states: false,
                ..Default::default()
            },
            || false,
        )
        .unwrap();
    assert_eq!(preserved.blocks, modded.blocks);
    assert!(source
        .resampled_cancellable(
            Transform {
                scale: [0., 1., 1.],
                ..Default::default()
            },
            || false
        )
        .is_err());
    assert!(source
        .resampled_cancellable(Transform::default(), || true)
        .is_err());
}
