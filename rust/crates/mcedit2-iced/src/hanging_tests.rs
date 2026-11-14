//! 地图展示框的三版本编码、六向几何、完整源范围与实际地图资源归属。
use super::*;
use crate::{
    hanging_fixture as f,
    metadata_fixture::{field, gzip, nbt, Fixture},
};
use mcedit2_world::{
    hanging,
    nbt_document::{Document, List, Operation, Value},
};
pub(super) fn complete() {
    for version in [1343, 3465, 3955] {
        let assets = super::frame_tests::assets(version);
        let bank = assets.open();
        let mut fixture = Fixture::new(version);
        crate::object_fixture::populate(&mut fixture);
        std::fs::create_dir(fixture.root.join("data")).unwrap();
        let map = nbt(vec![
            field("DataVersion", Value::Int(version)),
            field(
                "data",
                Value::Compound(vec![
                    field(
                        "dimension",
                        if version == 1343 {
                            Value::Byte(0)
                        } else {
                            Value::String("minecraft:overworld".into())
                        },
                    ),
                    field("scale", Value::Byte(0)),
                    field("width", Value::Short(128)),
                    field("height", Value::Short(128)),
                    field(
                        "colors",
                        Value::ByteArray(
                            (0..128 * 128)
                                .map(|i| if i % 128 < 64 { 18 } else { 34 })
                                .collect(),
                        ),
                    ),
                ]),
            ),
        ]);
        let map = gzip(&map);
        std::fs::write(fixture.root.join("data/map_7.dat"), &map).unwrap();
        let image = mcedit2_world::map_data::read(&fixture.root, 7, Some(version)).unwrap();
        let atlas = bank
            .atlas()
            .unwrap()
            .append_runtime_cancellable([crate::map_frame::texture(&image)], &|| false)
            .unwrap();
        let facings: Vec<u8> = if version == 1343 {
            hanging::HORIZONTAL.to_vec()
        } else {
            (0..6).collect()
        };
        for facing in facings {
            for rotation in 0..8 {
                for invisible in [false, true] {
                    let mut value = f::frame(version, facing, rotation, true, 1, [-19, 72, -17]);
                    if invisible {
                        let Value::Compound(fields) = &mut value else {
                            unreachable!()
                        };
                        fields
                            .iter_mut()
                            .find(|v| v.name == "Invisible")
                            .unwrap()
                            .value = Value::Byte(1);
                    }
                    let visual = f::visual(&value, version);
                    let parsed = crate::map_frame::parse(&visual).unwrap().unwrap();
                    assert_eq!(
                        (parsed.id, parsed.facing, parsed.rotation, parsed.invisible),
                        (7, facing, rotation, invisible && version != 1343)
                    );
                    let quad =
                        crate::map_frame::quad(&parsed, &image, &atlas, [-32, 0, -32]).unwrap();
                    let source = hanging::Frame::read(&visual).unwrap();
                    let (min, max) = source.render_bounds(true);
                    for p in quad.xyz {
                        for i in 0..3 {
                            let v = f64::from(p[i])
                                + f64::from([-32, 0, -32][i])
                                + if i == 0 || i == 2 { 0.5 } else { 0. };
                            assert!(
                                v >= min[i] - 0.00001 && v <= max[i] + 0.00001,
                                "{version} {facing} {rotation} {invisible}: {v} / {min:?} {max:?}"
                            );
                        }
                    }
                    let prepared = crate::frame_geometry::prepare(
                        &visual,
                        false,
                        &bank,
                        Some(&fixture.root),
                        Some(version),
                        false,
                        &|| false,
                        None,
                        None,
                    )
                    .unwrap();
                    let atlas = append_runtime_cached(
                        bank.atlas().unwrap(),
                        prepared.textures.clone(),
                        &|| false,
                    )
                    .unwrap();
                    let faces = prepared.quads(&atlas, true).unwrap();
                    assert_eq!(
                        faces.len(),
                        if invisible && version != 1343 { 1 } else { 23 }
                    );
                    let map = faces.last().unwrap();
                    let delta = std::array::from_fn(|i| {
                        (prepared.center[i] - f64::from([-32, 0, -32][i])) as f32
                    });
                    let map = map.translated(delta);
                    for (a, b) in map
                        .xyz
                        .into_iter()
                        .flatten()
                        .zip(quad.xyz.into_iter().flatten())
                    {
                        assert!((a - b).abs() < 0.00001);
                    }
                    assert_eq!(map.uv, quad.uv);
                    assert_eq!(
                        quad.uv,
                        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]]
                            .map(|[u, v]| atlas.entries[&image.texture_id()].sample(u, v))
                    );
                }
            }
        }
        let root = f::frame(version, 3, 3, true, 1, [1; 3]);
        let list = Value::List(List {
            element_type: 10,
            items: vec![root.clone()],
        });
        let raw = if version == 1343 {
            let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
                &fixture.world,
                "minecraft:overworld",
                0,
                0,
            )
            .unwrap()
            .unwrap();
            mcedit2_world::nbt_document::patch(
                &raw,
                &Operation::Replace {
                    path: "/Level/Entities".into(),
                    value: list,
                },
            )
            .unwrap()
        } else {
            nbt(vec![
                field("DataVersion", Value::Int(version)),
                field("Position", Value::IntArray(vec![0, 0])),
                field("Entities", list),
            ])
        };
        let path = fixture.root.join(if version == 1343 {
            "region/r.0.0.mca"
        } else {
            "entities/r.0.0.mca"
        });
        crate::object_fixture::write_region(&path, &[((0, 0), raw)]);
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let before = std::fs::read(&path).unwrap();
        let mut reader =
            mcedit2_world::scene_annotations::Reader::new(&fixture.world, "minecraft:overworld")
                .unwrap();
        let annotations = Arc::new(reader.read(0, 0).unwrap());
        let snapshot = Arc::new(Snapshot::build(
            (0, 0),
            1,
            std::iter::once(Arc::clone(&annotations)),
            Some(mcedit2_world::editing::Bounds::new((2, 1, 1), (2, 1, 1)).unwrap()),
        ));
        assert_eq!(
            snapshot.markers.len(),
            1,
            "地图边缘不能按展示框 0.75 格碰撞箱裁掉"
        );
        let frame = Frame::build_with_root(
            snapshot,
            &bank,
            Some(version),
            true,
            true,
            &|| false,
            Some(&fixture.root),
            false,
            &BTreeMap::new(),
            &BTreeMap::new(),
            crate::language::Language::English,
        )
        .unwrap();
        assert_eq!(
            (frame.rendered, frame.missing),
            (1, 0),
            "{version}: {:?}",
            frame.diagnostics
        );
        assert_eq!(
            frame.chunks.iter().map(|c| c.quads().len()).sum::<usize>(),
            23
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::read(fixture.root.join("data/map_7.dat")).unwrap(),
            map
        );
        let mut wrong = Document::new(nbt(if let Value::Compound(fields) = root {
            fields
        } else {
            unreachable!()
        }))
        .unwrap();
        let key = if version == 1343 {
            "/Item/Damage"
        } else if version == 3465 {
            "/Item/tag/map"
        } else {
            "/Item/components/minecraft:map_id"
        };
        wrong
            .apply(&Operation::Replace {
                path: key.into(),
                value: Value::String("损坏 ID".into()),
            })
            .unwrap();
        let Value::Compound(fields) = wrong.inspect("").unwrap() else {
            unreachable!()
        };
        assert!(crate::map_frame::parse(&f::visual(&Value::Compound(fields), version)).is_err());
    }
}
