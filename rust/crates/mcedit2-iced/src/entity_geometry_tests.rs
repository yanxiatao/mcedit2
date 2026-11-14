//! 实体几何、材质、只读拾取与场景交接的类别流程；不增加独立测试入口。
use super::*;
use crate::{gpu_mesh::Quality, spectator::Camera, textures::java_obj_tests::Fixture};
use iced::{Point, Size};
use image::ImageEncoder;
use mcedit2_world::scene_annotations::{ChunkAnnotations, Marker};

pub(crate) fn assets() -> (Fixture, Arc<TextureBank>) {
    let fixture = Fixture::new();
    for (index, texture) in TEXTURES.iter().enumerate() {
        let (namespace, path) = texture.split_once(':').unwrap();
        let mut png = Vec::new();
        let rgba = [
            20u8.wrapping_add((index as u8).wrapping_mul(13)),
            190u8.wrapping_sub((index as u8).wrapping_mul(9)),
            60u8.wrapping_add((index as u8).wrapping_mul(13)),
            255,
        ];
        let pixels: Vec<u8> = (0..64 * 64).flat_map(|_| rgba).collect();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, 64, 64, image::ExtendedColorType::Rgba8)
            .unwrap();
        fixture.put(&format!("assets/{namespace}/textures/{path}.png"), &png);
    }
    for path in [
        "type/plains",
        "type/desert",
        "profession/farmer",
        "profession_level/stone",
    ] {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &vec![255; 64 * 64 * 4],
                64,
                64,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        fixture.put(
            &format!("assets/minecraft/textures/entity/villager/{path}.png"),
            &png,
        );
    }
    fixture.put(
        "assets/minecraft/textures/entity/villager/profession/farmer.png.mcmeta",
        br#"{"villager":{"hat":"full"}}"#,
    );
    let bank = fixture.open();
    for texture in TEXTURES {
        let id = crate::resource_models::normalize_texture(texture).unwrap();
        assert!(
            bank.atlas().unwrap().entries.contains_key(&id),
            "{texture} -> {id}"
        );
    }
    (fixture, bank)
}
pub(super) fn actor(
    id: &str,
    position: [f64; 3],
    index: usize,
    appearance: BTreeMap<String, Value>,
) -> Marker {
    // 扩展字段必须来自实际保留的 Compound；缺少字段与丢失整份快照是不同情况。
    let payload = Value::Compound(
        appearance
            .iter()
            .map(|(name, value)| mcedit2_world::nbt_document::Field {
                name: name.clone(),
                value: value.clone(),
            })
            .collect(),
    )
    .payload()
    .unwrap();
    let mut visual = EntityVisual::parse_block_entity(&payload, Some(3955)).unwrap();
    visual.rotation = Some([0., 0.]);
    visual.appearance = appearance;
    Marker {
        kind: Kind::Entity,
        id: Some(id.into()),
        position,
        path: format!("Entities/{index}"),
        source: "original-entity-nbt".into(),
        chunk: (
            (position[0] / 16.).floor() as i32,
            (position[2] / 16.).floor() as i32,
        ),
        sha256: format!("{index:064x}"),
        fields: 4,
        bytes: 100,
        tick: None,
        priority: None,
        block_state: None,
        visual_error: None,
        block_visual: None,
        geometry: None,
        visual: Some(Arc::new(visual)),
    }
}
pub(crate) fn source(id: &str, position: [f64; 3]) -> Arc<Snapshot> {
    snapshot(vec![actor(id, position, 0, BTreeMap::new())])
}
pub(super) fn snapshot(markers: Vec<Marker>) -> Arc<Snapshot> {
    Arc::new(Snapshot::build(
        (0, 0),
        1,
        std::iter::once(Arc::new(ChunkAnnotations {
            lighting: Default::default(),
            markers,
            diagnostics: Vec::new(),
        })),
        None,
    ))
}
fn npc_state_assembly() {
    let (fixture, bank) = assets();
    let hat_id = "minecraft:entity/villager/profession/farmer";
    assert_eq!(bank.villager_hat(hat_id).unwrap(), "full");
    let overlay = Fixture::new();
    overlay.put(
        "assets/minecraft/textures/entity/villager/profession/farmer.png.mcmeta",
        br#"{"villager":{"hat":"partial"}}"#,
    );
    let stacked = TextureBank::open_stack_java_controlled(
        &[fixture.path.clone(), overlay.path.clone()],
        &crate::textures::LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stacked.villager_hat(hat_id).unwrap(), "partial");
    let replacement = Fixture::new();
    let png = std::fs::read(
        fixture
            .path
            .join("assets/minecraft/textures/entity/villager/profession/farmer.png"),
    )
    .unwrap();
    replacement.put(
        "assets/minecraft/textures/entity/villager/profession/farmer.png",
        &png,
    );
    let stacked = TextureBank::open_stack_java_controlled(
        &[fixture.path.clone(), replacement.path.clone()],
        &crate::textures::LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stacked.villager_hat(hat_id).unwrap(), "none");
    let build = |id: &str, appearance: BTreeMap<String, Value>, textured: bool| {
        Frame::build(
            snapshot(vec![actor(id, [0.5, 0., 0.5], 0, appearance)]),
            &bank,
            Some(3955),
            textured,
            &|| false,
        )
        .unwrap()
    };
    let adult = build("minecraft:sheep", BTreeMap::new(), true);
    let sheared = build(
        "minecraft:sheep",
        BTreeMap::from([("Sheared".into(), Value::Byte(1))]),
        true,
    );
    assert!(adult.chunks[0].quads().len() > sheared.chunks[0].quads().len());
    let colors = [
        0xe6e6e6, 0xba6015, 0x953a8d, 0x2b86a3, 0xbea22d, 0x609517, 0xb6687f, 0x353b3d, 0x757571,
        0x107575, 0x66258a, 0x2d337f, 0x623f25, 0x465d10, 0x84221c, 0x151518,
    ];
    for (color, rgb) in colors.into_iter().enumerate() {
        let frame = build(
            "minecraft:sheep",
            BTreeMap::from([("Color".into(), Value::Byte(color as i8))]),
            true,
        );
        let expected = [
            ((rgb >> 16) & 255) as f32 / 255.,
            ((rgb >> 8) & 255) as f32 / 255.,
            (rgb & 255) as f32 / 255.,
            1.,
        ];
        assert!(frame.chunks[0]
            .quads()
            .iter()
            .any(|q| q.rgba[0] == expected));
    }
    let baby = build(
        "minecraft:sheep",
        BTreeMap::from([("Age".into(), Value::Int(-1))]),
        true,
    );
    assert_ne!(
        adult.chunks[0].quads()[0].xyz,
        baby.chunks[0].quads()[0].xyz
    );
    let stand = build("minecraft:armor_stand", BTreeMap::new(), true);
    let arms = build(
        "minecraft:armor_stand",
        BTreeMap::from([("ShowArms".into(), Value::Byte(1))]),
        true,
    );
    assert!(arms.chunks[0].quads().len() > stand.chunks[0].quads().len());
    let marker = build(
        "minecraft:armor_stand",
        BTreeMap::from([
            ("Marker".into(), Value::Byte(1)),
            ("Small".into(), Value::Byte(1)),
        ]),
        true,
    );
    assert_eq!(marker.rendered, 1);
    assert_ne!(
        stand.chunks[0].quads()[0].xyz,
        marker.chunks[0].quads()[0].xyz
    );
    let field = |name: &str, value| mcedit2_world::nbt_document::Field {
        name: name.into(),
        value,
    };
    let modifier = |id: &str, amount, operation: &str| {
        Value::Compound(vec![
            field("id", Value::String(id.into())),
            field("amount", Value::Double(amount)),
            field("operation", Value::String(operation.into())),
        ])
    };
    let modifiers = Value::List(mcedit2_world::nbt_document::List {
        element_type: 10,
        items: vec![
            modifier("oracle:add", 1., "add_value"),
            modifier("oracle:base", 0.5, "add_multiplied_base"),
            modifier("oracle:total", 0.25, "add_multiplied_total"),
        ],
    });
    let scale_attr = Value::Compound(vec![
        field("id", Value::String("minecraft:generic.scale".into())),
        field("base", Value::Double(2.)),
        field("modifiers", modifiers),
    ]);
    let visual = EntityVisual {
        source_nbt: None,
        source_version: Some(3955),
        rotation: Some([0., 0.]),
        appearance: BTreeMap::from([(
            "attributes".into(),
            Value::List(mcedit2_world::nbt_document::List {
                element_type: 10,
                items: vec![scale_attr],
            }),
        )]),
    };
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../mcedit2-world/tests/fixtures/scale-attribute-reference.json"
    ))
    .unwrap();
    let expected = reference["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "combined")
        .unwrap()["value"]
        .as_f64()
        .unwrap();
    assert_eq!(npc::scale(&visual).unwrap() as f64, expected);
    let official_nbt: &[(&str, &[u8])] = &[
        (
            "default",
            include_bytes!("../../mcedit2-world/tests/fixtures/scale-attribute-default.nbt"),
        ),
        (
            "combined",
            include_bytes!("../../mcedit2-world/tests/fixtures/scale-attribute-combined.nbt"),
        ),
        (
            "below_min",
            include_bytes!("../../mcedit2-world/tests/fixtures/scale-attribute-below_min.nbt"),
        ),
        (
            "above_max",
            include_bytes!("../../mcedit2-world/tests/fixtures/scale-attribute-above_max.nbt"),
        ),
        (
            "duplicate_rejected",
            include_bytes!(
                "../../mcedit2-world/tests/fixtures/scale-attribute-duplicate_rejected.nbt"
            ),
        ),
        (
            "duplicate_replaced",
            include_bytes!(
                "../../mcedit2-world/tests/fixtures/scale-attribute-duplicate_replaced.nbt"
            ),
        ),
    ];
    for (id, bytes) in official_nbt {
        let document = mcedit2_world::nbt_document::Document::new(bytes.to_vec()).unwrap();
        let value = document.inspect("").unwrap();
        let visual = EntityVisual {
            source_nbt: None,
            source_version: Some(3955),
            rotation: Some([0., 0.]),
            appearance: BTreeMap::from([(
                "attributes".into(),
                Value::List(mcedit2_world::nbt_document::List {
                    element_type: 10,
                    items: vec![value],
                }),
            )]),
        };
        let expected = reference["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == *id)
            .unwrap()["value"]
            .as_f64()
            .unwrap();
        assert!(
            (npc::scale(&visual).unwrap() as f64 - expected).abs() < 0.000001,
            "{id}"
        );
    }
    let posed = build(
        "minecraft:armor_stand",
        BTreeMap::from([(
            "Pose".into(),
            Value::Compound(vec![field(
                "Head",
                Value::List(mcedit2_world::nbt_document::List {
                    element_type: 5,
                    items: vec![Value::Float(20.), Value::Float(15.), Value::Float(-10.)],
                }),
            )]),
        )]),
        true,
    );
    assert_eq!(posed.rendered, 1);
    assert_ne!(
        stand.chunks[0].quads().as_ref(),
        posed.chunks[0].quads().as_ref()
    );
    let scaled = build(
        "minecraft:armor_stand",
        BTreeMap::from([(
            "attributes".into(),
            Value::List(mcedit2_world::nbt_document::List {
                element_type: 10,
                items: vec![Value::Compound(vec![
                    field("id", Value::String("minecraft:generic.scale".into())),
                    field("base", Value::Double(2.)),
                ])],
            }),
        )]),
        true,
    );
    for (a, b) in stand.chunks[0].quads()[0]
        .xyz
        .iter()
        .zip(scaled.chunks[0].quads()[0].xyz)
    {
        assert!((0..3).all(|i| (a[i] * 2. - b[i]).abs() < 0.00001));
    }
    let data = Value::Compound(vec![
        field("type", Value::String("minecraft:plains".into())),
        field("profession", Value::String("minecraft:farmer".into())),
        field("level", Value::Int(1)),
    ]);
    let villager = build(
        "minecraft:villager",
        BTreeMap::from([("VillagerData".into(), data.clone())]),
        true,
    );
    let child = build(
        "minecraft:villager",
        BTreeMap::from([
            ("VillagerData".into(), data),
            ("Age".into(), Value::Int(-1)),
        ]),
        true,
    );
    assert_eq!((villager.rendered, child.rendered), (1, 1));
    assert!(villager.chunks[0].quads().len() > child.chunks[0].quads().len());
    assert!(villager.chunks[0]
        .quads()
        .iter()
        .all(|q| q.mode & crate::material_mode::ORDERED_OPAQUE != 0));
    let untextured = build(
        "minecraft:villager",
        BTreeMap::from([(
            "VillagerData".into(),
            Value::Compound(vec![
                field("type", Value::String("minecraft:plains".into())),
                field("profession", Value::String("minecraft:none".into())),
            ]),
        )]),
        false,
    );
    assert!(untextured.chunks[0]
        .quads()
        .iter()
        .all(|q| q.mode == crate::material_mode::ORDERED_OPAQUE));
}
fn npc_java_render_reference() {
    #[derive(serde::Deserialize)]
    struct References {
        cases: Vec<Case>,
    }
    #[derive(serde::Deserialize)]
    struct Case {
        id: String,
        layer: String,
        young: bool,
        yaw: f32,
        scale: f32,
        overrides: BTreeMap<String, mcedit2_world::entity_model::PartPose>,
        vertices: Vec<[f32; 5]>,
    }
    let cases: References = serde_json::from_str(include_str!(
        "../../mcedit2-world/tests/fixtures/npc-model-reference.json"
    ))
    .unwrap();
    let (_fixture, bank) = assets();
    let catalog = Catalog::java_1_21_1().unwrap();
    for case in cases.cases {
        let (texture, groups) = if case.layer.starts_with("minecraft:sheep#") {
            (
                if case.layer.ends_with("#fur") {
                    "minecraft:entity/sheep/sheep_fur"
                } else {
                    "minecraft:entity/sheep/sheep"
                },
                case.young.then_some(npc::Groups {
                    head_parts: &["/head"],
                    head_scale: 1.,
                    head_translate: [0., 8. / 16., 4. / 16.],
                    body_scale: 0.5,
                    body_translate: [0., 1.5, 0.],
                }),
            )
        } else if case.layer == "minecraft:armor_stand#main" {
            (
                "minecraft:entity/armorstand/wood",
                case.young.then_some(npc::Groups {
                    head_parts: &["/head"],
                    head_scale: 0.75,
                    head_translate: [0., 1., 0.],
                    body_scale: 0.5,
                    body_translate: [0., 1.5, 0.],
                }),
            )
        } else {
            ("minecraft:entity/villager/villager", None)
        };
        let actual = npc::render(
            catalog,
            &bank,
            &case.layer,
            texture,
            &case.overrides,
            npc::RenderStyle {
                yaw: case.yaw,
                groups,
                outer: case.scale,
                tint: [1.; 4],
                ordered: false,
            },
        )
        .unwrap();
        let mut actual: Vec<_> = actual
            .iter()
            .flat_map(|q| q.xyz.into_iter().zip(q.uv))
            .collect();
        let atlas_uv = &bank.atlas().unwrap().entries[texture];
        assert_eq!(actual.len(), case.vertices.len(), "{}", case.id);
        for expected in case.vertices {
            let uv = atlas_uv.sample(expected[3], expected[4]);
            let found = actual.iter().position(|(xyz, actual_uv)| {
                (0..3).all(|i| (xyz[i] - expected[i]).abs() < 0.0001)
                    && (0..2).all(|i| (actual_uv[i] - uv[i]).abs() < 0.0001)
            });
            let index = found.unwrap_or_else(|| panic!("{}: 未找到顶点 {expected:?}", case.id));
            actual.swap_remove(index);
        }
    }
}
pub(crate) fn scene(
    bank: Arc<TextureBank>,
    snapshot: Arc<Snapshot>,
) -> crate::spectator::SpectatorScene {
    let mut scene = crate::textures::java_obj_tests::scene(bank.clone());
    Arc::make_mut(&mut scene.surface).voxels.clear();
    let mesh = Arc::new(crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    *scene.cache.lock().unwrap() = Some(mesh);
    scene.entity_enabled = true;
    scene.entity_frame = Some(Arc::new(
        Frame::build(snapshot, &bank, Some(3955), true, &|| false).unwrap(),
    ));
    scene.camera = Camera {
        position: [0., 0.8, -3.],
        yaw: 0.,
        pitch: 0.,
        zoom: 1.,
        perspective: true,
    };
    scene
}
fn geometry_materials_and_read_only_selection() {
    let (_fixture, bank) = assets();
    let mut markers = Vec::new();
    for id in [
        "zombie", "skeleton", "spider", "creeper", "cow", "pig", "frog",
    ] {
        let mut appearance = BTreeMap::new();
        if id == "frog" {
            appearance.insert(
                "variant".into(),
                Value::String("minecraft:temperate".into()),
            );
        }
        if id == "pig" {
            appearance.insert("Saddle".into(), Value::Byte(1));
        }
        markers.push(actor(
            &format!("minecraft:{id}"),
            [0.5, 0., 3.5],
            markers.len(),
            appearance,
        ));
    }
    for variant in ["cold", "warm"] {
        markers.push(actor(
            "minecraft:frog",
            [0.5, 0., 3.5],
            markers.len(),
            BTreeMap::from([(
                "variant".into(),
                Value::String(format!("minecraft:{variant}")),
            )]),
        ));
    }
    markers.push(actor(
        "addon:custom_actor",
        [4.5, 0., 3.5],
        markers.len(),
        BTreeMap::new(),
    ));
    markers.push(actor(
        "minecraft:pig",
        [7.5, 0., 3.5],
        markers.len(),
        BTreeMap::from([("Age".into(), Value::Int(-12))]),
    ));
    markers.push(actor(
        "minecraft:marker",
        [1., 2., 3.],
        markers.len(),
        BTreeMap::new(),
    ));
    markers.push(actor(
        "minecraft:cow",
        [1., 2., 3.],
        markers.len(),
        BTreeMap::from([("Invisible".into(), Value::Byte(-1))]),
    ));
    let data = snapshot(markers);
    let frame = Frame::build(data.clone(), &bank, Some(3955), true, &|| false).unwrap();
    assert_eq!(
        (frame.rendered, frame.missing, frame.invisible),
        (10, 1, 2),
        "{:?}",
        frame.diagnostics
    );
    assert_eq!(frame.diagnostics.len(), 1);
    assert!(frame
        .chunks
        .iter()
        .flat_map(|c| c.picking.source_groups())
        .all(|g| g.owner.is_none()));
    for chunk in &frame.chunks {
        for index in 0..chunk.picking.source_groups().len() {
            assert!(frame.marker((chunk.key.0, chunk.key.1), index).is_some());
        }
    }
    assert!(Frame::build(data.clone(), &bank, Some(3955), true, &|| true).is_err());
    let old = Frame::build(data, &bank, Some(3465), true, &|| false).unwrap();
    assert_eq!(old.rendered, 10);
    let mut far_marker = actor(
        "minecraft:creeper",
        [29_999_999.25, 64., -29_999_999.75],
        0,
        BTreeMap::new(),
    );
    far_marker.chunk = (0, 0);
    let far = snapshot(vec![far_marker]);
    let precise = Frame::build(far, &bank, Some(3955), true, &|| false).unwrap();
    let (min, max) = precise.chunks[0].picking.world_bounds().unwrap();
    assert!((min[0] - 29_999_998.5).abs() < 0.001 && (max[0] - 29_999_999.).abs() < 0.001);
    let key = precise.chunks[0].key;
    let hit = precise.chunks[0]
        .picking
        .cast(
            Camera {
                position: [(min[0] + max[0]) / 2., (min[1] + max[1]) / 2., min[2] - 4.],
                yaw: 0.,
                pitch: 0.,
                zoom: 1.,
                perspective: true,
            },
            Point::new(80., 64.),
            crate::render_viewport::RenderViewport::one_to_one(Size::new(160., 128.)),
            Some(&precise.atlas),
            Quality::Fast,
            f64::INFINITY,
        )
        .unwrap();
    assert_eq!(hit.group_index, 0);
    assert_eq!(
        precise
            .marker((key.0, key.1), hit.group_index)
            .unwrap()
            .marker()
            .chunk,
        (0, 0)
    );
    let outside = source("minecraft:creeper", [i32::MAX as f64 + 32., 0., 0.]);
    let invalid = Frame::build(outside, &bank, Some(3955), true, &|| false).unwrap();
    assert_eq!((invalid.rendered, invalid.missing), (0, 1));
    assert!(invalid.chunks.is_empty());
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|error| error.contains("超出")),
        "{:?}",
        invalid.diagnostics
    );
    let source = source("minecraft:creeper", [0.5, 0., 3.5]);
    let scene = scene(bank.clone(), source.clone());
    let base = scene.cache.lock().unwrap().clone().unwrap();
    let composite = compose(&base, scene.entity_frame.as_ref(), &scene.entity_cache);
    assert!(StreamMesh::same_terrain(&base, &composite));
    assert!(Arc::ptr_eq(
        &composite,
        &compose(&base, scene.entity_frame.as_ref(), &scene.entity_cache)
    ));
    assert!(Arc::ptr_eq(
        &base,
        &compose(&composite, None, &scene.entity_cache)
    ));
    let hit = composite
        .pick(
            scene.camera,
            Point::new(80., 64.),
            crate::render_viewport::RenderViewport::one_to_one(Size::new(160., 128.)),
            Quality::Fast,
        )
        .unwrap();
    assert!(hit.owner.is_none());
    assert_eq!(
        hit.marker.as_ref().unwrap().marker().id.as_deref(),
        Some("minecraft:creeper")
    );
    // 显示模型的正常点选只进入实体详情；不构造源方块归属或编辑请求。
    let (mut app, _, _, _) = crate::picking_integration_tests::fixture();
    app.scene_layers.snapshot = Some(source.clone());
    *app.spectator_presented.lock().unwrap() = Some(composite.clone());
    let selected = hit.marker.unwrap();
    drop(crate::update_impl(
        &mut app,
        Message::EntityModelSelected(composite.clone(), selected.clone()),
    ));
    assert!(app.scene_layers.selected.is_some());
    assert!(!app.current_session((0, 0)).unwrap().has_pending_changes());
    app.scene_layers.snapshot = Some(snapshot(Vec::new()));
    app.scene_layers.selected = None;
    drop(crate::update_impl(
        &mut app,
        Message::EntityModelSelected(composite, selected),
    ));
    assert!(app.scene_layers.selected.is_none());
}
fn generation_cancel_and_atlas_handoff() {
    let (_fixture, bank) = assets();
    let source = source("minecraft:creeper", [0.5, 0., 3.5]);
    let frame = Arc::new(
        scene::assemble(
            source.clone(),
            &bank,
            Some(3955),
            true,
            true,
            &|| false,
            Some(std::path::Path::new("")),
            false,
            &SkinResults::new(),
            &HeadResults::new(),
            crate::language::Language::English,
            None,
            None,
            None,
            crate::entity_visibility::Visibility::default(),
            None,
        )
        .unwrap(),
    );
    let mut state = State::default();
    let task = state.maintain(
        Some(source.clone()),
        Some(bank.clone()),
        Some(3955),
        true,
        true,
    );
    drop(task);
    let epoch = state.epoch;
    assert!(state.running);
    state.accept(epoch, Ok(frame.clone()));
    assert!(Arc::ptr_eq(state.frame.as_ref().unwrap(), &frame));
    drop(state.maintain(
        Some(source.clone()),
        Some(bank.clone()),
        Some(3955),
        true,
        true,
    ));
    assert!(!state.running);
    // 真正撤下显示帧并经历多次无快照更新，编译缓存仍限于同一资源和世界上下文。
    for _ in 0..3 {
        drop(state.maintain(None, Some(bank.clone()), Some(3955), true, false));
        assert!(state.frame.is_none());
        assert!(Arc::ptr_eq(state.geometry_cache.as_ref().unwrap(), &frame));
    }
    let task = state.maintain(
        Some(source.clone()),
        Some(bank.clone()),
        Some(3955),
        true,
        true,
    );
    let Message::EntityModelsLoaded(epoch, result) =
        crate::picking_integration_tests::edit_task_output(task)
    else {
        panic!("应完成正式实体编译任务");
    };
    state.accept(epoch, result);
    assert!(
        state
            .frame
            .as_ref()
            .unwrap()
            .assembly
            .as_ref()
            .unwrap()
            .statistics
            .reused_geometry
            > 0
    );
    let frame = Arc::clone(state.frame.as_ref().unwrap());
    let old_cancel = state.cancelled.clone();
    state.enabled = false;
    drop(state.maintain(
        Some(source.clone()),
        Some(bank.clone()),
        Some(3955),
        true,
        true,
    ));
    assert!(old_cancel.load(Ordering::Acquire));
    state.accept(epoch, Ok(frame.clone()));
    // 方块实体独立于生物开关；保留旧帧供同代交接，显示端按 show_entities 过滤。
    assert!(state
        .frame
        .as_ref()
        .is_some_and(|retained| Arc::ptr_eq(retained, &frame)));
    state.enabled = true;
    drop(state.maintain(
        Some(source.clone()),
        Some(bank.clone()),
        Some(3955),
        false,
        true,
    ));
    let plain =
        Arc::new(Frame::build(source.clone(), &bank, Some(3955), false, &|| false).unwrap());
    state.accept(state.epoch, Ok(plain.clone()));
    assert!(plain
        .chunks
        .iter()
        .flat_map(|c| c.quads().iter())
        .all(|q| q.mode == 0 && q.uv == [plain.atlas.white.sample(0.5, 0.5); 4]));
    let scene = scene(bank, source);
    let base = scene.cache.lock().unwrap().clone().unwrap();
    let old = compose(&base, Some(&frame), &scene.entity_cache);
    let mut different = Frame::build(
        frame.snapshot.clone(),
        scene.texture_bank.as_deref().unwrap(),
        Some(3955),
        true,
        &|| false,
    )
    .unwrap();
    different.atlas = Arc::new(TextureAtlas::empty_test_atlas());
    let different = Arc::new(different);
    assert!(Arc::ptr_eq(
        &old,
        &compose(&old, Some(&different), &scene.entity_cache)
    ));
    let mut replacement = (*base).clone();
    replacement.atlas = Some(different.atlas.clone());
    let replacement = Arc::new(replacement);
    let next = compose(&replacement, Some(&different), &scene.entity_cache);
    assert!(Arc::ptr_eq(next.entities.as_ref().unwrap(), &different));
    assert!(Arc::ptr_eq(next.atlas.as_ref().unwrap(), &different.atlas));
}
pub(crate) fn installed(bank: &TextureBank, world: &mcedit2_world::WorldSummary) {
    let mut reader =
        mcedit2_world::scene_annotations::Reader::new(world, &world.dimensions[0].name).unwrap();
    let chunks = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| (x, z)))
        .map(|(x, z)| Arc::new(reader.read(x, z).unwrap()))
        .collect::<Vec<_>>();
    let data = Arc::new(Snapshot::build((0, 0), 1, chunks.into_iter(), None));
    let entities = data
        .markers
        .iter()
        .filter(|m| matches!(m.marker().kind, Kind::Entity | Kind::Player))
        .count();
    let started = std::time::Instant::now();
    let frame = Frame::build(Arc::clone(&data), bank, world.data_version, true, &|| false).unwrap();
    // 统计中包含诊断与隐身附属物；只有实际几何才有拾取组。
    let owners = frame.entities.values().flatten().collect::<Vec<_>>();
    for chunk in frame.chunks.iter() {
        let groups = chunk.picking.source_groups();
        let coordinate = (chunk.key.0, chunk.key.1);
        let count = frame.entities.get(&coordinate).map_or(0, Vec::len);
        assert_eq!(groups.len(), count, "几何组必须有同区块源归属");
        assert!(groups.iter().all(|group| !group.quads.is_empty()));
    }
    let mut modeled_entities = 0;
    for (index, owner) in owners.iter().enumerate() {
        assert_eq!(
            data.markers
                .iter()
                .filter(|source| source.same(owner))
                .count(),
            1,
            "几何必须来自快照中的唯一对象：{}",
            owner.marker().path
        );
        assert!(!owners[..index].iter().any(|other| other.same(owner)));
        assert_ne!(owner.marker().id.as_deref(), Some("minecraft:marker"));
        modeled_entities += usize::from(matches!(owner.marker().kind, Kind::Entity | Kind::Player));
    }
    assert!(
        modeled_entities > 0 && frame.rendered > 0,
        "实际区域应有可显示的实体"
    );
    assert!(frame.missing == 0 || !frame.diagnostics.is_empty());
    let report = serde_json::json!({"world":world.root,"data_version":world.data_version,"chunks":9,
        "source_entities":entities,"entity_sources_with_geometry":modeled_entities,"entity_sources_without_geometry":entities-modeled_entities,"managed_block_visuals":frame.managed_blocks.len(),"modeled":frame.rendered,"missing":frame.missing,"invisible":frame.invisible,"quads":frame.chunks.iter().map(|c|c.quads().len()).sum::<usize>(),"seconds":started.elapsed().as_secs_f64(),"diagnostics":frame.diagnostics});
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/entity-model-instance-profile.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!("实际存档静态实体与方块实体模型：{report}");
}

