//! 挂画整组验证，归入既有渲染类别：原客户端数值、真实来源、数据包、资源覆盖与拾取。
use crate::metadata_fixture::field;
use crate::{
    entity_geometry::{self, Frame},
    painting_geometry,
    scene_layers::Snapshot,
    textures::{java_obj_tests::Fixture, LoadControl, TextureBank},
};
use image::ImageEncoder;
use mcedit2_world::{
    entity_catalog::EntityVisual,
    nbt_document::{List, Operation, Value},
    painting::{Catalog, Source, Variant},
    scene_annotations::{ChunkAnnotations, Kind},
};
use std::{collections::BTreeMap, sync::Arc};

fn visual(version: i32, id: &str, variant: &Variant, facing: u8, tile: [i32; 3]) -> EntityVisual {
    crate::hanging_fixture::visual(
        &crate::hanging_fixture::painting(version, id, variant, facing, 1, tile),
        version,
    )
}

fn references() {
    for (version, json, count) in [
        (
            1343,
            include_str!("../../mcedit2-world/tests/fixtures/painting-reference-1.12.2.json"),
            26,
        ),
        (
            3465,
            include_str!("../../mcedit2-world/tests/fixtures/painting-reference-1.20.1.json"),
            30,
        ),
        (
            3955,
            include_str!("../../mcedit2-world/tests/fixtures/painting-reference-1.21.1.json"),
            50,
        ),
    ] {
        let catalog = Catalog::builtin(version).unwrap();
        assert_eq!(catalog.0.len(), count);
        let reference: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut maximum = 0_f32;
        for (id, art) in &catalog.0 {
            for facing in 0..4 {
                let source =
                    Source::read(&visual(version, id, art, facing, [-19, 72, -17]), &catalog)
                        .unwrap();
                let faces =
                    painting_geometry::faces(&source, (version == 1343).then_some(art)).unwrap();
                let case = if version == 1343 {
                    &reference[id]
                } else {
                    &reference[id][facing as usize]
                };
                let expected: Vec<[f32; 5]> =
                    serde_json::from_value(case["vertices"].clone()).unwrap();
                assert_eq!(faces.len() * 4, expected.len(), "{version} {id}");
                for (actual, expected) in faces
                    .iter()
                    .flat_map(|q| (0..4).map(move |i| (q.xyz[i], q.uv[i])))
                    .zip(expected)
                {
                    let xyz = if version == 1343 {
                        source.rotate([expected[0], expected[1], expected[2]])
                    } else {
                        [expected[0], expected[1], expected[2]]
                    };
                    for axis in 0..3 {
                        maximum = maximum.max((actual.0[axis] - xyz[axis]).abs());
                    }
                    for axis in 0..2 {
                        assert!(
                            (actual.1[axis] - expected[3 + axis]).abs() < 0.000001,
                            "{version} {id} {facing}: UV {:?} / {:?}",
                            actual.1,
                            &expected[3..]
                        );
                    }
                }
                if version == 3955 {
                    let expected: Vec<f64> =
                        serde_json::from_value(case["bounds"].clone()).unwrap();
                    let (min, max) = source.bounds();
                    assert_eq!([min, max].concat(), expected, "{id} {facing}");
                }
            }
        }
        assert!(maximum < 0.000002, "{version} 顶点偏差 {maximum}");
        eprintln!("挂画 {version}：{count} 幅 / 四朝向，原生角点最大偏差 {maximum}");
    }
}
fn png() -> Vec<u8> {
    let rgba: Vec<u8> = (0..256)
        .flat_map(|y| (0..256).flat_map(move |x| [x as u8, y as u8, 113, 255]))
        .collect();
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&rgba, 256, 256, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}
fn marker(visual: EntityVisual) -> mcedit2_world::scene_annotations::Marker {
    let catalog = Catalog::builtin(visual.source_version.unwrap()).unwrap();
    let source = Source::read(&visual, &catalog).unwrap();
    let mut marker =
        entity_geometry::tests::actor("minecraft:painting", source.center(), 0, BTreeMap::new());
    marker.kind = Kind::Item;
    marker.visual = Some(Arc::new(visual));
    let (mut minimum, mut maximum) = source.bounds();
    for i in [0, 2] {
        minimum[i] -= 0.5;
        maximum[i] -= 0.5;
    }
    marker.geometry = Some(mcedit2_world::scene_annotations::Geometry::Box { minimum, maximum });
    marker
}
fn resources_and_picking() {
    let fixture = Fixture::new();
    let bytes = png();
    let catalog = Catalog::builtin(3955).unwrap();
    for art in catalog.0.values() {
        let (ns, path) = art.asset_id.split_once(':').unwrap();
        fixture.put(&format!("assets/{ns}/textures/painting/{path}.png"), &bytes);
    }
    fixture.put("assets/minecraft/textures/painting/back.png", &bytes);
    fixture.put(
        "assets/minecraft/textures/painting/paintings_kristoffer_zetterstrand.png",
        &bytes,
    );
    let bank = fixture.open();
    let atlas = bank.atlas().unwrap();
    for version in [1343, 3465, 3955] {
        let catalog = Catalog::builtin(version).unwrap();
        for (id, art) in &catalog.0 {
            for facing in 0..4 {
                let source = visual(version, id, art, facing, [-19, 72, -17]);
                let (_, quads) = painting_geometry::quads(&source, &bank, atlas, true).unwrap();
                assert_eq!(
                    quads.len(),
                    usize::from(art.width) * usize::from(art.height) * 6
                );
                assert!(quads.iter().all(|q| q.single_sided() && !q.blended()));
                let frame = Frame::build(
                    entity_geometry::tests::snapshot(vec![marker(source.clone())]),
                    &bank,
                    Some(3955),
                    true,
                    &|| false,
                )
                .unwrap();
                assert_eq!(
                    (frame.rendered, frame.missing),
                    (1, 0),
                    "{version} {id} {facing}: {:?}",
                    frame.diagnostics
                );
                let source = Source::read(&source, &catalog).unwrap();
                let center = source.center();
                let center = [center[0] - 0.5, center[1], center[2] - 0.5];
                let normal = source.rotate([0., 0., -1.]);
                let chunk = &frame.chunks[0];
                let hit = chunk.picking.cast(
                    crate::spectator::Camera {
                        position: std::array::from_fn(|i| center[i] + f64::from(normal[i]) * 4.),
                        yaw: (180. + f32::from(facing) * 90.) % 360.,
                        pitch: 0.,
                        zoom: 1.,
                        perspective: true,
                    },
                    iced::Point::new(64., 64.),
                    crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(128., 128.)),
                    Some(&frame.atlas),
                    crate::gpu_mesh::Quality::Fast,
                    f64::INFINITY,
                );
                // 具体距离由生产拾取检查；同一源只生成一个归属组。
                assert!(hit.is_some(), "{version} {id} {facing}");
                let (_, white) = painting_geometry::quads(
                    &visual(version, id, art, facing, [-19, 72, -17]),
                    &bank,
                    atlas,
                    false,
                )
                .unwrap();
                assert!(white
                    .iter()
                    .all(|q| q.uv == [atlas.white.sample(0.5, 0.5); 4]));
            }
        }
    }
    let art = &catalog.0["minecraft:pointer"];
    let source = visual(3955, "minecraft:pointer", art, 2, [-19, 72, -17]);
    let marker = marker(source.clone());
    let clip = mcedit2_world::editing::Bounds::new((-21, 72, -17), (-21, 72, -17)).unwrap();
    let snapshot = Arc::new(Snapshot::build(
        (0, 0),
        1,
        std::iter::once(Arc::new(ChunkAnnotations {
            lighting: Default::default(),
            markers: vec![marker.clone()],
            diagnostics: vec![],
        })),
        Some(clip),
    ));
    assert_eq!(
        snapshot.markers.len(),
        1,
        "整幅画与选区相交时不能按中心裁掉"
    );
    let hidden = Frame::build_with_root(
        snapshot,
        &bank,
        Some(3955),
        true,
        false,
        &|| false,
        None,
        false,
        &BTreeMap::new(),
        &BTreeMap::new(),
        crate::language::Language::English,
    )
    .unwrap();
    assert!(hidden.chunks.is_empty());
    let overlay = Fixture::new();
    let mut red = Vec::new();
    image::codecs::png::PngEncoder::new(&mut red)
        .write_image(&[231, 17, 29, 0], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    overlay.put("assets/minecraft/textures/painting/pointer.png", &red);
    let overridden = TextureBank::open_stack_java_controlled(
        &[fixture.path.clone(), overlay.path.clone()],
        &LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(overridden
        .atlas()
        .unwrap()
        .entries
        .contains_key("minecraft:painting/pointer"));
    let slot = &overridden.atlas().unwrap().entries["minecraft:painting/pointer"];
    assert_eq!(
        overridden
            .atlas_pixel(slot.sample(0.5, 0.5), slot.page)
            .unwrap()[..3],
        [231, 17, 29]
    );
    let (_, solid) =
        painting_geometry::quads(&source, &overridden, overridden.atlas().unwrap(), true).unwrap();
    assert!(solid
        .iter()
        .all(|q| q.mode & crate::material_mode::FORCE_OPAQUE_ALPHA != 0));
    let mut bad = source.clone();
    bad.appearance
        .insert("variant".into(), Value::String("example:missing".into()));
    let mut marker = marker;
    marker.visual = Some(Arc::new(bad));
    let failed = Frame::build(
        entity_geometry::tests::snapshot(vec![marker]),
        &bank,
        Some(3955),
        true,
        &|| false,
    )
    .unwrap();
    assert_eq!((failed.rendered, failed.missing), (0, 1));
    assert!(failed
        .diagnostics
        .iter()
        .any(|d| d.contains("example:missing")));
}
pub(crate) fn complete() {
    references();
    resources_and_picking();
    actual_sources_and_packs();
    official_clients();
}

fn entity(visual: &EntityVisual) -> Value {
    let version = visual.source_version.unwrap();
    Value::Compound(
        [
            "id",
            "Pos",
            "TileX",
            "TileY",
            "TileZ",
            if version == 1343 { "Facing" } else { "facing" },
            if version == 1343 { "Motive" } else { "variant" },
        ]
        .into_iter()
        .map(|key| field(key, visual.source_field(key).unwrap()))
        .collect(),
    )
}
fn overlay(version: i32, original: &[u8], entities: Vec<Value>) -> Vec<u8> {
    let list = Value::List(List {
        element_type: 10,
        items: entities,
    });
    if version == 1343 {
        mcedit2_world::nbt_document::patch(
            original,
            &Operation::Replace {
                path: "/Level/Entities".into(),
                value: list,
            },
        )
        .unwrap()
    } else {
        crate::metadata_fixture::nbt(vec![
            field("DataVersion", Value::Int(version)),
            field("Position", Value::IntArray(vec![0, 0])),
            field("Entities", list),
        ])
    }
}
fn actual_sources_and_packs() {
    let native_assets = Fixture::new();
    let bytes = png();
    for path in ["pointer", "kebab", "back"] {
        native_assets.put(
            &format!("assets/minecraft/textures/painting/{path}.png"),
            &bytes,
        );
    }
    let native_bank = native_assets.open();
    for version in [1343, 3465, 3955] {
        let mut fixture = crate::metadata_fixture::Fixture::new(version);
        crate::object_fixture::populate(&mut fixture);
        let catalog = Catalog::builtin(version).unwrap();
        let source = visual(
            version,
            "minecraft:pointer",
            &catalog.0["minecraft:pointer"],
            1,
            [2, 2, 2],
        );
        let main = mcedit2_world::RegionIndex::open(fixture.root.join("region/r.0.0.mca"))
            .unwrap()
            .read_chunk_at(0, 0)
            .unwrap()
            .unwrap();
        let raw = overlay(version, &main, vec![entity(&source)]);
        let file = if version == 1343 {
            "region/r.0.0.mca"
        } else {
            "entities/r.0.0.mca"
        };
        crate::object_fixture::write_region(&fixture.root.join(file), &[((0, 0), raw)]);
        let before = fixture.file(file);
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let mut reader =
            mcedit2_world::scene_annotations::Reader::new(&fixture.world, "minecraft:overworld")
                .unwrap();
        let annotations = reader.read(0, 0).unwrap();
        let marker = annotations
            .markers
            .iter()
            .find(|m| m.id.as_deref() == Some("minecraft:painting"))
            .unwrap();
        let actual = Source::read(marker.visual.as_deref().unwrap(), &catalog).unwrap();
        assert_eq!(marker.position, actual.center());
        assert!(marker.geometry.is_some());
        assert_eq!(marker.kind, Kind::Item);
        assert_eq!(
            marker.visual.as_ref().unwrap().source_version,
            Some(version)
        );
        let frame = Frame::build(
            entity_geometry::tests::snapshot(vec![marker.clone()]),
            &native_bank,
            Some(3955),
            true,
            &|| false,
        )
        .unwrap();
        assert_eq!((frame.rendered, frame.missing), (1, 0));
        let entities = [mcedit2_world::clipboard::OpaqueEntity {
            position: [-18.5, 70., -16.5],
            block_position: None,
            compound: entity(&source).payload().unwrap(),
        }];
        let blocks = [mcedit2_world::editing::parse_state("minecraft:air").unwrap()];
        let volume = crate::pending_import::Volume {
            size: [1; 3],
            offset: [0; 3],
            blocks: &blocks,
            mask: None,
            block_entities: &[],
            entities: &entities,
            biomes: None,
            numeric_biomes: None,
            data_version: Some(version),
            painting_variants: &BTreeMap::new(),
            metadata: false,
            map_world: None,
            item_environment: None,
        };
        let scene = crate::textures::java_obj_tests::scene(Arc::clone(&native_bank));
        let preview = crate::pending_import::build_geometry_with_budget(
            &volume,
            [0; 3],
            &scene,
            false,
            crate::language::Language::English,
            Default::default(),
            &|| false,
        )
        .unwrap();
        assert!(
            preview.diagnostics.is_empty(),
            "{version}: {:?}",
            preview.diagnostics
        );
        let (center, expected) =
            painting_geometry::quads(&source, &native_bank, native_bank.atlas().unwrap(), true)
                .unwrap();
        let source_position = Source::read(&source, &catalog).unwrap().center();
        let delta = std::array::from_fn(|i| {
            (entities[0].position[i] + center[i] - source_position[i]) as f32
        });
        let actual: Vec<_> = preview
            .mesh
            .chunks
            .iter()
            .flat_map(|c| {
                c.quads()
                    .iter()
                    .map(|q| q.translated(c.origin.map(|n| n as f32)))
            })
            .collect();
        assert_eq!(
            actual,
            expected
                .into_iter()
                .map(|q| q.translated(delta))
                .collect::<Vec<_>>(),
            "{version} 挂画结构预览位移 / UV"
        );
        let changed = visual(
            version,
            "minecraft:kebab",
            &catalog.0["minecraft:kebab"],
            3,
            [3, 2, 3],
        );
        let staged = overlay(version, &main, vec![entity(&changed)]);
        let staged = reader
            .read_with_entity_overlay(0, 0, Some(&staged))
            .unwrap();
        let staged = staged
            .markers
            .iter()
            .find(|m| m.id.as_deref() == Some("minecraft:painting"))
            .unwrap();
        assert_ne!(staged.position, marker.position);
        assert_ne!(staged.sha256, marker.sha256);
        assert_eq!(fixture.file(file), before);
    }
    let mut fixture = crate::metadata_fixture::Fixture::new(3955);
    crate::object_fixture::populate(&mut fixture);
    let packfile = |pack: &str, file: &str, bytes: &[u8]| {
        let path = fixture.root.join("datapacks").join(pack).join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    packfile(
        "lower",
        "pack.mcmeta",
        br#"{"pack":{"pack_format":48,"description":"lower"}}"#,
    );
    for id in ["wide", "filtered"] {
        packfile(
            "lower",
            &format!("data/example/painting_variant/{id}.json"),
            br#"{"width":2,"height":2,"asset_id":"minecraft:pointer"}"#,
        );
    }
    packfile("upper","pack.mcmeta",br#"{"pack":{"pack_format":48,"description":"upper"},"filter":{"block":[{"namespace":"example","path":"painting_variant/(wide|filtered)\\.json"}]},"overlays":{"entries":[{"formats":48,"directory":"v48"}]}}"#);
    packfile(
        "upper",
        "data/example/painting_variant/wide.json",
        br#"{"width":3,"height":2,"asset_id":"custom:gallery/pointer"}"#,
    );
    packfile(
        "upper",
        "v48/data/example/painting_variant/wide.json",
        br#"{"width":4,"height":3,"asset_id":"custom:gallery/pointer"}"#,
    );
    packfile("disabled", "pack.mcmeta", b"broken");
    let mut document = mcedit2_world::nbt_file::FileDocument::open(
        &fixture.root,
        &fixture.root.join("level.dat"),
        false,
    )
    .unwrap()
    .document;
    document
        .apply(&Operation::InsertField {
            path: "/Data".into(),
            name: "DataPacks".into(),
            value: Value::Compound(vec![field(
                "Enabled",
                Value::List(List {
                    element_type: 8,
                    items: ["vanilla", "file/lower", "file/upper"]
                        .into_iter()
                        .map(|s| Value::String(s.into()))
                        .collect(),
                }),
            )]),
        })
        .unwrap();
    std::fs::write(
        fixture.root.join("level.dat"),
        crate::metadata_fixture::gzip(document.bytes()),
    )
    .unwrap();
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
    let catalog = fixture.world.world_data.painting_catalog(3955).unwrap();
    assert!(Arc::ptr_eq(
        &catalog,
        &fixture.world.world_data.painting_catalog(3955).unwrap()
    ));
    assert!(!catalog.0.contains_key("example:filtered"));
    let art = &catalog.0["example:wide"];
    assert_eq!((art.width, art.height), (4, 3));
    assert_eq!(
        fixture
            .world
            .world_data
            .painting_catalog(3465)
            .unwrap()
            .0
            .len(),
        30
    );
    let assets = Fixture::new();
    let bytes = png();
    assets.put(
        "assets/custom/textures/painting/gallery/pointer.png",
        &bytes,
    );
    assets.put("assets/minecraft/textures/painting/back.png", &bytes);
    // 普通资源包的 data/ 不能改变源世界挂画尺寸。
    assets.put(
        "data/example/painting_variant/wide.json",
        br#"{"width":1,"height":1,"asset_id":"minecraft:kebab"}"#,
    );
    let mut bank = assets.open().as_ref().clone();
    bank.apply_world_data(&fixture.world, &LoadControl::default())
        .unwrap();
    let source = visual(3955, "example:wide", art, 0, [2, 2, 2]);
    let (_, quads) = painting_geometry::quads(&source, &bank, bank.atlas().unwrap(), true).unwrap();
    assert_eq!(quads.len(), 72);
    frozen_preview(&bank, art);
    let raw = overlay(3955, &[], vec![entity(&source)]);
    crate::object_fixture::write_region(&fixture.root.join("entities/r.0.0.mca"), &[((0, 0), raw)]);
    let mut reader =
        mcedit2_world::scene_annotations::Reader::new(&fixture.world, "minecraft:overworld")
            .unwrap();
    let annotations = reader.read(0, 0).unwrap();
    let snapshot = Arc::new(Snapshot::build(
        (0, 0),
        1,
        std::iter::once(Arc::new(annotations)),
        None,
    ));
    let frame = Frame::build(snapshot, &bank, Some(3955), true, &|| false).unwrap();
    assert_eq!(
        (frame.rendered, frame.missing),
        (1, 0),
        "{:?}",
        frame.diagnostics
    );
    let mut inline = source.clone();
    inline.appearance.insert(
        "variant".into(),
        Value::Compound(vec![
            field("width", Value::Int(5)),
            field("height", Value::Int(6)),
            field("asset_id", Value::String("custom:gallery/pointer".into())),
        ]),
    );
    assert_eq!(Source::read(&inline, &catalog).unwrap().variant.width, 5);
    inline.appearance.insert("facing".into(), Value::Byte(4));
    assert!(Source::read(&inline, &catalog).is_err());
    let original = fixture.world.world_data.origin.clone();
    packfile(
        "upper",
        "v48/data/example/painting_variant/wide.json",
        br#"{"width":0,"height":3,"asset_id":"custom:gallery/pointer"}"#,
    );
    let changed = mcedit2_world::world_data::Snapshot::read(&fixture.root, Some(3955)).unwrap();
    assert_ne!(changed.origin, original);
    assert!(changed.painting_catalog(3955).is_err());
    assert!(bank
        .apply_world_data(&fixture.world, &LoadControl::default())
        .is_err());
}
fn frozen_preview(bank: &TextureBank, art: &Variant) {
    for (width, height) in [(2, 5), (7, 3)] {
        let id = format!("generic:gallery_{width}x{height}");
        let mut definition = art.clone();
        definition.width = width;
        definition.height = height;
        let source = visual(3955, &id, &definition, 1, [-19, 72, -17]);
        let definitions = BTreeMap::from([(id, definition)]);
        // 资源包只有实际图像；源结构冻结的定义决定任意命名空间与尺寸。
        let bank = bank.clone();
        let expected = painting_geometry::quads_bound(
            &source,
            &bank,
            bank.atlas().unwrap(),
            true,
            &definitions,
        )
        .unwrap();
        assert_eq!(
            expected.1.len(),
            usize::from(width) * usize::from(height) * 6
        );
        let entities = [mcedit2_world::clipboard::OpaqueEntity {
            position: [1., 2., 3.],
            block_position: None,
            compound: entity(&source).payload().unwrap(),
        }];
        let blocks = [mcedit2_world::editing::parse_state("minecraft:air").unwrap()];
        let volume = crate::pending_import::Volume {
            size: [1; 3],
            offset: [0; 3],
            blocks: &blocks,
            mask: None,
            block_entities: &[],
            entities: &entities,
            biomes: None,
            numeric_biomes: None,
            data_version: Some(3955),
            painting_variants: &definitions,
            metadata: false,
            map_world: None,
            item_environment: None,
        };
        let scene = crate::textures::java_obj_tests::scene(Arc::new(bank));
        let preview = crate::pending_import::build_geometry_with_budget(
            &volume,
            [32, 0, -32],
            &scene,
            false,
            crate::language::Language::English,
            Default::default(),
            &|| false,
        )
        .unwrap();
        assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);
        let actual: Vec<_> = preview
            .mesh
            .chunks
            .iter()
            .flat_map(|c| {
                c.quads()
                    .iter()
                    .flat_map(|q| {
                        q.xyz.map(|p| {
                            std::array::from_fn::<_, 3, _>(|i| {
                                f64::from(p[i]) + f64::from(c.origin[i])
                            })
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let source_position = Source::read_bound(&source, &Catalog(BTreeMap::new()), &definitions)
            .unwrap()
            .center();
        let delta: [f64; 3] = std::array::from_fn(|i| {
            [32., 0., -32.][i] + entities[0].position[i] + expected.0[i] - source_position[i]
        });
        let expected: Vec<_> = expected
            .1
            .iter()
            .flat_map(|q| {
                q.xyz
                    .map(|p| std::array::from_fn::<_, 3, _>(|i| f64::from(p[i]) + delta[i]))
            })
            .collect();
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual
            .into_iter()
            .flatten()
            .zip(expected.into_iter().flatten())
        {
            assert!((a - b).abs() < 0.00001);
        }
    }
}
fn official_clients() {
    let Some(paths) = std::env::var_os("MCEDIT2_ITEM_CLIENTS") else {
        return;
    };
    for path in std::env::split_paths(&paths) {
        let bank = TextureBank::open_stack_java_controlled(
            &[path],
            &LoadControl {
                all_png: false,
                ..Default::default()
            },
        )
        .unwrap();
        let version = bank.item_data_version().unwrap();
        let catalog = Catalog::builtin(version).unwrap();
        for (id, art) in &catalog.0 {
            let visual = visual(version, id, art, 2, [2, 2, 2]);
            let (_, quads) =
                painting_geometry::quads(&visual, &bank, bank.atlas().unwrap(), true).unwrap();
            assert_eq!(
                quads.len(),
                usize::from(art.width) * usize::from(art.height) * 6
            );
            // 与现有实体相同的 CPU 光栅入口消费完整图集和实际材质，而非仅检查 PNG 文件名。
            for side in [1_f32, -1.] {
                let vertices: Vec<_> = quads
                    .iter()
                    .flat_map(|q| {
                        [0, 1, 2, 0, 2, 3].map(|i| {
                            let [x, y, z] = q.xyz[i];
                            crate::gpu_mesh::Vertex {
                                xy: [side * x / 4., y / 4.],
                                depth: 0.5 + side * z / 4.,
                                rgba: q.rgba[i],
                                uv: q.uv[i],
                                mode: q.mode,
                                xyz: q.xyz[i],
                                world_mode: 0,
                                atlas_page: q.atlas_page,
                                light: crate::lighting::FULLBRIGHT,
                            }
                        })
                    })
                    .collect();
                let raster = crate::canvas_raster::render(
                    &vertices,
                    bank.atlas().map(Arc::as_ref),
                    iced::Size::new(96., 96.),
                    crate::gpu_mesh::Quality::Fast,
                    &|| false,
                )
                .unwrap()
                .unwrap();
                assert!(
                    raster.rgba.chunks_exact(4).any(|p| p[3] != 0),
                    "{version} {id} {side}"
                );
            }
        }
        eprintln!(
            "实际客户端 {version}：{} 幅挂画画面 / 背板光栅化通过",
            catalog.0.len()
        );
    }
}
