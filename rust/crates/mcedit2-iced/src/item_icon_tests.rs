//! 放入现有资源类别的整组物品管线验证；不增加测试入口。
use super::*;
use crate::{
    language::Language,
    textures::{LoadControl, TextureBank},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
fn png(color: [u8; 4]) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba(color))
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}
fn pack(path: &Path, color: [u8; 4]) {
    std::fs::create_dir_all(path).unwrap();
    let put = |name: &str, data: &[u8]| {
        let target = path.join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, data).unwrap();
    };
    for (name, data) in [
        (
            "assets/minecraft/blockstates/heavy_core.json",
            r#"{"variants":{"":{"model":"block/heavy_core"}}}"#,
        ),
        (
            "assets/minecraft/models/block/heavy_core.json",
            r#"{"textures":{"all":"item/a"},"elements":[{"from":[4,0,4],"to":[12,8,12],"faces":{"up":{"texture":"all"}}}],"display":{"gui":{"rotation":[30,225,0]}}}"#,
        ),
        (
            "assets/minecraft/models/item/heavy_core.json",
            r#"{"parent":"block/heavy_core"}"#,
        ),
        (
            "version.json",
            r#"{"id":"1.21.1","name":"1.21.1","world_version":3955,"protocol_version":767}"#,
        ),
        (
            "assets/minecraft/models/item/generated.json",
            r#"{"parent":"builtin/generated"}"#,
        ),
        (
            "assets/minecraft/models/item/diamond.json",
            r#"{"parent":"item/generated","textures":{"layer0":"item/a"},"overrides":[{"predicate":{"custom_model_data":3},"model":"item/diamond_alt"}]}"#,
        ),
        (
            "assets/minecraft/models/item/diamond_alt.json",
            r#"{"parent":"item/generated","textures":{"layer0":"item/b"}}"#,
        ),
        (
            "assets/minecraft/models/item/leather_chestplate.json",
            r#"{"parent":"item/generated","textures":{"layer0":"item/white"}}"#,
        ),
        (
            "assets/minecraft/models/item/stone.json",
            r#"{"parent":"block/cube_all","textures":{"all":"item/a"},"display":{"gui":{"rotation":[30,225,0],"scale":[0.625,0.625,0.625]}}}"#,
        ),
        (
            "assets/minecraft/models/item/glass.json",
            r#"{"loader":"neoforge:separate_transforms","base":{"elements":[]},"perspectives":{"gui":{"parent":"block/cube_all","textures":{"all":"item/b"}}}}"#,
        ),
        (
            "assets/minecraft/models/item/chest.json",
            r#"{"parent":"builtin/entity"}"#,
        ),
        (
            "assets/minecraft/models/item/apple.json",
            r##"{"parent":"item/generated","textures":{"layer0":"#layer1","layer1":"#layer0"}}"##,
        ),
        (
            "assets/minecraft/lang/zh_cn.json",
            r#"{"item.minecraft.diamond":"钻石","block.minecraft.stone":"石头"}"#,
        ),
        (
            "assets/minecraft/lang/en_us.json",
            r#"{"item.minecraft.diamond":"Diamond","block.minecraft.stone":"Stone"}"#,
        ),
    ] {
        put(name, data.as_bytes());
    }
    put("assets/minecraft/textures/item/a.png", &png(color));
    put(
        "assets/minecraft/textures/item/b.png",
        &png([0, 255, 0, 255]),
    );
    put("assets/minecraft/textures/item/white.png", &png([255; 4]));
    put(
        "assets/minecraft/models/item/emerald.json",
        br#"{"parent":"item/generated","textures":{"layer0":"item/animated"}}"#,
    );
    let mut animated = Cursor::new(Vec::new());
    image::RgbaImage::from_fn(2, 4, |_, y| {
        image::Rgba(if y < 2 {
            [255, 0, 0, 128]
        } else {
            [0, 0, 255, 128]
        })
    })
    .write_to(&mut animated, image::ImageFormat::Png)
    .unwrap();
    put(
        "assets/minecraft/textures/item/animated.png",
        &animated.into_inner(),
    );
    put(
        "assets/minecraft/textures/item/animated.png.mcmeta",
        br#"{"animation":{"width":2,"height":2,"frametime":2}}"#,
    );
    for id in [
        "white_bed",
        "black_banner",
        "white_banner",
        "shield",
        "shulker_box",
        "conduit",
        "decorated_pot",
        "skeleton_skull",
        "wither_skeleton_skull",
        "creeper_head",
        "zombie_head",
        "player_head",
        "piglin_head",
        "dragon_head",
    ] {
        put(
            &format!("assets/minecraft/models/item/{id}.json"),
            br#"{"parent":"builtin/entity"}"#,
        );
    }
    for texture in [
        "entity/chest/normal",
        "entity/bed/white",
        "entity/shulker/shulker",
        "entity/conduit/base",
        "entity/decorated_pot/decorated_pot_base",
        "entity/decorated_pot/decorated_pot_side",
        "entity/decorated_pot/angler_pottery_pattern",
        "entity/banner_base",
        "entity/shield_base",
        "entity/shield_base_nopattern",
        "entity/skeleton/skeleton",
        "entity/skeleton/wither_skeleton",
        "entity/creeper/creeper",
        "entity/zombie/zombie",
        "entity/player/wide/steve",
        "entity/piglin/piglin",
        "entity/enderdragon/dragon",
    ] {
        put(
            &format!("assets/minecraft/textures/{texture}.png"),
            &png(if texture.contains("shield") {
                [0, 0, 255, 255]
            } else {
                [255; 4]
            }),
        );
    }
    for family in ["banner", "shield"] {
        for mask in ["base", "border"] {
            put(
                &format!("assets/minecraft/textures/entity/{family}/{mask}.png"),
                &png([255, 255, 255, 128]),
            );
        }
    }
    for (id, predicate) in [
        ("crossbow", "charged"),
        ("elytra", "broken"),
        ("bundle", "filled"),
        ("light", "level"),
    ] {
        put(&format!("assets/minecraft/models/item/{id}.json"),serde_json::to_vec(&serde_json::json!({"parent":"item/generated","textures":{"layer0":"item/a"},"overrides":[{"predicate":{predicate:0.1},"model":"item/diamond_alt"}]})).unwrap().as_slice());
    }
    put(
        "assets/minecraft/models/item/enchanted_book.json",
        br#"{"parent":"item/generated","textures":{"layer0":"item/a"}}"#,
    );
    put(
        "assets/minecraft/models/item/diamond_chestplate.json",
        br#"{"parent":"item/generated","textures":{"layer0":"trims/items/test_trim_blue"}}"#,
    );
    put("assets/minecraft/atlases/blocks.json",br#"{"sources":[{"type":"paletted_permutations","textures":["trims/items/test_trim"],"palette_key":"trims/key","permutations":{"blue":"trims/blue"}}]}"#);
    let mut trim = Cursor::new(Vec::new());
    image::RgbaImage::from_fn(2, 4, |_, y| {
        image::Rgba(if y < 2 {
            [255, 0, 0, 128]
        } else {
            [0, 255, 0, 255]
        })
    })
    .write_to(&mut trim, image::ImageFormat::Png)
    .unwrap();
    put(
        "assets/minecraft/textures/trims/items/test_trim.png",
        trim.get_ref(),
    );
    put(
        "assets/minecraft/textures/trims/items/test_trim.png.mcmeta",
        br#"{"animation":{"width":2,"height":2,"frametime":2}}"#,
    );
    put(
        "assets/minecraft/textures/trims/key.png",
        &png([255, 0, 0, 255]),
    );
    put(
        "assets/minecraft/textures/trims/blue.png",
        &png([0, 0, 255, 128]),
    );
    let mut glint = Cursor::new(Vec::new());
    image::RgbaImage::from_fn(16, 16, |x, y| {
        image::Rgba([if (x + y) % 5 == 0 { 255 } else { 100 }, 40, 200, 255])
    })
    .write_to(&mut glint, image::ImageFormat::Png)
    .unwrap();
    for id in ["enchanted_glint_item", "enchanted_item_glint"] {
        put(
            &format!("assets/minecraft/textures/misc/{id}.png"),
            glint.get_ref(),
        );
    }
    put("assets/minecraft/textures/gui/unrelated.png", b"not a PNG");
}
fn load(path: &Path) -> TextureBank {
    TextureBank::open_stack_java_controlled(
        &[path.to_owned()],
        &LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap()
}
fn pixels(bank: &TextureBank, key: Appearance) -> crate::canvas_raster::Raster {
    render(bank, &key, &|| false).unwrap().unwrap().0
}
fn center(raster: &crate::canvas_raster::Raster) -> [u8; 4] {
    raster.rgba[(16 * 32 + 16) * 4..(16 * 32 + 16) * 4 + 4]
        .try_into()
        .unwrap()
}
fn wait(cache: &Arc<Cache>, bank: &TextureBank, key: &Appearance) -> Handle {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match cache.request(bank, key.clone()) {
            Icon::Ready(handle) => return handle,
            Icon::Error(error) => panic!("{error}"),
            Icon::Pending => {
                assert!(Instant::now() < deadline, "图标工作线程没有完成");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
pub(super) fn pipeline() {
    crate::item_generated::tests::complete();
    full_preset_colors();
    let temp = std::env::temp_dir().join(format!("mcedit2-item-icons-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("items");
    pack(&path, [255, 0, 0, 255]);
    let bank = load(&path);
    assert!(bank.get("minecraft:item/a").is_some());
    assert!(bank.get("minecraft:item/b").is_some());
    assert!(bank.get("minecraft:gui/unrelated").is_none());
    let key = Appearance::new("minecraft:diamond", Some(3955), None);
    assert_eq!(center(&pixels(&bank, key.clone())), [255, 0, 0, 255]);
    let mut alt = key.clone();
    alt.properties
        .as_mut()
        .unwrap()
        .insert("custom_model_data".into(), 3f32.to_bits());
    assert_eq!(center(&pixels(&bank, alt)), [0, 255, 0, 255]);
    let dyed = Value::Compound(vec![mcedit2_world::nbt_document::Field {
        name: "components".into(),
        value: Value::Compound(vec![mcedit2_world::nbt_document::Field {
            name: "minecraft:dyed_color".into(),
            value: Value::Compound(vec![mcedit2_world::nbt_document::Field {
                name: "rgb".into(),
                value: Value::Int(0x123456),
            }]),
        }]),
    }]);
    assert_eq!(
        center(&pixels(
            &bank,
            Appearance::new("minecraft:leather_chestplate", Some(3955), Some(&dyed))
        )),
        [0x12, 0x34, 0x56, 255]
    );
    for id in ["minecraft:stone", "minecraft:glass", "minecraft:heavy_core"] {
        let raster = pixels(&bank, Appearance::new(id, Some(3955), None));
        assert!(raster.rgba.chunks_exact(4).any(|p| p[3] != 0));
    }
    assert!(
        pixels(&bank, Appearance::new("minecraft:chest", Some(3955), None))
            .rgba
            .chunks_exact(4)
            .any(|p| p[3] != 0)
    );
    special_pipeline(&bank);
    appearance_pipeline(&bank);
    assert!(render(
        &bank,
        &Appearance::new("minecraft:apple", Some(3955), None),
        &|| false
    )
    .err()
    .unwrap()
    .contains("别名循环"));
    assert!(render(
        &bank,
        &Appearance::new("minecraft:diamond", Some(3465), None),
        &|| false
    )
    .is_err());
    assert!(render(&bank, &key, &|| true).unwrap().is_none());
    assert_eq!(
        item_catalog::search(Some(3955), Some(&bank), Language::Chinese, "钻石"),
        vec![item_catalog::Choice {
            id: "minecraft:diamond",
            damage: None
        }]
    );
    let matches = item_catalog::search(
        Some(3955),
        Some(&bank),
        Language::Chinese,
        "Minecraft:diamond",
    );
    assert!(matches.contains(&item_catalog::Choice {
        id: "minecraft:diamond",
        damage: None
    }));
    assert!(matches
        .iter()
        .all(|choice| choice.id.contains("minecraft:diamond")));
    assert_eq!(
        item_catalog::get(Some(1343)).unwrap().items["minecraft:stone"].variants[&1].model,
        "granite"
    );
    assert!(item_catalog::search(Some(1343), None, Language::English, "minecraft:stone").len() > 1);
    let heavy_core = mcedit2_world::editing::parse_state("minecraft:heavy_core").unwrap();
    assert_eq!(
        bank.resolve_id_at(&heavy_core, crate::isometric::Face::Top, (0, 0, 0)),
        Some("minecraft:item/a".into())
    );
    let cache = Arc::new(Cache::default());
    let first = wait(&cache, &bank, &key);
    let second = wait(&cache, &bank, &key);
    assert_eq!(first.id(), second.id());
    let animated = Appearance::new("minecraft:emerald", Some(3955), None);
    for (tick, expected) in [(0, [128, 0, 0, 128]), (2, [0, 0, 128, 128])] {
        let snapshot = bank
            .with_runtime_atlas(Arc::new(bank.atlas().unwrap().snapshot_at(tick).unwrap()))
            .unwrap();
        assert_eq!(center(&pixels(&snapshot, animated.clone())), expected);
    }
    let first_animated = wait(&cache, &bank, &animated);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Icon::Ready(handle) = cache.request(&bank, animated.clone()) {
            if handle.id() != first_animated.id() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "近期可见的动画图标没有更新");
        std::thread::sleep(Duration::from_millis(5));
    }
    let next = temp.join("next");
    pack(&next, [0, 0, 255, 255]);
    let next_bank = load(&next);
    let _ = cache.request(&bank, Appearance::new("minecraft:stone", Some(3955), None));
    let next_handle = wait(&cache, &next_bank, &key);
    assert_ne!(first.id(), next_handle.id());
    assert_eq!(center(&pixels(&next_bank, key)), [0, 0, 255, 255]);
    std::fs::remove_dir_all(path).unwrap();
    std::fs::remove_dir_all(next).unwrap();
    std::fs::remove_dir(temp).unwrap();
    official_clients();
}
fn official_clients() {
    let Some(paths) = std::env::var_os("MCEDIT2_ITEM_CLIENTS") else {
        return;
    };
    let output =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/item-icons-verification");
    std::fs::create_dir_all(&output).unwrap();
    let mut incomplete = Vec::new();
    for client in std::env::split_paths(&paths) {
        let bank = load(&client);
        let version = bank
            .item_data_version()
            .expect("准确客户端必须具有数据版本");
        let catalog = item_catalog::get(Some(version)).unwrap();
        let choices = item_catalog::search(Some(version), Some(&bank), Language::English, "");
        let mut sheet = image::RgbaImage::new(32 * 32, (choices.len().div_ceil(32) * 32) as u32);
        let mut errors = BTreeMap::<String, String>::new();
        let mut rendered = 0;
        for (i, choice) in choices.iter().enumerate() {
            match render(
                &bank,
                &Appearance::variant(choice.id, Some(version), choice.damage),
                &|| false,
            ) {
                Ok(Some((raster, _, _))) => {
                    if crate::item_specials::payload(choice.id, None).unwrap() != "" {
                        assert!(
                            raster.rgba.chunks_exact(4).any(|p| p[3] != 0),
                            "{} 专用物品图标为空",
                            choice.id
                        );
                    }
                    let image = image::RgbaImage::from_raw(32, 32, raster.rgba).unwrap();
                    image::imageops::replace(
                        &mut sheet,
                        &image,
                        (i % 32 * 32) as i64,
                        (i / 32 * 32) as i64,
                    );
                    rendered += 1;
                }
                Ok(None) => panic!("未取消的图标任务"),
                Err(error) => {
                    errors.insert(format!("{}:{:?}", choice.id, choice.damage), error);
                }
            }
        }
        let presets = crate::item_presets::get(Some(version)).unwrap();
        let mut appearances =
            image::RgbaImage::new(8 * 32, (presets.color_cases.len().div_ceil(8) * 32) as u32);
        for (index, preset) in presets.color_cases.iter().enumerate() {
            let result = render(
                &bank,
                &Appearance::new(&preset.id, Some(version), Some(preset.value().unwrap())),
                &|| false,
            )
            .unwrap_or_else(|e| panic!("{version} 完整外观 {index} {}: {e}", preset.id));
            let raster = result.unwrap().0;
            assert!(raster.rgba.chunks_exact(4).any(|p| p[3] != 0));
            let image = image::RgbaImage::from_raw(32, 32, raster.rgba).unwrap();
            image::imageops::replace(
                &mut appearances,
                &image,
                (index % 8 * 32) as i64,
                (index / 8 * 32) as i64,
            );
        }
        appearances
            .save(output.join(format!("appearances-{version}.png")))
            .unwrap();
        for required in [
            "minecraft:diamond",
            "minecraft:stone",
            "minecraft:apple",
            "minecraft:glass",
            "minecraft:leather_chestplate",
            "minecraft:enchanted_book",
            "minecraft:filled_map",
            "minecraft:heavy_core",
            "minecraft:chest",
            "minecraft:shield",
        ] {
            if required == "minecraft:heavy_core" && version != 3955 {
                continue;
            }
            assert!(catalog.items.contains_key(required));
            assert!(
                !errors.contains_key(&format!(
                    "{required}:{:?}",
                    if version == 1343 { Some(0) } else { None }
                )),
                "{required} {errors:?}"
            );
        }
        let unexpected: Vec<_> = errors
            .iter()
            .filter(|(id, _)| version != 1343 || id.as_str() != "minecraft:stone_slab:Some(2)")
            .collect();
        if !unexpected.is_empty() {
            incomplete.push(format!("{version}: {unexpected:?}"));
        }
        sheet
            .save(output.join(format!("items-{version}.png")))
            .unwrap();
        std::fs::write(output.join(format!("items-{version}.json")),serde_json::to_vec_pretty(&serde_json::json!({"version":version,"choices":choices.len(),"rendered":rendered,"errors":errors})).unwrap()).unwrap();
        eprintln!(
            "[item-reference] {version}: {rendered}/{} 个物品变体已绘制，{} 个专用或无效模型诊断",
            choices.len(),
            errors.len()
        );
    }
    assert!(incomplete.is_empty(), "原版物品渲染缺口：{incomplete:?}");
}

fn special_pipeline(bank: &TextureBank) {
    use sha2::{Digest, Sha256};
    let proof: serde_json::Value =
        serde_json::from_str(include_str!("../assets/item-layers-3465.proof.json")).unwrap();
    assert_eq!(
        proof["baseline_catalog_sha256"].as_str().unwrap(),
        format!(
            "{:x}",
            Sha256::digest(include_bytes!(
                "../../mcedit2-world/assets/entity-layers-1.21.1.json"
            ))
        )
    );
    assert_eq!(proof["layers"].as_array().unwrap().len(), 16);
    let reference: BTreeMap<String, Vec<[f32; 5]>> =
        serde_json::from_str(include_str!("../assets/item-special-reference-3955.json")).unwrap();
    for (id, names) in [
        ("chest", vec!["chest"]),
        ("white_bed", vec!["bed_head", "bed_foot"]),
        ("shulker_box", vec!["shulker"]),
        ("black_banner", vec!["banner"]),
        ("shield", vec!["shield"]),
        ("conduit", vec!["conduit"]),
        ("decorated_pot", vec!["pot_base", "pot_sides"]),
        ("skeleton_skull", vec!["skeleton_skull"]),
        ("wither_skeleton_skull", vec!["wither_skeleton_skull"]),
        ("creeper_head", vec!["creeper_head"]),
        ("zombie_head", vec!["zombie_head"]),
        ("player_head", vec!["player_head"]),
        ("piglin_head", vec!["piglin_head"]),
        ("dragon_head", vec!["dragon_head"]),
    ] {
        let key = Appearance::new(&format!("minecraft:{id}"), Some(3955), None);
        let mut scene = bank
            .item_scene(&key.model_id().unwrap(), &key.properties().unwrap())
            .unwrap();
        assert!(scene.builtin);
        let _ = crate::item_specials::prepare(bank, &key, &mut scene, &|| false).unwrap();
        let actual: Vec<_> = scene
            .quads
            .iter()
            .flat_map(|q| {
                (0..4).map(|i| {
                    [
                        q.xyz[i][0],
                        q.xyz[i][1],
                        q.xyz[i][2],
                        q.uv[i][0],
                        q.uv[i][1],
                    ]
                })
            })
            .collect();
        let expected: Vec<_> = names
            .into_iter()
            .flat_map(|name| reference[name].iter().copied())
            .collect();
        // 多层材质会重复绘制同一原生几何；核对全部顶点 / UV 及完整覆盖，不把绘制遍数当作模型尺寸。
        let mut remaining = expected.clone();
        for point in actual {
            let matches = |other: &[f32; 5]| {
                point
                    .iter()
                    .zip(other)
                    .all(|(a, b)| (a - b).abs() < 0.00002)
            };
            assert!(
                expected.iter().any(matches),
                "{id} 与真实客户端的顶点 / UV 不匹配：{point:?}"
            );
            if let Some(index) = remaining.iter().position(matches) {
                remaining.swap_remove(index);
            }
        }
        assert!(remaining.is_empty(), "{id} 未覆盖全部原生模型顶点");
    }
    let values = |component: &str, value: Value| {
        Value::Compound(vec![mcedit2_world::nbt_document::Field {
            name: "components".into(),
            value: Value::Compound(vec![mcedit2_world::nbt_document::Field {
                name: component.into(),
                value,
            }]),
        }])
    };
    let patterns = Value::List(mcedit2_world::nbt_document::List {
        element_type: 10,
        items: vec![Value::Compound(vec![
            mcedit2_world::nbt_document::Field {
                name: "pattern".into(),
                value: Value::String("minecraft:border".into()),
            },
            mcedit2_world::nbt_document::Field {
                name: "color".into(),
                value: Value::String("red".into()),
            },
        ])],
    });
    for id in ["minecraft:white_banner", "minecraft:shield"] {
        let plain = Appearance::new(id, Some(3955), None);
        let data = values("minecraft:banner_patterns", patterns.clone());
        let patterned = Appearance::new(id, Some(3955), Some(&data));
        assert_ne!(plain, patterned);
        assert_ne!(pixels(bank, plain).rgba, pixels(bank, patterned).rgba);
    }
    let banner = crate::banner_geometry::prepare_item(0, &[], false, bank, &|| false).unwrap();
    let shield = crate::banner_geometry::prepare_item(0, &[], true, bank, &|| false).unwrap();
    assert_ne!(banner.layers, shield.layers);
    assert!(Arc::ptr_eq(
        &banner,
        &crate::banner_geometry::prepare_item(0, &[], false, bank, &|| false).unwrap()
    ));
    let plain = Appearance::new("minecraft:decorated_pot", Some(3955), None);
    let decorations = Value::List(mcedit2_world::nbt_document::List {
        element_type: 8,
        items: vec![Value::String("minecraft:angler_pottery_sherd".into()); 4],
    });
    let data = values("minecraft:pot_decorations", decorations);
    let decorated = Appearance::new("minecraft:decorated_pot", Some(3955), Some(&data));
    assert_ne!(plain, decorated);
    let mut scene = bank
        .item_scene("minecraft:decorated_pot", &decorated.properties().unwrap())
        .unwrap();
    crate::item_specials::prepare(bank, &decorated, &mut scene, &|| false).unwrap();
    assert!(scene
        .quads
        .iter()
        .any(|q| q.texture == "minecraft:entity/decorated_pot/angler_pottery_pattern"));
    let bad = values("minecraft:banner_patterns", Value::Int(1));
    let bad = Appearance::new("minecraft:white_banner", Some(3955), Some(&bad));
    assert!(render(bank, &bad, &|| false).is_err());
    let bad = Value::Compound(vec![mcedit2_world::nbt_document::Field {
        name: "tag".into(),
        value: Value::Compound(vec![mcedit2_world::nbt_document::Field {
            name: "BlockEntityTag".into(),
            value: Value::Compound(vec![mcedit2_world::nbt_document::Field {
                name: "Base".into(),
                value: Value::String("坏底色".into()),
            }]),
        }]),
    }]);
    assert!(render(
        bank,
        &Appearance::new("minecraft:shield", Some(3955), Some(&bad)),
        &|| false
    )
    .is_err());
    let bad = Value::Compound(vec![mcedit2_world::nbt_document::Field {
        name: "components".into(),
        value: Value::Int(1),
    }]);
    assert!(render(
        bank,
        &Appearance::new("minecraft:shield", Some(3955), Some(&bad)),
        &|| false
    )
    .is_err());
    for id in ["minecraft:bed", "minecraft:banner"] {
        assert_eq!(
            item_catalog::get(Some(1343)).unwrap().items[id]
                .variants
                .len(),
            16
        );
    }
}

// 整个导出语料与真实原版提供器逐项对照，包含默认值、创造栏和自定义效果混色。
fn full_preset_colors() {
    use sha2::{Digest, Sha256};
    for (version, data, proof) in [
        (
            1343,
            include_bytes!("../assets/item-presets-1343.json").as_slice(),
            include_str!("../assets/item-presets-1343.proof.json"),
        ),
        (
            3465,
            include_bytes!("../assets/item-presets-3465.json").as_slice(),
            include_str!("../assets/item-presets-3465.proof.json"),
        ),
        (
            3955,
            include_bytes!("../assets/item-presets-3955.json").as_slice(),
            include_str!("../assets/item-presets-3955.proof.json"),
        ),
    ] {
        let proof: serde_json::Value = serde_json::from_str(proof).unwrap();
        assert_eq!(
            proof["output_sha256"].as_str().unwrap(),
            format!("{:x}", Sha256::digest(data))
        );
        let catalog = crate::item_presets::get(Some(version)).unwrap();
        let choices =
            crate::item_presets::search(Some(version), None, Language::English, "").unwrap();
        assert_eq!(choices.len(), catalog.presets.len());
        for preset in catalog.presets.iter().chain(&catalog.color_cases) {
            let value = preset.value().unwrap();
            let key = Appearance::new(&preset.id, Some(version), Some(value));
            assert_eq!(
                *key.foil.as_ref().unwrap(),
                preset.foil,
                "{version} {} foil: {value:?}",
                preset.id
            );
            let properties = key.properties().unwrap();
            assert_eq!(
                properties.keys().collect::<Vec<_>>(),
                preset.properties.keys().collect::<Vec<_>>(),
                "{version} {} properties",
                preset.id
            );
            for (name, actual) in properties {
                assert!(
                    (actual - preset.properties[&name]).abs() < 0.000001,
                    "{version} {} property {name}: {actual} != {}: {value:?}",
                    preset.id,
                    preset.properties[&name]
                );
            }
            let colors = key
                .colors
                .as_ref()
                .unwrap_or_else(|e| panic!("{version} {} {:?}: {e}", preset.id, value));
            assert_eq!(colors.len(), preset.tints.len());
            for (index, (actual, expected)) in colors.iter().zip(&preset.tints).enumerate() {
                assert_eq!(
                    actual & 0xffffff,
                    expected & 0xffffff,
                    "{version} {} tint {index}: {value:?}",
                    preset.id
                );
            }
        }
        assert!(
            crate::item_presets::search(Some(version), None, Language::English, "potion")
                .unwrap()
                .len()
                >= 30
        );
        if version == 3955 {
            for (id, component_name, child) in [
                ("minecraft:potion", "minecraft:potion_contents", "potion"),
                (
                    "minecraft:leather_chestplate",
                    "minecraft:dyed_color",
                    "rgb",
                ),
            ] {
                let candidates = if child == "potion" {
                    &catalog.presets
                } else {
                    &catalog.color_cases
                };
                let preset = candidates
                    .iter()
                    .rev()
                    .find(|p| {
                        p.id == id
                            && field(
                                field(
                                    field(Some(p.value().unwrap()), "components"),
                                    component_name,
                                ),
                                child,
                            )
                            .is_some()
                    })
                    .unwrap();
                let mut shorthand = preset.value().unwrap().clone();
                let Value::Compound(root) = &mut shorthand else {
                    panic!("物品")
                };
                let Value::Compound(components) = &mut root
                    .iter_mut()
                    .find(|f| f.name == "components")
                    .unwrap()
                    .value
                else {
                    panic!("组件")
                };
                let entry = components
                    .iter_mut()
                    .find(|f| f.name == component_name)
                    .unwrap();
                entry.value = field(Some(&entry.value), child).unwrap().clone();
                assert_eq!(
                    Appearance::new(id, Some(version), Some(&shorthand))
                        .colors
                        .as_ref()
                        .unwrap()
                        .iter()
                        .map(|c| c & 0xffffff)
                        .collect::<Vec<_>>(),
                    preset
                        .tints
                        .iter()
                        .map(|c| c & 0xffffff)
                        .collect::<Vec<_>>()
                );
            }
        }
        eprintln!(
            "[item-presets] {version}: {} 完整预设 + {} 原版动态颜色参照通过",
            catalog.presets.len(),
            catalog.color_cases.len()
        );
    }
}

fn appearance_pipeline(bank: &TextureBank) {
    let snapshot = bank
        .with_runtime_atlas(Arc::new(bank.atlas().unwrap().snapshot_at(0).unwrap()))
        .unwrap();
    let bank = &snapshot;
    let next = bank
        .with_runtime_atlas(Arc::new(bank.atlas().unwrap().snapshot_at(2).unwrap()))
        .unwrap();
    assert_eq!(
        next.item_tile("minecraft:trims/items/test_trim_blue")
            .unwrap()
            .rgba[0..4],
        [0, 255, 0, 255]
    );
    assert!(bank.item_texture_animated("minecraft:trims/items/test_trim_blue"));
    use crate::gpu_mesh::Vertex;
    let catalog = crate::item_presets::get(Some(3955)).unwrap();
    for id in [
        "minecraft:crossbow",
        "minecraft:elytra",
        "minecraft:bundle",
        "minecraft:light",
    ] {
        let preset = catalog.color_cases.iter().find(|p| p.id == id).unwrap();
        assert_eq!(
            center(&pixels(
                bank,
                Appearance::new(id, Some(3955), Some(preset.value().unwrap()))
            )),
            [0, 255, 0, 255]
        );
    }
    assert_eq!(
        bank.item_tile("minecraft:trims/items/test_trim_blue")
            .unwrap()
            .rgba[0..4],
        [0, 0, 255, 64]
    );
    let base = pixels(bank, Appearance::new("minecraft:diamond", Some(3955), None));
    let mut enchanted = Appearance::new("minecraft:diamond", Some(3955), None);
    enchanted.foil = Ok(true);
    let glowing = pixels(bank, enchanted.clone());
    assert_ne!(base.rgba, glowing.rgba);
    assert_eq!(
        base.rgba.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>(),
        glowing
            .rgba
            .chunks_exact(4)
            .map(|p| p[3])
            .collect::<Vec<_>>()
    );
    for (id, expected) in [("minecraft:chest", false), ("minecraft:shield", true)] {
        let plain = Appearance::new(id, Some(3955), None);
        let mut enchanted = plain.clone();
        enchanted.foil = Ok(true);
        assert_eq!(
            pixels(bank, plain).rgba != pixels(bank, enchanted).rgba,
            expected,
            "专用 {id} 光效通道"
        );
    }
    let cache = Arc::new(Cache::default());
    let first = wait(&cache, bank, &enchanted);
    let limit = Instant::now() + Duration::from_secs(2);
    loop {
        if let Icon::Ready(image) = cache.request(bank, enchanted.clone()) {
            if first.id() != image.id() {
                break;
            }
        }
        assert!(Instant::now() < limit, "无纹理动画的附魔图标没有刷新");
        std::thread::sleep(Duration::from_millis(5));
    }
    let plain = Appearance::new("minecraft:diamond", Some(3955), None);
    assert_eq!(plain.clone().with_online_skins(true), plain);
    let head = Appearance::new("minecraft:player_head", Some(3955), None);
    assert_ne!(head.clone().with_online_skins(true), head);
    let generation = cache.0.lock().unwrap().generation.clone();
    cache.set_online_skins(true);
    assert!(!Arc::ptr_eq(
        &generation,
        &cache.0.lock().unwrap().generation
    ));
    assert!(cache.0.lock().unwrap().entries.is_empty());
    let uv = &bank.atlas().unwrap().entries["minecraft:item/a"];
    let face = |depth, rgba| {
        [0, 1, 2, 0, 2, 3].map(|i| Vertex {
            xy: [[-0.8, -0.8], [0.8, -0.8], [0.8, 0.8], [-0.8, 0.8]][i],
            depth,
            rgba,
            uv: uv.sample(0.5, 0.5),
            mode: 0,
            xyz: [i as f32 % 2., i as f32 / 2., 0.],
            world_mode: 0,
            atlas_page: uv.page,
            light: crate::lighting::FULLBRIGHT,
        })
    };
    let mut vertices = face(0.7, [0.1, 0.1, 0.1, 1.]).to_vec();
    vertices.extend(face(0.3, [0.3, 0.3, 0.3, 1.]));
    let front = face(0.3, [0.3, 0.3, 0.3, 1.]);
    let mut sheet = image::RgbaImage::new(32 * 6, 32 * 3);
    for (row, version) in [1343, 3465, 3955].into_iter().enumerate() {
        for (col, time) in [0, 250, 500, 1000, 3000, 7000].into_iter().enumerate() {
            let passes = crate::item_glint::prepare(bank, Some(version), time).unwrap();
            let render = |mesh: &[Vertex]| {
                crate::canvas_raster::render_item(
                    mesh,
                    bank.atlas().map(|a| a.as_ref()),
                    iced::Size::new(32., 32.),
                    Quality::Fast,
                    &passes,
                    &|| false,
                )
                .unwrap()
                .unwrap()
            };
            let raster = render(&vertices);
            assert_eq!(
                raster.rgba,
                render(&front).rgba,
                "背后的几何不可重复叠加光效"
            );
            let image = image::RgbaImage::from_raw(32, 32, raster.rgba).unwrap();
            image::imageops::replace(&mut sheet, &image, (col * 32) as i64, (row * 32) as i64);
        }
    }
    let output =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/item-icons-verification");
    std::fs::create_dir_all(&output).unwrap();
    sheet.save(output.join("glint.png")).unwrap();
}
