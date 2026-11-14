//! 三版原生绘制计划、真实 Region、资源覆盖、源拾取和放置预览；并入已有类别。
use super::*;
use crate::{metadata_fixture::field, textures::java_obj_tests::Fixture};
use mcedit2_world::{
    editing::parse_state,
    nbt_document::{Document, List, Operation, Value},
    piston::{self, Source},
    sections::State,
};
use std::{fs, io::Read};

fn rows(version: i32) -> Vec<serde_json::Value> {
    let bytes: &[u8] = match version {
        1343 => include_bytes!("../../mcedit2-world/tests/fixtures/piston-render-1.12.2.json.gz"),
        3465 => include_bytes!("../../mcedit2-world/tests/fixtures/piston-render-1.20.1.json.gz"),
        3955 => include_bytes!("../../mcedit2-world/tests/fixtures/piston-render-1.21.1.json.gz"),
        _ => unreachable!(),
    };
    let mut text = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut text)
        .unwrap();
    serde_json::from_slice(&text).unwrap()
}
fn root_state(version: i32) -> State {
    parse_state(if version == 1343 {
        "numeric:36[data=0]"
    } else {
        "minecraft:moving_piston[facing=north,type=normal]"
    })
    .unwrap()
}
fn payload(row: &serde_json::Value) -> Vec<u8> {
    row["nbt"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i8 as u8)
        .collect()
}
fn source(row: &serde_json::Value, version: i32) -> Source {
    piston::parse(&payload(row), &root_state(version), Some(version)).unwrap()
}
fn png() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([170, 100, 40, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}
fn assets(version: i32) -> Fixture {
    let fixture = frame_tests::assets(version);
    let docs: serde_json::Value = serde_json::from_str(match version {
        1343 => include_str!("../../mcedit2-world/tests/fixtures/piston-models-1.12.2.json"),
        3465 => include_str!("../../mcedit2-world/tests/fixtures/piston-models-1.20.1.json"),
        3955 => include_str!("../../mcedit2-world/tests/fixtures/piston-models-1.21.1.json"),
        _ => unreachable!(),
    })
    .unwrap();
    let png = png();
    for (path, doc) in docs.as_object().unwrap() {
        fixture.put(path, &serde_json::to_vec(doc).unwrap());
        if let Some(textures) = doc["textures"].as_object() {
            for id in textures
                .values()
                .filter_map(|v| v.as_str())
                .filter(|v| !v.starts_with('#'))
            {
                let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
                fixture.put(&format!("assets/{ns}/textures/{path}.png"), &png);
            }
        }
    }
    let grass = if version == 1343 {
        "grass"
    } else {
        "grass_block"
    };
    fixture.put(
        &format!("assets/minecraft/blockstates/{grass}.json"),
        &serde_json::to_vec(
            &serde_json::json!({"variants":{"":{"model":format!("minecraft:block/{grass}")}}}),
        )
        .unwrap(),
    );
    fixture.put(&format!("assets/minecraft/models/block/{grass}.json"),
        &serde_json::to_vec(&serde_json::json!({"textures":{"all":"minecraft:block/stone"},
            "elements":[{"from":[-4,0,0],"to":[20,16,16],"faces":{"up":{"texture":"#all","tintindex":0}}}]})).unwrap());
    if version == 1343 {
        fixture.put("assets/mcedit2/biomes/numeric.json", &serde_json::to_vec(&serde_json::json!({
            "format":1,"source":"移动方块颜色夹具","versions":{"minimum":1343,"maximum":1343,"unversioned":false},"blend_radius":2,
            "entries":[{"id":1,"name":"红","temperature":0.5,"rainfall":0.5,"water_color":16777215,"grass_color":16711680},
            {"id":2,"name":"蓝","temperature":0.5,"rainfall":0.5,"water_color":16777215,"grass_color":255}]})).unwrap());
    } else {
        for (name, color) in [("plains", 16711680), ("desert", 255)] {
            fixture.put(
                &format!("data/minecraft/worldgen/biome/{name}.json"),
                &serde_json::to_vec(&serde_json::json!({"temperature":0.5,"downfall":0.5,
                    "effects":{"water_color":16777215,"grass_color":color,"foliage_color":color}}))
                .unwrap(),
            );
        }
    }
    if version == 1343 {
        use std::io::Write;
        let mut files = Vec::new();
        let mut directories = vec![fixture.path.clone()];
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    directories.push(entry.path());
                } else {
                    files.push(entry.path());
                }
            }
        }
        files.sort();
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for file in files {
            let name = file
                .strip_prefix(&fixture.path)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(&fs::read(file).unwrap()).unwrap();
        }
        fixture.put("1.12.2.jar", &archive.finish().unwrap().into_inner());
    }
    fixture
}
fn open(resources: &Fixture, version: i32) -> Arc<TextureBank> {
    if version == 1343 {
        Arc::new(
            TextureBank::open_stack_java_controlled(
                &[resources.path.join("1.12.2.jar")],
                &crate::textures::LoadControl {
                    all_png: false,
                    ..Default::default()
                },
            )
            .unwrap(),
        )
    } else {
        resources.open()
    }
}
fn native_plans(version: i32, bank: &TextureBank, rows: &[serde_json::Value]) {
    // 旧客户端独立保存的原始状态 -> 资源位置绑定，在测试开始时一次解码。
    let legacy: std::collections::HashMap<_, _> = if version == 1343 {
        let data: serde_json::Value =
            serde_json::from_str(mcedit2_world::legacy_assets::mapping("1.12").unwrap()).unwrap();
        data["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let raw = row[5].as_str().unwrap();
                let display = if let Some(path) = row[2].as_str() {
                    let variant = row[3].as_str().unwrap_or("");
                    parse_state(&format!(
                        "minecraft:{path}{}",
                        if variant.contains('=') {
                            format!("[{variant}]")
                        } else {
                            String::new()
                        }
                    ))
                    .unwrap()
                } else {
                    parse_state(raw).unwrap()
                };
                (
                    mcedit2_world::editing::render_state(&parse_state(raw).unwrap()),
                    display,
                )
            })
            .collect()
    } else {
        Default::default()
    };
    for row in rows {
        let parsed = source(row, version);
        let original = parsed.clone();
        let parts = parsed
            .render_parts(bank.render_state(&parsed.moved))
            .unwrap();
        let expected = row["parts"].as_array().unwrap();
        assert_eq!(parts.len(), expected.len(), "{version}: {parsed:?}");
        for (part, expected) in parts.iter().zip(expected) {
            let state = parse_state(expected["state"].as_str().unwrap()).unwrap();
            let state = legacy
                .get(&mcedit2_world::editing::render_state(&state))
                .unwrap_or(&state);
            assert_eq!(&part.state, state, "{version}: {parsed:?}");
            assert_eq!(part.cull, expected["cull"].as_bool().unwrap());
            for axis in 0..3 {
                assert!(
                    (f64::from(part.translation[axis])
                        - expected["translation"][axis].as_f64().unwrap())
                    .abs()
                        < 0.0000001,
                    "{parsed:?}"
                );
                assert_eq!(
                    [-19, 72, -17][axis] + part.sample_offset[axis],
                    expected["sample"][axis].as_i64().unwrap() as i32,
                    "{parsed:?}"
                );
            }
        }
        assert_eq!(parsed, original);
    }
    eprintln!("原生移动活塞绘制计划：{version}，{} 组通过", rows.len());
}
fn selected<'a>(rows: &'a [serde_json::Value], version: i32) -> &'a serde_json::Value {
    rows.iter()
        .find(|row| {
            let s = source(row, version);
            s.facing == 5
                && s.extending
                && !s.source
                && s.progress() == 0.5
                && bank_stone(&s.moved, version)
        })
        .unwrap()
}
fn bank_stone(state: &State, version: i32) -> bool {
    if version == 1343 {
        state.name == "numeric:1" && state.properties == vec![("data".into(), "0".into())]
    } else {
        state.name == "minecraft:stone"
    }
}
fn raw_chunk(version: i32, row: &serde_json::Value) -> Vec<u8> {
    use mcedit2_world::chunk_creation::{empty_chunk, BiomeSource, ChunkCreationProfile};
    let legacy = version == 1343;
    let profile = ChunkCreationProfile {
        data_version: version,
        min_y: if legacy { 0 } else { -64 },
        height: if legacy { 256 } else { 384 },
        has_skylight: true,
        dimension_source: "user:移动活塞来源".into(),
        initial_biome: if legacy {
            "numeric:1"
        } else {
            "minecraft:plains"
        }
        .into(),
        biome_source: if legacy {
            BiomeSource::LegacyNumeric {
                origin: "旧列".into(),
                id: 1,
            }
        } else {
            BiomeSource::UserSpecified {
                origin: "源夹具".into(),
                entries: ["minecraft:plains".into()].into_iter().collect(),
            }
        },
    };
    let mut doc = Document::new(empty_chunk(&profile, (-2, -2)).unwrap()).unwrap();
    let index = 8 * 256 + 15 * 16 + 13;
    let mut compound = vec![10, 0, 0];
    compound.extend(payload(row));
    let be = Document::new(compound).unwrap().inspect("").unwrap();
    let operations = if legacy {
        let mut blocks = vec![0; 4096];
        blocks[index] = 36;
        vec![
            Operation::Replace {
                path: "/Level/Sections/4/Blocks".into(),
                value: Value::ByteArray(blocks),
            },
            Operation::Replace {
                path: "/Level/TileEntities".into(),
                value: Value::List(List {
                    element_type: 10,
                    items: vec![be],
                }),
            },
        ]
    } else {
        let mut packed = vec![0_i64; 256];
        packed[index / 16] = 1_i64 << ((index % 16) * 4);
        vec![
            Operation::Replace {
                path: "/sections/8/block_states/palette".into(),
                value: Value::List(List {
                    element_type: 10,
                    items: vec![
                        Value::Compound(vec![field("Name", Value::String("minecraft:air".into()))]),
                        Value::Compound(vec![
                            field("Name", Value::String("minecraft:moving_piston".into())),
                            field(
                                "Properties",
                                Value::Compound(vec![
                                    field("facing", Value::String("north".into())),
                                    field("type", Value::String("normal".into())),
                                ]),
                            ),
                        ]),
                    ],
                }),
            },
            Operation::InsertField {
                path: "/sections/8/block_states".into(),
                name: "data".into(),
                value: Value::LongArray(packed),
            },
            Operation::Replace {
                path: "/block_entities".into(),
                value: Value::List(List {
                    element_type: 10,
                    items: vec![be],
                }),
            },
        ]
    };
    doc.apply_batch(&operations).unwrap();
    doc.bytes().to_vec()
}
fn snapshot_from_world(world: &mcedit2_world::WorldSummary, bank: &TextureBank) -> Arc<Snapshot> {
    let mut reader =
        mcedit2_world::scene_annotations::Reader::new(world, "minecraft:overworld").unwrap();
    reader.set_block_visual_numeric_states(bank.block_visual_numeric_states());
    let chunk = reader
        .read_with_layers_filtered(
            mcedit2_world::chunk_display::Request {
                chunk: (-2, -2),
                layers: Kind::BlockEntity.bit(),
                heightmap: mcedit2_world::chunk_display::Heightmap::Source,
            },
            None,
            None,
            false,
        )
        .unwrap();
    assert!(chunk.diagnostics.is_empty(), "{:?}", chunk.diagnostics);
    Arc::new(Snapshot::build(
        (0, 0),
        1,
        std::iter::once(Arc::new(chunk)),
        None,
    ))
}
fn frame(
    snapshot: Arc<Snapshot>,
    bank: &TextureBank,
    version: i32,
    old: Option<&Arc<Frame>>,
) -> Arc<Frame> {
    Arc::new(
        scene::assemble(
            snapshot,
            bank,
            Some(version),
            true,
            false,
            &|| false,
            None,
            false,
            &SkinResults::new(),
            &HeadResults::new(),
            crate::language::Language::English,
            old,
            None,
            None,
            crate::entity_visibility::Visibility::default(),
            None,
        )
        .unwrap(),
    )
}
fn integrated(version: i32, resources: &Fixture, bank: &TextureBank, row: &serde_json::Value) {
    let mut world = crate::metadata_fixture::Fixture::new(version);
    let path = world.root.join("region/r.-1.-1.mca");
    let raw = raw_chunk(version, row);
    crate::object_fixture::write_region(&path, &[((-2, -2), raw.clone())]);
    world.world = mcedit2_world::inspect_world(&world.root).unwrap();
    let original = fs::read(&path).unwrap();
    let snapshot = snapshot_from_world(&world.world, bank);
    let first = frame(Arc::clone(&snapshot), bank, version, None);
    assert_eq!((first.rendered, first.missing), (1, 0));
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    assert!(first.manages_block_visual([-19, 72, -17], &root_state(version), bank, &first.atlas));
    let chunk = &first.chunks[0];
    let bounds = chunk.picking.world_bounds().unwrap();
    assert!(bounds.0[0] < -19.5);
    let hit = chunk
        .picking
        .cast(
            crate::spectator::Camera {
                position: [-19.75, 72., -20.],
                yaw: 0.,
                pitch: 0.,
                zoom: 1.,
                perspective: true,
            },
            iced::Point::new(64., 64.),
            crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(128., 128.)),
            Some(&first.atlas),
            crate::gpu_mesh::Quality::Fast,
            f64::INFINITY,
        )
        .unwrap();
    let owner = first.marker((-2, -2), hit.group_index).unwrap().marker();
    assert_eq!(owner.position, [-19., 72., -17.]);
    let reused = frame(snapshot, bank, version, Some(&first));
    assert!(reused.assembly.as_ref().unwrap().statistics.reused_geometry > 0);
    let blocks = [root_state(version)];
    let entity = mcedit2_world::clipboard::OpaqueBlockEntity {
        position: [0; 3],
        compound: payload(row),
    };
    let variants = BTreeMap::new();
    let volume = crate::pending_import::Volume {
        size: [1; 3],
        offset: [0; 3],
        blocks: &blocks,
        mask: None,
        block_entities: std::slice::from_ref(&entity),
        entities: &[],
        biomes: None,
        numeric_biomes: None,
        data_version: Some(version),
        painting_variants: &variants,
        metadata: false,
        map_world: None,
        item_environment: None,
    };
    let view = crate::textures::java_obj_tests::scene(Arc::new(bank.clone()));
    let preview = crate::pending_import::build_geometry_with_budget(
        &volume,
        [-19, 72, -17],
        &view,
        false,
        crate::language::Language::English,
        Default::default(),
        &|| false,
    )
    .unwrap();
    assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);
    assert!(!preview.mesh.chunks.is_empty());
    assert!(crate::pending_import::build_geometry_with_budget(
        &volume,
        [-19, 72, -17],
        &view,
        false,
        crate::language::Language::English,
        Default::default(),
        &|| true
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    let mut changed = Document::new(raw).unwrap();
    changed
        .apply(&Operation::Replace {
            path: format!(
                "{}/0/progress",
                if version == 1343 {
                    "/Level/TileEntities"
                } else {
                    "/block_entities"
                }
            ),
            value: Value::Float(0.75),
        })
        .unwrap();
    crate::object_fixture::write_region(&path, &[((-2, -2), changed.bytes().to_vec())]);
    let next = frame(
        snapshot_from_world(&world.world, bank),
        bank,
        version,
        Some(&first),
    );
    assert!(next.chunks[0].picking.world_bounds().unwrap().0[0] > bounds.0[0]);
    assert_eq!(first.chunks[0].picking.world_bounds().unwrap(), bounds);
    assert_ne!(fs::read(&path).unwrap(), original);
    if version != 1343 {
        resources.put("assets/minecraft/models/block/moving_piston.json",br#"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"minecraft:block/stone"}}}]}"#);
        let custom = resources.open();
        assert!(!custom.vanilla_piston_supported(&root_state(version)));
        assert_eq!(custom.geometry(&root_state(version)).unwrap().len(), 1);
    }
}
pub(super) fn complete() {
    for version in [1343, 3465, 3955] {
        let resources = assets(version);
        let bank = open(&resources, version);
        let rows = rows(version);
        assert!(bank.vanilla_piston_supported(&root_state(version)));
        native_plans(version, &bank, &rows);
        biome_context(version, &bank, selected(&rows, version));
        integrated(version, &resources, &bank, selected(&rows, version));
    }
}

fn biome_context(version: i32, bank: &TextureBank, row: &serde_json::Value) {
    use mcedit2_world::editing::{BiomeTarget, Bounds, Session};
    let mut world = crate::metadata_fixture::Fixture::new(version);
    if version != 1343 {
        crate::metadata_fixture::define_overworld(&mut world);
    }
    let mut doc = Document::new(raw_chunk(version, row)).unwrap();
    doc.apply(&Operation::Replace {
        path: if version == 1343 {
            "/Level/TileEntities/0/blockId"
        } else {
            "/block_entities/0/blockState"
        }
        .into(),
        value: if version == 1343 {
            Value::Int(2)
        } else {
            Value::Compound(vec![
                field("Name", Value::String("minecraft:grass_block".into())),
                field(
                    "Properties",
                    Value::Compound(vec![field("snowy", Value::String("false".into()))]),
                ),
            ])
        },
    })
    .unwrap();
    let chunks: Vec<_> = (-3..=-1)
        .flat_map(|x| (-3..=-1).map(move |z| (x, z)))
        .map(|coordinate| {
            let mut neighbor = Document::new(doc.bytes().to_vec()).unwrap();
            for (name, value) in [("xPos", coordinate.0), ("zPos", coordinate.1)]
                .into_iter()
                .filter(|(_, value)| *value != -2)
            {
                neighbor
                    .apply(&Operation::Replace {
                        path: format!("{}{name}", if version == 1343 { "/Level/" } else { "/" }),
                        value: Value::Int(value),
                    })
                    .unwrap();
                neighbor
                    .apply(&Operation::Replace {
                        path: format!(
                            "{}/0/{}",
                            if version == 1343 {
                                "/Level/TileEntities"
                            } else {
                                "/block_entities"
                            },
                            if name == "xPos" { "x" } else { "z" }
                        ),
                        value: Value::Int(value * 16 + if name == "xPos" { 13 } else { 15 }),
                    })
                    .unwrap();
            }
            (coordinate, neighbor.bytes().to_vec())
        })
        .collect();
    let path = world.root.join("region/r.-1.-1.mca");
    crate::object_fixture::write_region(&path, &chunks);
    world.world = mcedit2_world::inspect_world(&world.root).unwrap();
    let saved = fs::read(&path).unwrap();
    let mut session = Session::new(world.world.clone(), "minecraft:overworld").unwrap();
    let read = |session: &Session, old: Option<Arc<Snapshot>>| {
        crate::scene_layers::tests::model_snapshot(session.clone(), bank, old, None)
    };
    let first_snapshot = read(&session, None);
    assert!(!first_snapshot.biomes.is_empty());
    let first = frame(Arc::clone(&first_snapshot), bank, version, None);
    assert_eq!((first.rendered, first.missing), (1, 0));
    let first_quads = first.chunks[0].quads();
    assert!(first_quads[0]
        .rgba
        .iter()
        .all(|c| c[0] > 0.99 && c[1] == 0. && c[2] == 0.));
    let repeated = read(&session, Some(Arc::clone(&first_snapshot)));
    assert!(Arc::ptr_eq(&first_snapshot, &repeated));
    let stroke = Bounds::new((-32, 64, -16), (-17, 79, -1)).unwrap();
    let target = if version == 1343 {
        BiomeTarget::Numeric(2)
    } else {
        BiomeTarget::Named("minecraft:desert".into())
    };
    let prepared = session
        .prepare_biome_brush_cancellable(&[stroke], Default::default(), &target, || false)
        .unwrap();
    session.commit_structure(prepared.into_structure()).unwrap();
    let changed = read(&session, Some(Arc::clone(&first_snapshot)));
    assert!(!Arc::ptr_eq(&first_snapshot, &changed));
    assert!(Arc::ptr_eq(
        &first_snapshot.markers[0].chunk,
        &changed.markers[0].chunk
    ));
    let next = frame(Arc::clone(&changed), bank, version, Some(&first));
    let stats = &next.assembly.as_ref().unwrap().statistics;
    assert_eq!(
        (
            stats.prepared_parts,
            stats.reused_preparations,
            stats.compiled_parts
        ),
        (0, 1, 1)
    );
    assert_ne!(*next.chunks[0].quads(), *first_quads);
    let next_quads = next.chunks[0].quads();
    assert!(next_quads[0].rgba[0][2] > 0.);
    let marker = changed.markers[0].marker();
    let BlockVisual::Piston(source) = marker.block_visual.as_deref().unwrap() else {
        panic!("实际移动方块来源缺失")
    };
    let part = source
        .render_parts(bank.render_state(&source.moved))
        .unwrap()
        .remove(0);
    let sample =
        std::array::from_fn::<_, 3, _>(|i| marker.position[i] as i32 + part.sample_offset[i]);
    let color = bank.tint_context(&changed.biomes).block(
        &part.state,
        0,
        None,
        (sample[0], sample[1], sample[2]),
    );
    assert_eq!(&next_quads[0].rgba[0][..3], &color);
    // 资源模型超出原格，源快照不能按格心误删；共享投影仅留下实际进入切面的几何。
    let clip = Bounds::new((-21, 72, -17), (-20, 73, -17)).unwrap();
    let clipped = crate::scene_layers::tests::model_snapshot(
        session.clone(),
        bank,
        Some(Arc::clone(&changed)),
        Some(clip),
    );
    assert_eq!(clipped.markers.len(), 1);
    let projected = crate::projected_entities::project(
        Arc::clone(&next),
        crate::projected_entities::Mapping::Orthogonal {
            origin: [-19., 100., -17.],
            screen: [0.; 2],
            right: [1., 0., 0.],
            down: [0., 0., 1.],
            depth: [0., -1., 0.],
            scale: 1.,
        },
        ([-20.25, 71.5, -17.5], [-19.75, 73., -16.5]),
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert!(!projected.triangles.is_empty());
    for vertex in projected.triangles.iter().flat_map(|t| &t.vertices) {
        assert!(vertex.xy[0] >= -1.25 && vertex.xy[0] <= -0.75);
    }
    // 预览覆盖从合成地形传给同一移动模型，不借用未变化的后台实体颜色。
    let mut blocks = vec![parse_state("minecraft:air").unwrap(); 8 * 8 * 8];
    blocks[(3 * 8 + 3) * 8 + 3] = root_state(version);
    let entity = mcedit2_world::clipboard::OpaqueBlockEntity {
        position: [3; 3],
        compound: {
            let mut payload = Document::new({
                let mut bytes = vec![10, 0, 0];
                bytes.extend(payload(row));
                bytes
            })
            .unwrap();
            payload
                .apply(&Operation::Replace {
                    path: if version == 1343 {
                        "/blockId"
                    } else {
                        "/blockState"
                    }
                    .into(),
                    value: doc
                        .inspect(if version == 1343 {
                            "/Level/TileEntities/0/blockId"
                        } else {
                            "/block_entities/0/blockState"
                        })
                        .unwrap(),
                })
                .unwrap();
            payload.bytes()[3..].to_vec()
        },
    };
    let names = mcedit2_world::clipboard::BiomeVolume {
        resolution: 1,
        size: [8; 3],
        cells: vec!["minecraft:desert".to_string(); 8 * 8 * 8],
    };
    let numeric = mcedit2_world::numeric_biome::Volume {
        size: [8; 3],
        cells: vec![2; 8 * 8 * 8],
        data_version: Some(version),
    };
    let variants = BTreeMap::new();
    let volume = crate::pending_import::Volume {
        size: [8; 3],
        offset: [0; 3],
        blocks: &blocks,
        mask: None,
        block_entities: std::slice::from_ref(&entity),
        entities: &[],
        biomes: if version == 1343 { None } else { Some(&names) },
        numeric_biomes: if version == 1343 {
            Some(&numeric)
        } else {
            None
        },
        data_version: Some(version),
        painting_variants: &variants,
        metadata: false,
        map_world: None,
        item_environment: None,
    };
    let mut view = crate::textures::java_obj_tests::scene(Arc::new(bank.clone()));
    Arc::make_mut(&mut view.surface).biomes = Arc::clone(&changed.biomes);
    let preview = crate::pending_import::build_geometry_with_budget(
        &volume,
        [-22, 69, -20],
        &view,
        false,
        crate::language::Language::English,
        Default::default(),
        &|| false,
    )
    .unwrap();
    assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);
    assert!(!preview.mesh.chunks.is_empty());
    assert!(preview.mesh.chunks.iter().all(|c| c
        .quads()
        .iter()
        .all(|q| q.rgba[0][0] == 0. && q.rgba[0][2] > 0.99)));
    assert!(session.undo().unwrap());
    let restored = read(&session, Some(changed));
    let restored = frame(restored, bank, version, Some(&next));
    assert_eq!(*restored.chunks[0].quads(), *first_quads);
    assert_eq!(*first.chunks[0].quads(), *first_quads);
    assert_eq!(fs::read(&path).unwrap(), saved);
}
