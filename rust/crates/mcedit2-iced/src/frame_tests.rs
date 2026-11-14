//! 展示框整组验证：准确客户端模型、fixed 上下文、六向几何、预览与动态材质。
use super::*;
use crate::hanging_fixture as f;
use image::ImageEncoder;
use mcedit2_world::nbt_document::{Field, Value};

pub(super) fn assets(version: i32) -> crate::textures::java_obj_tests::Fixture {
    let fixture = crate::textures::java_obj_tests::Fixture::new();
    let references: serde_json::Value =
        serde_json::from_str(include_str!("../tests/fixtures/frame-models.json")).unwrap();
    let reference = &references[version.to_string()];
    fixture.put("version.json", &serde_json::to_vec(&serde_json::json!({"id":reference["version"],"name":reference["version"],"world_version":version,"protocol_version":0})).unwrap());
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[180, 150, 100, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    for (path, doc) in reference["models"].as_object().unwrap() {
        fixture.put(path, &serde_json::to_vec(doc).unwrap());
        if let Some(textures) = doc["textures"].as_object() {
            for texture in textures
                .values()
                .filter_map(serde_json::Value::as_str)
                .filter(|s| !s.starts_with('#'))
            {
                let texture = texture.strip_prefix("minecraft:").unwrap_or(texture);
                fixture.put(&format!("assets/minecraft/textures/{texture}.png"), &png);
            }
        }
    }
    fixture.put("assets/minecraft/textures/block/stone.png", &png);
    fixture.put("assets/minecraft/textures/item/source.png", &png);
    fixture.put(
        "assets/minecraft/models/item/generated.json",
        br#"{"parent":"builtin/generated"}"#,
    );
    fixture.put("assets/minecraft/models/item/diamond_sword.json",br#"{"parent":"item/generated","textures":{"layer0":"item/source"},"display":{"gui":{"translation":[16,0,0]},"fixed":{"rotation":[0,180,0],"translation":[2,3,4],"scale":[0.75,0.5,0.25]}}}"#);
    fixture.put("assets/minecraft/models/item/chest.json",br#"{"parent":"builtin/entity","display":{"fixed":{"rotation":[0,180,0],"scale":[0.5,0.5,0.5]}}}"#);
    fixture.put("assets/minecraft/textures/entity/chest/normal.png", &png);
    fixture
}
fn change(value: &mut Value, name: &str, new: Value) {
    let Value::Compound(fields) = value else {
        unreachable!()
    };
    if let Some(field) = fields.iter_mut().find(|f| f.name == name) {
        field.value = new;
    } else {
        fields.push(Field {
            name: name.into(),
            value: new,
        });
    }
}
fn euler(facing: u8, p: [f32; 3]) -> [f32; 3] {
    let [yaw, pitch] = mcedit2_world::hanging::angles(facing);
    let (sy, cy) = (180. - yaw).to_radians().sin_cos();
    let (sx, cx) = pitch.to_radians().sin_cos();
    let [x, y, z] = p;
    let (x, z) = (cy * x + sy * z, -sy * x + cy * z);
    [x, cx * y - sx * z, sx * y + cx * z]
}
fn quads(prepared: &crate::frame_geometry::Prepared, bank: &TextureBank) -> Vec<WorldQuad> {
    let atlas =
        append_runtime_cached(bank.atlas().unwrap(), prepared.textures.clone(), &|| false).unwrap();
    prepared.quads(&atlas, true).unwrap()
}
pub(super) fn complete() {
    for version in [1343, 3465, 3955] {
        let assets = assets(version);
        let bank = assets.open();
        assert_eq!(bank.item_data_version(), Some(version));
        let facings: Vec<_> = if version == 1343 {
            mcedit2_world::hanging::HORIZONTAL.to_vec()
        } else {
            (0..6).collect()
        };
        for facing in facings {
            for rotation in 0..8 {
                for invisible in [false, true] {
                    let mut value = f::frame(version, facing, rotation, false, 1, [-19, 72, -17]);
                    change(&mut value, "Invisible", Value::Byte(i8::from(invisible)));
                    let original = value.clone();
                    let visual = f::visual(&value, version);
                    let prepared = crate::frame_geometry::prepare(
                        &visual,
                        false,
                        &bank,
                        None,
                        Some(version),
                        false,
                        &|| false,
                        None,
                        None,
                    )
                    .unwrap();
                    assert!(prepared.diagnostics.is_empty());
                    assert_eq!(prepared.center, [-19., 72.5, -17.]);
                    let faces = quads(&prepared, &bank);
                    let shown = !(invisible && version != 1343);
                    assert_eq!(faces.len(), if shown { 28 } else { 6 }); // 1x1 实心薄片：前后和四条边。
                    let fixed = bank
                        .item_scene_context("minecraft:diamond_sword", &BTreeMap::new(), "fixed")
                        .unwrap()
                        .transform;
                    let (s, c) = (rotation as f32 * 45.).to_radians().sin_cos();
                    let expected = [
                        [-0.5, 0.5, 1. / 32.],
                        [-0.5, -0.5, 1. / 32.],
                        [0.5, -0.5, 1. / 32.],
                        [0.5, 0.5, 1. / 32.],
                    ]
                    .map(|p| {
                        let [x, y, z] = fixed.point(p).map(|v| v * 0.5);
                        euler(
                            facing,
                            [
                                c * x - s * y,
                                s * x + c * y,
                                z + if shown { 0.4375 } else { 0.5 },
                            ],
                        )
                    });
                    let front = &faces[if shown { 22 } else { 0 }];
                    for (a, b) in front
                        .xyz
                        .into_iter()
                        .flatten()
                        .zip(expected.into_iter().flatten())
                    {
                        assert!(
                            (a - b).abs() < 0.000002,
                            "{version} {facing} {rotation}: {a}/{b}"
                        );
                    }
                    assert!(front.single_sided());
                    assert_eq!(value, original);
                }
            }
        }
        let mut value = f::frame(version, 3, 2, false, 1, [2, 3, 4]);
        change(&mut value, "Item", Value::Compound(vec![]));
        let empty = crate::frame_geometry::prepare(
            &f::visual(&value, version),
            false,
            &bank,
            None,
            Some(version),
            false,
            &|| false,
            None,
            None,
        )
        .unwrap();
        assert_eq!(quads(&empty, &bank).len(), 22);
        for q in quads(&empty, &bank) {
            assert!(!q.blended());
            assert_eq!(
                q.mode & (3 | 32 | crate::material_mode::FORCE_OPAQUE_ALPHA),
                if version == 1343 {
                    1 | 32
                } else {
                    crate::material_mode::FORCE_OPAQUE_ALPHA
                }
            );
            for p in q.xyz {
                assert!((-0.5..=-0.4375).contains(&p[2]));
            }
        }
        if version != 1343 {
            let glow = crate::frame_geometry::prepare(
                &f::visual(&value, version),
                true,
                &bank,
                None,
                Some(version),
                false,
                &|| false,
                None,
                None,
            )
            .unwrap();
            assert_ne!(quads(&empty, &bank)[0].uv, quads(&glow, &bank)[0].uv);
            change(&mut value, "Invisible", Value::Byte(1));
            let hidden = crate::frame_geometry::prepare(
                &f::visual(&value, version),
                true,
                &bank,
                None,
                Some(version),
                false,
                &|| false,
                None,
                None,
            )
            .unwrap();
            assert!(quads(&hidden, &bank).is_empty());
        }
        let mut chest = f::frame(version, 3, 0, false, 1, [2, 3, 4]);
        let mut item = f::visual(&chest, version).source_field("Item").unwrap();
        change(&mut item, "id", Value::String("minecraft:chest".into()));
        change(&mut chest, "Item", item);
        let chest = crate::frame_geometry::prepare(
            &f::visual(&chest, version),
            false,
            &bank,
            None,
            Some(version),
            false,
            &|| false,
            None,
            None,
        )
        .unwrap();
        assert!(quads(&chest, &bank).len() > 22);
        preview(version, &bank);
    }
    official_clients();
}
fn preview(version: i32, bank: &Arc<TextureBank>) {
    let value = f::frame(version, 3, 3, false, 1, [-19, 72, -17]);
    let visual = f::visual(&value, version);
    let source = crate::frame_geometry::prepare(
        &visual,
        false,
        bank,
        None,
        Some(version),
        false,
        &|| false,
        None,
        None,
    )
    .unwrap();
    let mut marker = tests::actor(
        "minecraft:item_frame",
        mcedit2_world::hanging::Frame::read(&visual)
            .unwrap()
            .center(),
        0,
        BTreeMap::new(),
    );
    marker.kind = Kind::Item;
    marker.visual = Some(Arc::new(visual.clone()));
    let snapshot = tests::snapshot(vec![marker]);
    let rendered =
        Frame::build(Arc::clone(&snapshot), bank, Some(version), true, &|| false).unwrap();
    assert_eq!(
        (rendered.rendered, rendered.missing),
        (1, 0),
        "{:?}",
        rendered.diagnostics
    );
    assert_eq!(rendered.entities.values().map(Vec::len).sum::<usize>(), 1);
    let chunk = &rendered.chunks[0];
    let hit = chunk.picking.cast(
        crate::spectator::Camera {
            position: [source.center[0], source.center[1], source.center[2] + 4.],
            yaw: 180.,
            pitch: 0.,
            zoom: 1.,
            perspective: true,
        },
        iced::Point::new(64., 64.),
        crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(128., 128.)),
        Some(&rendered.atlas),
        crate::gpu_mesh::Quality::Fast,
        f64::INFINITY,
    );
    assert!(hit.is_some(), "{version}: 展示框正面需要保留实际几何拾取");
    let entity = mcedit2_world::clipboard::OpaqueEntity {
        position: [1.25, 3.5, -2.75],
        block_position: None,
        compound: value.payload().unwrap(),
    };
    let blocks = [mcedit2_world::editing::parse_state("minecraft:air").unwrap()];
    let volume = crate::pending_import::Volume {
        size: [1; 3],
        offset: [0; 3],
        blocks: &blocks,
        mask: None,
        block_entities: &[],
        entities: std::slice::from_ref(&entity),
        biomes: None,
        numeric_biomes: None,
        data_version: Some(version),
        painting_variants: &BTreeMap::new(),
        metadata: false,
        map_world: None,
        item_environment: None,
    };
    let scene = crate::textures::java_obj_tests::scene(Arc::clone(bank));
    let frame = crate::pending_import::build_geometry_with_budget(
        &volume,
        [16, 0, 16],
        &scene,
        false,
        crate::language::Language::English,
        Default::default(),
        &|| false,
    )
    .unwrap();
    assert!(frame.diagnostics.is_empty(), "{:?}", frame.diagnostics);
    let actual: Vec<_> = frame
        .mesh
        .chunks
        .iter()
        .flat_map(|c| {
            c.quads()
                .iter()
                .flat_map(|q| {
                    q.xyz.map(|p| {
                        std::array::from_fn::<_, 3, _>(|i| f64::from(p[i]) + f64::from(c.origin[i]))
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let pos = mcedit2_world::entity_catalog::parse_preview(&entity.compound).unwrap()[0]
        .pos
        .unwrap();
    let delta: [f64; 3] =
        std::array::from_fn(|i| [16., 0., 16.][i] + entity.position[i] + source.center[i] - pos[i]);
    let expected: Vec<_> = quads(&source, bank)
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
fn official_clients() {
    let Some(paths) = std::env::var_os("MCEDIT2_ITEM_CLIENTS") else {
        return;
    };
    for path in std::env::split_paths(&paths) {
        let bank = TextureBank::open_stack_java_controlled(
            &[path],
            &crate::textures::LoadControl {
                all_png: false,
                ..Default::default()
            },
        )
        .unwrap();
        let version = bank.item_data_version().unwrap();
        let environment = super::item_world_tests::environment(version);
        super::map_tests::official_bank(&bank);
        let mut count = 0;
        for (id, item) in &crate::item_catalog::get(Some(version)).unwrap().items {
            if id == "minecraft:air" || id == "minecraft:filled_map" {
                continue;
            }
            let variants: Vec<_> = if version == 1343 {
                item.variants.keys().copied().collect()
            } else {
                vec![0]
            };
            for damage in variants {
                let mut value = f::frame(version, 3, 1, false, 1, [2, 3, 4]);
                let mut stack = f::visual(&value, version).source_field("Item").unwrap();
                change(&mut stack, "id", Value::String(id.clone()));
                if version == 1343 {
                    change(&mut stack, "Damage", Value::Short(damage));
                }
                change(&mut value, "Item", stack);
                let result = crate::frame_geometry::prepare(
                    &f::visual(&value, version),
                    false,
                    &bank,
                    None,
                    Some(version),
                    false,
                    &|| false,
                    Some(crate::frame_geometry::ItemContext {
                        environment: &environment,
                        offset: [0.; 3],
                    }),
                    None,
                );
                // 1.12.2 原版注册表确实指向客户端未提供的 old_wood_slab 模型。
                // 与背包图标类别相同，缺失资源必须正常报错，不能换成另一种木台阶。
                if version == 1343 && id == "minecraft:stone_slab" && damage == 2 {
                    let Err(error) = result else {
                        panic!("原版缺失模型意外获得了替代几何")
                    };
                    assert_eq!(error, "缺少物品模型 model:minecraft:item/old_wood_slab");
                    continue;
                }
                let p = result.unwrap_or_else(|error| panic!("{version} {id}:{damage}：{error}"));
                assert!(quads(&p, &bank).len() >= 22);
                count += 1;
            }
        }
        eprintln!(
            "展示框 {version}：准确客户端 {count} 个物品模型变体，fixed 全三维面和实际图集通过"
        );
    }
}