fn profile_pixels_and_source_cache_lifecycle() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use mcedit2_world::skull::{Kind as SkullKind, Profile, Property, Source};
    let (fixture, _bank) = assets();
    // 资源定义与真实世界中的纹理属性分别使用各自的客户端读取合同。
    fixture.put(
        "assets/minecraft/blockstates/player_head.json",
        br#"{"variants":{"":{"model":"minecraft:block/skull"}}}"#,
    );
    fixture.put(
        "assets/minecraft/models/block/skull.json",
        br#"{"parent":"builtin/entity"}"#,
    );
    let bank = fixture.open();
    let cache = Fixture::new();
    let rgba: Vec<u8> = (0..64 * 64).flat_map(|_| [63, 127, 191, 255]).collect();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, 64, 64, image::ExtendedColorType::Rgba8)
        .unwrap();
    let mut appearances = Vec::new();
    for length in 60..=64 {
        let hash = "a".repeat(length);
        cache.put(&format!("heads/texture-{hash}.png"), &png);
        let text =
            format!("{{textures:{{SKIN:{{url:'http://textures.minecraft.net/texture/{hash}'}}}}}}");
        let profile = Profile {
            name: None,
            uuid: Some([7; 16]),
            properties: vec![Property {
                name: "textures".into(),
                value: if length % 2 == 0 {
                    STANDARD.encode(text).trim_end_matches('=').to_owned()
                } else {
                    STANDARD.encode(text)
                },
                signature: None,
            }],
        };
        let sources = crate::skull_skin::Sources {
            cache: cache.path.clone(),
            online: false,
        };
        let appearance =
            Arc::new(crate::skull_skin::load(&profile, false, &sources, &|| false).unwrap());
        let skin = appearance.skin.as_ref().unwrap();
        assert!(skin.cached);
        assert_eq!(skin.texture.rgba, rgba);
        appearances.push(appearance);
    }
    assert!(appearances
        .windows(2)
        .all(|pair| pair[0].skin.as_ref().unwrap().texture.id
            == pair[1].skin.as_ref().unwrap().texture.id));
    let appearance = Arc::clone(&appearances[0]);
    let profile = appearance.profile.clone();
    let key = crate::skull_skin::key(&profile, false);
    let state = mcedit2_world::sections::State {
        name: "minecraft:player_head".into(),
        properties: vec![("rotation".into(), "0".into())],
    };
    assert!(bank.vanilla_skull_supported(&state));
    let mut marker = actor("minecraft:skull", [0., 0., 0.], 0, BTreeMap::new());
    marker.kind = Kind::BlockEntity;
    marker.block_state = Some(Arc::new(state.clone()));
    marker.block_visual = Some(Arc::new(BlockVisual::Skull(Source {
        state,
        kind: SkullKind::Player,
        legacy_rotation: None,
        profile: Some(profile),
    })));
    let source = snapshot(vec![marker.clone()]);
    let other_dimension = Arc::new(Snapshot::build(
        (0, 1),
        2,
        std::iter::once(Arc::new(ChunkAnnotations {
            lighting: Default::default(),
            markers: vec![marker],
            diagnostics: Vec::new(),
        })),
        None,
    ));
    let root = fixture.path.clone();
    let mut provider = State::default();
    drop(provider.maintain_heads(Some(&source), Some(&bank), Some(&root), true));
    let epoch = provider.head_epoch;
    assert_eq!(provider.head_inflight, Some((epoch, key)));
    drop(provider.maintain_heads(None, Some(&bank), Some(&root), true));
    assert_eq!(provider.head_epoch, epoch);
    assert_eq!(provider.head_inflight, Some((epoch, key)));
    provider.accept_head_skin(epoch, key, Ok(Arc::clone(&appearance)));
    assert!(provider.head_results.active().is_empty());
    drop(provider.maintain_heads(Some(&source), Some(&bank), Some(&root), true));
    assert!(provider.head_inflight.is_none());
    assert_eq!(provider.head_requests, 1);
    let frame = Frame::build_with_root(
        Arc::clone(&source),
        &bank,
        Some(3955),
        true,
        true,
        &|| false,
        None,
        false,
        &BTreeMap::new(),
        provider.head_results.active(),
        crate::language::Language::English,
    )
    .unwrap();
    assert_eq!((frame.rendered, frame.missing), (1, 0));
    assert!(frame
        .atlas
        .entries
        .contains_key(&appearance.skin.as_ref().unwrap().texture.id));
    incremental_source_scene(&source.markers[0], &bank, &appearance);
    drop(provider.maintain_heads(Some(&other_dimension), Some(&bank), Some(&root), true));
    assert_eq!(provider.head_epoch, epoch);
    assert!(provider.head_inflight.is_none());
    assert!(Arc::ptr_eq(
        provider
            .head_results
            .active()
            .get(&key)
            .unwrap()
            .as_ref()
            .unwrap(),
        &appearance
    ));
    let empty = snapshot(Vec::new());
    drop(provider.maintain_heads(Some(&empty), Some(&bank), Some(&root), true));
    assert!(provider.head_results.active().is_empty());
    let reloaded_bank = fixture.open();
    drop(provider.maintain_heads(Some(&source), Some(&reloaded_bank), Some(&root), true));
    assert!(provider.head_inflight.is_none());
    assert_eq!(provider.head_epoch, epoch);
    assert!(Arc::ptr_eq(
        provider
            .head_results
            .active()
            .get(&key)
            .unwrap()
            .as_ref()
            .unwrap(),
        &appearance
    ));
    // 暂时隐藏视图也进入相同的非活动结果缓存，不由 UI 可见性重置网络来源。
    drop(provider.maintain_heads(None, Some(&bank), Some(&root), false));
    drop(provider.maintain_heads(Some(&source), Some(&bank), Some(&root), true));
    assert!(provider.head_inflight.is_none());
    provider.reload_appearances();
    drop(provider.maintain_heads(Some(&source), Some(&bank), Some(&root), true));
    let retry = provider.head_epoch;
    assert_ne!(retry, epoch);
    provider.accept_head_skin(epoch, key, Ok(appearance));
    assert_eq!(provider.head_inflight, Some((retry, key)));
    assert!(provider.head_results.active().is_empty());
    provider.accept_head_skin(retry, key, Err("真实源错误".into()));
    assert_eq!(
        provider
            .head_results
            .active()
            .get(&key)
            .unwrap()
            .as_ref()
            .err()
            .unwrap(),
        "真实源错误"
    );
    drop(provider.maintain_heads(Some(&empty), Some(&bank), Some(&root), true));
    drop(provider.maintain_heads(Some(&source), Some(&bank), Some(&root), true));
    assert!(provider.head_inflight.is_none());
    let other_root = root.join("other-world");
    drop(provider.maintain_heads(Some(&source), Some(&bank), Some(&other_root), true));
    assert!(provider.head_results.active().is_empty());
    assert!(provider.head_inflight.is_some());
    let mut player = actor(
        "minecraft:player",
        [0., 0., 0.],
        1,
        BTreeMap::from([
            (
                "mcedit2:player_uuid".into(),
                Value::IntArray(vec![0, 0, 0, 3]),
            ),
            ("playerGameType".into(), Value::Int(0)),
            ("ShoulderEntityLeft".into(), Value::Compound(Vec::new())),
            ("ShoulderEntityRight".into(), Value::Compound(Vec::new())),
        ]),
    );
    player.kind = Kind::Player;
    let uuid = player_uuid(&player).unwrap();
    let players = snapshot(vec![player]);
    let mut skins = State {
        online_skins: true,
        ..Default::default()
    };
    drop(skins.maintain_skins(Some(&players), Some(&root), true));
    let epoch = skins.skin_epoch;
    drop(skins.maintain_skins(None, Some(&root), true));
    assert_eq!(skins.skin_inflight, Some((epoch, uuid)));
    let player_appearance = Arc::new(crate::player_skin::Appearance::Custom(Arc::clone(
        appearances[0].skin.as_ref().unwrap(),
    )));
    skins.accept_skin(epoch, uuid, Ok(Arc::clone(&player_appearance)));
    drop(skins.maintain_skins(Some(&players), Some(&root), true));
    assert!(skins.skin_inflight.is_none());
    assert_eq!(skins.skin_requests, 1);
    assert!(Arc::ptr_eq(
        skins
            .skin_results
            .active()
            .get(&uuid)
            .unwrap()
            .as_ref()
            .unwrap(),
        &player_appearance
    ));
    // 预算只约束已离开视图的外观，不裁掉活动素材。
    let mut set = crate::appearance_results::WorkingSet::new(1);
    set.select(BTreeSet::from([1u8, 2u8]), |v: &String| v.len());
    assert!(set.publish(1, "first".into(), 5));
    assert!(set.publish(2, "second".into(), 6));
    assert_eq!(set.active().len(), 2);
    set.select(BTreeSet::new(), String::len);
    set.select(BTreeSet::from([1, 2]), String::len);
    assert!(set.active().is_empty());
}
// 一个实际场景的增量加载、材质变化、移出、返回及跨源区块合并流程。
fn incremental_source_scene(
    prototype: &MarkerRef,
    bank: &TextureBank,
    appearance: &crate::skull_skin::Appearance,
) {
    let mut heads = HeadResults::new();
    let mut sources = Vec::new();
    for (n, x, suffix, color) in [
        (1, 0., "z", [63, 127, 191, 255]),
        (2, 32., "a", [197, 21, 41, 255]),
    ] {
        let mut marker = prototype.marker().clone();
        marker.position = [x, 0., 0.];
        marker.chunk = ((x / 16.) as i32, 0);
        marker.path = format!("block_entities/{n}");
        let BlockVisual::Skull(mut skull) = marker.block_visual.as_deref().unwrap().clone() else {
            panic!("场景素材必须是头颅")
        };
        let profile = skull.profile.as_mut().unwrap();
        profile.uuid = Some([n; 16]);
        let key = crate::skull_skin::key(profile, false);
        let mut resolved = appearance.clone();
        resolved.profile = profile.clone();
        let mut skin = (**resolved.skin.as_ref().unwrap()).clone();
        skin.texture.id = format!("mcedit2:test/dynamic/{suffix}");
        skin.texture.rgba = (0..64 * 64).flat_map(|_| color).collect();
        resolved.skin = Some(Arc::new(skin));
        heads.insert(key, Ok(Arc::new(resolved)));
        marker.block_visual = Some(Arc::new(BlockVisual::Skull(skull)));
        sources.push(Arc::new(ChunkAnnotations {
            lighting: Default::default(),
            markers: vec![marker],
            diagnostics: Vec::new(),
        }));
    }
    let make = |parts: Vec<Arc<ChunkAnnotations>>| {
        Arc::new(Snapshot::build((0, 0), 1, parts.into_iter(), None))
    };
    let build = |data: Arc<Snapshot>, heads: &HeadResults, previous: Option<&Arc<Frame>>| {
        Arc::new(
            scene::assemble(
                data,
                bank,
                Some(3955),
                true,
                true,
                &|| false,
                None,
                false,
                &SkinResults::new(),
                heads,
                crate::language::Language::English,
                previous,
                None,
                None,
                crate::entity_visibility::Visibility::default(),
                None,
            )
            .unwrap(),
        )
    };
    let initial = build(make(vec![Arc::clone(&sources[0])]), &heads, None);
    let loaded = build(make(sources.clone()), &heads, Some(&initial));
    let work = loaded.assembly.as_ref().unwrap().statistics;
    assert_eq!(
        (
            work.prepared_parts,
            work.reused_preparations,
            work.compiled_parts,
            work.reused_geometry
        ),
        (1, 1, 1, 1)
    );
    assert!(Arc::ptr_eq(
        &initial.chunks[0].quads(),
        &loaded.chunks[0].quads()
    ));
    assert_eq!(
        initial.atlas.entries["mcedit2:test/dynamic/z"],
        loaded.atlas.entries["mcedit2:test/dynamic/z"]
    );
    let compare = |a: &Frame, b: &Frame| {
        assert_eq!(
            (a.rendered, a.missing, a.invisible),
            (b.rendered, b.missing, b.invisible)
        );
        assert_eq!(a.chunks.len(), b.chunks.len());
        for (a_chunk, b_chunk) in a.chunks.iter().zip(b.chunks.iter()) {
            assert_eq!((a_chunk.key, a_chunk.origin), (b_chunk.key, b_chunk.origin));
            assert_eq!(a_chunk.quads().len(), b_chunk.quads().len());
            for (a_quad, b_quad) in a_chunk.quads().iter().zip(b_chunk.quads().iter()) {
                assert_eq!(
                    (a_quad.xyz, a_quad.uv, a_quad.rgba, a_quad.mode),
                    (b_quad.xyz, b_quad.uv, b_quad.rgba, b_quad.mode)
                );
                let pixels = |frame: &Frame, quad: &WorldQuad| {
                    let uv: [f32; 2] =
                        std::array::from_fn(|i| (quad.uv[0][i] + quad.uv[2][i]) / 2.);
                    let page = frame.atlas.page(quad.atlas_page).unwrap();
                    let x = (uv[0].clamp(0., 0.999_999) * page.width as f32) as usize;
                    let y = (uv[1].clamp(0., 0.999_999) * page.height as f32) as usize;
                    page.rgba[(y * page.width as usize + x) * 4..][..4].to_vec()
                };
                assert_eq!(pixels(a, a_quad), pixels(b, b_quad));
            }
            assert_eq!(
                a_chunk.picking.source_groups(),
                b_chunk.picking.source_groups()
            );
            for group in 0..a_chunk.picking.source_groups().len() {
                assert!(a
                    .marker((a_chunk.key.0, a_chunk.key.1), group)
                    .unwrap()
                    .same(b.marker((b_chunk.key.0, b_chunk.key.1), group).unwrap()));
            }
            let (min, max) = a_chunk.picking.world_bounds().unwrap();
            let camera = Camera {
                position: [(min[0] + max[0]) / 2., (min[1] + max[1]) / 2., min[2] - 4.],
                yaw: 0.,
                pitch: 0.,
                zoom: 1.,
                perspective: true,
            };
            let pick = |frame: &Frame, chunk: &MeshChunk| {
                chunk
                    .picking
                    .cast(
                        camera,
                        Point::new(80., 64.),
                        crate::render_viewport::RenderViewport::one_to_one(Size::new(160., 128.)),
                        Some(&frame.atlas),
                        Quality::Fast,
                        f64::INFINITY,
                    )
                    .unwrap()
            };
            let ah = pick(a, a_chunk);
            let bh = pick(b, b_chunk);
            assert_eq!((ah.group_index, ah.outline), (bh.group_index, bh.outline));
        }
    };
    compare(&loaded, &build(make(sources.clone()), &heads, None));
    let same = build(make(sources.clone()), &heads, Some(&loaded));
    let work = same.assembly.as_ref().unwrap().statistics;
    assert_eq!(
        (
            work.prepared_parts,
            work.compiled_parts,
            work.reused_meshes,
            work.runtime_pages_built
        ),
        (0, 0, 2, 0)
    );
    assert!(Arc::ptr_eq(&same.atlas, &loaded.atlas));
    let compacted = build(make(vec![Arc::clone(&sources[1])]), &heads, Some(&same));
    assert_eq!(
        compacted.atlas.pages.len(),
        bank.atlas().unwrap().pages.len() + 1
    );
    assert_eq!(
        compacted
            .assembly
            .as_ref()
            .unwrap()
            .statistics
            .reused_preparations,
        1
    );
    compare(
        &compacted,
        &build(make(vec![Arc::clone(&sources[1])]), &heads, None),
    );
    let returned = build(make(sources.clone()), &heads, Some(&compacted));
    assert!(Arc::ptr_eq(
        &returned.chunks[1].quads(),
        &compacted.chunks[0].quads()
    ));
    compare(&returned, &build(make(sources.clone()), &heads, None));
    // 不同真实源段落入同一个渲染区块时，合并拾取组仍对应各自源标记。
    let extra = Arc::new(ChunkAnnotations {
        lighting: Default::default(),
        markers: vec![actor(
            "minecraft:creeper",
            [0.75, 0., 0.],
            7,
            BTreeMap::new(),
        )],
        diagnostics: Vec::new(),
    });
    let joint_sources = vec![Arc::clone(&sources[0]), extra, Arc::clone(&sources[1])];
    let joint = build(make(joint_sources.clone()), &heads, Some(&returned));
    compare(&joint, &build(make(joint_sources.clone()), &heads, None));
    let steady = build(make(joint_sources.clone()), &heads, Some(&joint));
    assert!(Arc::ptr_eq(
        &joint.chunks[0].quads(),
        &steady.chunks[0].quads()
    ));
    let original_heads = heads.clone();
    let key = *heads.keys().next().unwrap();
    heads.insert(key, Err("明确的外观加载错误".into()));
    let changed = build(make(joint_sources.clone()), &heads, Some(&steady));
    assert_eq!(changed.missing, 1);
    assert_eq!(
        changed
            .assembly
            .as_ref()
            .unwrap()
            .statistics
            .reused_preparations,
        2
    );
    compare(&changed, &build(make(joint_sources), &heads, None));
    // 历史帧的像素与页映射不受新场景回收或错误材质替换影响。
    compare(&loaded, &build(make(sources), &original_heads, None));
}

pub(super) fn run_test_category(category: crate::test_categories::Category) {
    match category {
        crate::test_categories::Category::Rendering => {
            geometry_materials_and_read_only_selection();
            npc_state_assembly();
            npc_java_render_reference();
        }
        crate::test_categories::Category::Background => {
            generation_cancel_and_atlas_handoff();
            profile_pixels_and_source_cache_lifecycle();
        }
        _ => {}
    }
}
