//! 同一个渲染类别内核对跨版本原生模型、幼年变换和实际 NBT 来源。
use super::*;
use serde::Deserialize;

type Faces = BTreeMap<String, BTreeMap<usize, Vec<[f32; 5]>>>;
#[derive(Deserialize)]
struct Reference {
    version: String,
    reference: BTreeMap<String, Faces>,
    #[serde(default)]
    shared_reference: Vec<String>,
}
fn catalog_references() {
    let base: Reference = serde_json::from_str(include_str!(
        "../../mcedit2-world/tests/fixtures/entity-layer-reference-1.21.1.json"
    ))
    .unwrap();
    for (version, json, count) in [
        (
            1343,
            include_str!("../../mcedit2-world/tests/fixtures/entity-layer-reference-1.12.2.json"),
            24,
        ),
        (
            3465,
            include_str!("../../mcedit2-world/tests/fixtures/entity-layer-reference-1.20.1.json"),
            204,
        ),
        (
            3955,
            include_str!("../../mcedit2-world/tests/fixtures/entity-layer-reference-1.21.1.json"),
            213,
        ),
    ] {
        let mut reference: Reference = serde_json::from_str(json).unwrap();
        for shared in &reference.shared_reference {
            assert!(reference
                .reference
                .insert(shared.clone(), base.reference[shared].clone())
                .is_none());
        }
        let catalog = Catalog::for_data_version(Some(version)).unwrap();
        assert_eq!(catalog.version, reference.version);
        assert_eq!(catalog.layers().count(), count);
        // 旧版参照只覆盖早期 15 层；其余真实目录层仍须能生成几何。
        let layers = catalog.layers().collect::<std::collections::BTreeSet<_>>();
        assert!(reference
            .reference
            .keys()
            .all(|layer| layers.contains(layer.as_str())));
        if version == 1343 {
            assert_eq!(reference.reference.len(), 15);
            for layer in layers
                .iter()
                .filter(|layer| !reference.reference.contains_key(**layer))
            {
                assert!(
                    !catalog.bake(layer, &BTreeMap::new()).unwrap().is_empty(),
                    "{layer}"
                );
            }
        } else {
            assert_eq!(reference.reference.len(), layers.len());
        }
        let mut maximum = 0_f32;
        for (layer, expected) in &reference.reference {
            let mut actual: Faces = BTreeMap::new();
            for q in catalog.bake(layer, &BTreeMap::new()).unwrap() {
                actual
                    .entry(q.part)
                    .or_default()
                    .entry(q.cube)
                    .or_default()
                    .extend(q.corners);
            }
            assert_eq!(
                actual.keys().collect::<Vec<_>>(),
                expected.keys().collect::<Vec<_>>(),
                "{version}: {layer}"
            );
            for (part, cubes) in expected {
                assert_eq!(actual[part].len(), cubes.len(), "{layer} {part}");
                for (cube, vertices) in cubes {
                    assert_eq!(actual[part][cube].len(), vertices.len());
                    for (a, e) in actual[part][cube].iter().zip(vertices) {
                        assert_eq!(a[3..], e[3..], "{version}: {layer}/{part}/{cube} UV");
                        for axis in 0..3 {
                            maximum = maximum.max((a[axis] - e[axis]).abs());
                        }
                    }
                }
            }
        }
        assert!(maximum < 0.00002, "{version}: 模型角点最大偏差 {maximum}");
        eprintln!("原生 {version} 模型：{count} 层，角点最大偏差 {maximum}");
    }
    assert!(Catalog::for_data_version(None).is_err());
    assert!(Catalog::for_data_version(Some(4000)).is_err());
}
#[derive(Deserialize)]
struct YoungReferences {
    version: String,
    cases: Vec<YoungCase>,
}
#[derive(Deserialize)]
struct YoungCase {
    id: String,
    layer: String,
    young: bool,
    pitch: f32,
    yaw: f32,
    vertices: Vec<[f32; 5]>,
}
fn young_references(bank: &TextureBank) {
    for (version, json) in [
        (
            3465,
            include_str!("../../mcedit2-world/tests/fixtures/young-model-1.20.1.json"),
        ),
        (
            3955,
            include_str!("../../mcedit2-world/tests/fixtures/young-model-1.21.1.json"),
        ),
    ] {
        let reference: YoungReferences = serde_json::from_str(json).unwrap();
        let catalog = Catalog::for_data_version(Some(version)).unwrap();
        assert_eq!(catalog.version, reference.version);
        assert_eq!(reference.cases.len(), 22);
        for case in reference.cases {
            let texture = format!("minecraft:entity/{0}/{0}", case.id);
            let mut poses = BTreeMap::from([(
                "/head".into(),
                PartPose {
                    rotation: Some([case.pitch.to_radians(), 0., 0.]),
                    ..Default::default()
                },
            )]);
            if matches!(case.id.as_str(), "zombie" | "skeleton") {
                poses.insert(
                    "/hat".into(),
                    PartPose {
                        rotation: Some([case.pitch.to_radians(), 0., 0.]),
                        ..Default::default()
                    },
                );
            }
            if case.id == "spider" {
                spider_stance(&mut poses);
            }
            let groups = if case.young {
                Some(match case.id.as_str() {
                    "cow" => npc::Groups {
                        head_parts: &["/head"],
                        head_scale: 1.,
                        head_translate: [0., 0.625, 0.25],
                        body_scale: 0.5,
                        body_translate: [0., 1.5, 0.],
                    },
                    "pig" => npc::Groups {
                        head_parts: &["/head"],
                        head_scale: 1.,
                        head_translate: [0., 0.25, 0.25],
                        body_scale: 0.5,
                        body_translate: [0., 1.5, 0.],
                    },
                    "sheep" => npc::Groups {
                        head_parts: &["/head"],
                        head_scale: 1.,
                        head_translate: [0., 0.5, 0.25],
                        body_scale: 0.5,
                        body_translate: [0., 1.5, 0.],
                    },
                    "zombie" | "skeleton" => npc::Groups {
                        head_parts: &["/head"],
                        head_scale: 0.75,
                        head_translate: [0., 1., 0.],
                        body_scale: 0.5,
                        body_translate: [0., 1.5, 0.],
                    },
                    _ => panic!("原生参照出现未声明模型"),
                })
            } else {
                None
            };
            let quads = npc::render(
                catalog,
                bank,
                &case.layer,
                &texture,
                &poses,
                npc::RenderStyle {
                    yaw: case.yaw,
                    groups,
                    outer: 1.,
                    tint: [1.; 4],
                    ordered: false,
                },
            )
            .unwrap();
            let mut actual: Vec<_> = quads
                .iter()
                .flat_map(|q| q.xyz.into_iter().zip(q.uv))
                .collect();
            assert_eq!(actual.len(), case.vertices.len(), "{version} {}", case.id);
            let atlas_uv = &bank.atlas().unwrap().entries[&texture];
            for expected in case.vertices {
                let uv = atlas_uv.sample(expected[3], expected[4]);
                let index = actual
                    .iter()
                    .position(|(xyz, u)| {
                        (0..3).all(|i| (xyz[i] - expected[i]).abs() < 0.0001)
                            && (0..2).all(|i| (u[i] - uv[i]).abs() < 0.0001)
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "{version} {} young={} pitch={}: 原生顶点未匹配 {expected:?}",
                            case.id, case.young, case.pitch
                        )
                    });
                actual.swap_remove(index);
            }
        }
        eprintln!("原生 {version} 成年 / 幼年 / 俯仰渲染：22 组实际客户端顶点全部匹配");
    }
}
fn visual(version: i32, appearance: BTreeMap<String, Value>) -> EntityVisual {
    let marker = tests::actor("minecraft:pig", [0.; 3], 0, appearance);
    let mut visual = Arc::unwrap_or_clone(marker.visual.unwrap());
    visual.source_version = Some(version);
    visual.rotation = Some([37.25, 11.]);
    visual
}
fn variants(version: i32) -> Vec<(String, Kind, BTreeMap<String, Value>)> {
    let mut out = Vec::new();
    for id in [
        "zombie",
        "skeleton",
        "creeper",
        "spider",
        "cow",
        "pig",
        "sheep",
        "armor_stand",
        "shulker",
        if version == 1343 {
            "zombie_pigman"
        } else {
            "zombified_piglin"
        },
    ] {
        out.push((format!("minecraft:{id}"), Kind::Entity, BTreeMap::new()));
    }
    for id in [
        "zombie",
        "cow",
        "pig",
        "sheep",
        if version == 1343 {
            "zombie_pigman"
        } else {
            "zombified_piglin"
        },
    ] {
        let mut appearance = BTreeMap::from([(
            if matches!(id, "zombie" | "zombie_pigman" | "zombified_piglin") {
                "IsBaby"
            } else {
                "Age"
            }
            .into(),
            if matches!(id, "zombie" | "zombie_pigman" | "zombified_piglin") {
                Value::Byte(1)
            } else {
                Value::Int(-24000)
            },
        )]);
        if id == "pig" {
            appearance.insert("Saddle".into(), Value::Byte(1));
        }
        out.push((format!("minecraft:{id}"), Kind::Entity, appearance));
    }
    for hash in 0..if version == 1343 { 2 } else { 18 } {
        out.push((
            "mcedit2:player".into(),
            Kind::Player,
            BTreeMap::from([
                (
                    "mcedit2:player_uuid".into(),
                    Value::IntArray(vec![0, 0, 0, hash]),
                ),
                ("playerGameType".into(), Value::Int(0)),
                ("ShoulderEntityLeft".into(), Value::Compound(Vec::new())),
                ("ShoulderEntityRight".into(), Value::Compound(Vec::new())),
            ]),
        ));
    }
    if version == 1343 {
        for profession in 0..=5 {
            out.push((
                "minecraft:villager".into(),
                Kind::Entity,
                BTreeMap::from([("Profession".into(), Value::Int(profession))]),
            ));
        }
    } else {
        use mcedit2_world::nbt_document::Field;
        out.push((
            "minecraft:villager".into(),
            Kind::Entity,
            BTreeMap::from([(
                "VillagerData".into(),
                Value::Compound(vec![
                    Field {
                        name: "type".into(),
                        value: Value::String("minecraft:plains".into()),
                    },
                    Field {
                        name: "profession".into(),
                        value: Value::String("minecraft:farmer".into()),
                    },
                    Field {
                        name: "level".into(),
                        value: Value::Int(1),
                    },
                ]),
            )]),
        ));
        out.push((
            "minecraft:frog".into(),
            Kind::Entity,
            BTreeMap::from([(
                "variant".into(),
                Value::String("minecraft:temperate".into()),
            )]),
        ));
    }
    for color in 0..16 {
        out.push((
            "minecraft:shulker".into(),
            Kind::Entity,
            BTreeMap::from([
                ("Color".into(), Value::Byte(color)),
                ("Peek".into(), Value::Byte(100)),
                ("AttachFace".into(), Value::Byte(color % 6)),
            ]),
        ));
        out.push((
            "minecraft:sheep".into(),
            Kind::Entity,
            BTreeMap::from([
                ("Color".into(), Value::Byte(color)),
                ("Age".into(), Value::Int(-100)),
            ]),
        ));
    }
    out.push((
        "minecraft:sheep".into(),
        Kind::Entity,
        BTreeMap::from([("Sheared".into(), Value::Byte(1))]),
    ));
    out.push((
        "minecraft:armor_stand".into(),
        Kind::Entity,
        BTreeMap::from([
            ("Small".into(), Value::Byte(1)),
            ("ShowArms".into(), Value::Byte(1)),
        ]),
    ));
    out
}
fn native_bindings(bank: &TextureBank) {
    for version in [1343, 3465, 3955] {
        let catalog = Catalog::for_data_version(Some(version)).unwrap();
        let variants = variants(version);
        for (id, kind, appearance) in &variants {
            let quads = compile_visual(
                catalog,
                bank,
                *kind,
                Some(id),
                &visual(version, appearance.clone()),
                None,
                crate::language::Language::English,
            )
            .unwrap_or_else(|error| panic!("{version} {id} {appearance:?}: {error}"));
            assert!(!quads.is_empty(), "{version} {id}");
            assert!(quads.iter().all(|q| q
                .xyz
                .iter()
                .flatten()
                .chain(q.uv.iter().flatten())
                .all(|n| n.is_finite())));
            let marker = tests::actor(id, [0., 0., 0.], 0, appearance.clone());
            let mut marker = marker;
            marker.kind = *kind;
            marker.visual = Some(Arc::new(visual(version, appearance.clone())));
            let snapshot = tests::snapshot(vec![marker]);
            let frame = Frame::build(
                snapshot,
                bank,
                Some(if version == 3955 { 1343 } else { 3955 }),
                true,
                &|| false,
            )
            .unwrap();
            assert_eq!(
                (frame.rendered, frame.missing),
                (1, 0),
                "{version} {id}: {:?}",
                frame.diagnostics
            );
            assert_eq!(frame.entities.values().map(Vec::len).sum::<usize>(), 1);
        }
        eprintln!(
            "DataVersion {version}：{} 组原生外观及跨世界版本绑定通过",
            variants.len()
        );
    }
    let mut marker = tests::actor("minecraft:pig", [0., 0., 0.], 0, BTreeMap::new());
    Arc::make_mut(marker.visual.as_mut().unwrap()).source_version = None;
    let frame = Frame::build(
        tests::snapshot(vec![marker]),
        bank,
        Some(3955),
        true,
        &|| false,
    )
    .unwrap();
    assert_eq!((frame.rendered, frame.missing), (0, 1));
    assert!(frame
        .diagnostics
        .iter()
        .any(|error| error.contains("DataVersion None")));
}
fn actual_sources(bank: &TextureBank) {
    use crate::metadata_fixture::{field, gzip, nbt, player, Fixture};
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        crate::object_fixture::populate(&mut fixture);
        let catalog =
            mcedit2_world::entity_catalog::inspect(&fixture.world, "minecraft:overworld", 0, 0)
                .unwrap();
        assert!(!catalog.entities.is_empty());
        for entity in &catalog.entities {
            assert_eq!(
                entity.visual.as_ref().unwrap().source_version,
                Some(version)
            );
        }
        let mut reader =
            mcedit2_world::scene_annotations::Reader::new(&fixture.world, "minecraft:overworld")
                .unwrap();
        let annotations = Arc::new(reader.read(0, 0).unwrap());
        let snapshot = Arc::new(Snapshot::build(
            (0, 0),
            1,
            std::iter::once(annotations),
            None,
        ));
        let frame = Frame::build(snapshot.clone(), bank, Some(3955), true, &|| false).unwrap();
        let pig = snapshot
            .markers
            .iter()
            .find(|m| m.marker().id.as_deref() == Some("minecraft:pig"))
            .unwrap();
        assert_eq!(
            pig.marker().visual.as_ref().unwrap().source_version,
            Some(version)
        );
        assert!(frame.entities.values().flatten().any(|m| m.same(pig)));
        assert!(frame.rendered >= 1, "{version}: {:?}", frame.diagnostics);
        // 真正的结构放置入口使用结构自身版本，不再只开放 1.21.1。
        let Value::Compound(mut entity_fields) = crate::object_fixture::entity(version, 1, 2.)
        else {
            panic!("需要实体 Compound")
        };
        entity_fields.retain(|field| field.name != "Passengers");
        let entities = [mcedit2_world::clipboard::OpaqueEntity {
            position: [0.5, 0., 0.5],
            block_position: None,
            compound: Value::Compound(entity_fields).payload().unwrap(),
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
        let scene = crate::textures::java_obj_tests::scene(Arc::new(bank.clone()));
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
        let expected = compile_visual(
            Catalog::for_data_version(Some(version)).unwrap(),
            bank,
            Kind::Entity,
            Some("minecraft:pig"),
            pig.marker().visual.as_ref().unwrap(),
            None,
            crate::language::Language::English,
        )
        .unwrap();
        let actual = preview
            .mesh
            .chunks
            .iter()
            .flat_map(|chunk| chunk.quads().iter())
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            expected.iter().collect::<Vec<_>>(),
            "{version} 结构预览与原始实体的静态面 / UV"
        );
        let players = mcedit2_world::player_catalog::load_all(&fixture.world, &|| false).unwrap();
        assert!(players.diagnostics.is_empty(), "{:?}", players.diagnostics);
        assert!(!players.players.is_empty());
        for player in players.players {
            assert_eq!(player.visual.unwrap().source_version, Some(version));
        }
        // 嵌入玩家省略自身版本时，使用同文件 Data 的版本；独立文件没有父版本。
        let mut embedded = player(version, 3, "minecraft:overworld");
        embedded.retain(|field| field.name != "DataVersion");
        std::fs::write(
            fixture.root.join("level.dat"),
            gzip(&nbt(vec![field(
                "Data",
                Value::Compound(vec![
                    field("DataVersion", Value::Int(version)),
                    field("Player", Value::Compound(embedded)),
                ]),
            )])),
        )
        .unwrap();
        let mut external = player(
            if version == 3955 { 3465 } else { 3955 },
            2,
            "minecraft:the_nether",
        );
        std::fs::write(
            fixture
                .root
                .join("playerdata/00000000-0000-0000-0000-000000000002.dat"),
            gzip(&nbt(external.clone())),
        )
        .unwrap();
        let loaded = mcedit2_world::player_catalog::load_all(&fixture.world, &|| false).unwrap();
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        let embedded = loaded
            .players
            .iter()
            .find(|p| p.uuid == Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3]))
            .unwrap();
        assert_eq!(
            embedded.visual.as_ref().unwrap().source_version,
            Some(version)
        );
        let external_player = loaded
            .players
            .iter()
            .find(|p| p.uuid == Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]))
            .unwrap();
        assert_eq!(
            external_player.visual.as_ref().unwrap().source_version,
            Some(if version == 3955 { 3465 } else { 3955 })
        );
        external.retain(|field| field.name != "DataVersion");
        std::fs::write(
            fixture
                .root
                .join("playerdata/00000000-0000-0000-0000-000000000002.dat"),
            gzip(&nbt(external)),
        )
        .unwrap();
        let loaded = mcedit2_world::player_catalog::load_all(&fixture.world, &|| false).unwrap();
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        assert_eq!(
            loaded
                .players
                .iter()
                .find(|p| p.uuid == Some([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]))
                .unwrap()
                .visual
                .as_ref()
                .unwrap()
                .source_version,
            None
        );
    }
}
fn official_clients() {
    let Some(paths) = std::env::var_os("MCEDIT2_ITEM_CLIENTS") else {
        return;
    };
    let output = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/entity-models-verification");
    std::fs::create_dir_all(&output).unwrap();
    for client in std::env::split_paths(&paths) {
        let bank = TextureBank::open_stack_java_controlled(
            &[client],
            &crate::textures::LoadControl {
                all_png: false,
                ..Default::default()
            },
        )
        .unwrap();
        let version = bank.item_data_version().unwrap();
        let catalog = Catalog::for_data_version(Some(version)).unwrap();
        let variants = variants(version);
        let mut sheet = image::RgbaImage::new(128 * 10, (variants.len().div_ceil(10) * 128) as u32);
        let mut manifest = Vec::new();
        for (index, (id, kind, appearance)) in variants.iter().enumerate() {
            let quads = compile_visual(
                catalog,
                &bank,
                *kind,
                Some(id),
                &visual(version, appearance.clone()),
                None,
                crate::language::Language::English,
            )
            .unwrap_or_else(|error| panic!("准确客户端 {version} {id} {appearance:?}: {error}"));
            let vertices: Vec<_> = quads
                .iter()
                .flat_map(|q| {
                    [0, 1, 2, 0, 2, 3].map(|i| {
                        let [x, y, z] = q.xyz[i];
                        let (px, py, pz) = (
                            0.866 * x + 0.5 * z,
                            0.95 * (y - 1.) + 0.312 * (0.5 * x - 0.866 * z),
                            0.95 * (0.5 * x - 0.866 * z) - 0.312 * (y - 1.),
                        );
                        crate::gpu_mesh::Vertex {
                            xy: [px * 0.75, py * 0.75],
                            depth: 0.5 + pz / 16.,
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
                iced::Size::new(128., 128.),
                crate::gpu_mesh::Quality::Fast,
                &|| false,
            )
            .unwrap()
            .unwrap();
            assert!(
                raster.rgba.chunks_exact(4).any(|p| p[3] != 0),
                "{version} {id} 没有可见像素"
            );
            let image = image::RgbaImage::from_raw(128, 128, raster.rgba).unwrap();
            image::imageops::replace(
                &mut sheet,
                &image,
                (index % 10 * 128) as i64,
                (index / 10 * 128) as i64,
            );
            manifest.push(serde_json::json!({"index":index,"id":id,"kind":format!("{kind:?}"),"appearance":appearance,"quads":quads.len()}));
        }
        sheet
            .save(output.join(format!("entities-{version}.png")))
            .unwrap();
        std::fs::write(
            output.join(format!("entities-{version}.json")),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        eprintln!(
            "准确客户端 {version}：{} 组实体实际 PNG 与 Canvas 光栅化通过",
            variants.len()
        );
    }
}
pub(super) fn run() {
    catalog_references();
    let (_fixture, bank) = tests::assets();
    young_references(&bank);
    native_bindings(&bank);
    actual_sources(&bank);
    official_clients();
}
