//! 并入特殊模型整组：实际世界注册来源、图案合成、物品 / 盾牌、源场景与预览。
use super::*;
use crate::{banner_geometry, item_icons::Appearance, textures::java_obj_tests::Fixture};
use mcedit2_world::{
    banner::{Catalog, Pattern, PatternReference},
    nbt_document::{Field, List, Operation, Value},
};
use std::{fs, io::Write};

fn field(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
fn compound(fields: Vec<Field>) -> Value {
    Value::Compound(fields)
}
fn png(rgba: [u8; 4]) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba));
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}
fn assets() -> Fixture {
    let fixture = frame_tests::assets(3955);
    for name in ["white_banner", "white_wall_banner"] {
        fixture.put(
            &format!("assets/minecraft/blockstates/{name}.json"),
            br#"{"variants":{"":{"model":"block/banner"}}}"#,
        );
    }
    fixture.put("assets/minecraft/models/block/banner.json", br#"{}"#);
    for name in ["white_banner", "shield"] {
        fixture.put(&format!("assets/minecraft/models/item/{name}.json"),br#"{"parent":"builtin/entity","display":{"gui":{"rotation":[0,0,0],"scale":[0.75,0.75,0.75]}}}"#);
    }
    for (name, rgba) in [
        ("minecraft:entity/banner_base", [255; 4]),
        ("minecraft:entity/shield_base", [255; 4]),
        ("minecraft:entity/shield_base_nopattern", [255; 4]),
        ("minecraft:entity/banner/base", [0; 4]),
        ("minecraft:entity/shield/base", [0; 4]),
        ("cosmetic:entity/banner/emblem", [255, 0, 0, 255]),
        ("cosmetic:entity/shield/emblem", [255, 0, 0, 255]),
        ("cosmetic:entity/banner/alternate", [0, 0, 255, 255]),
        ("cosmetic:entity/shield/alternate", [0, 0, 255, 255]),
        ("example:entity/banner/crest", [0, 255, 0, 255]),
        ("example:entity/shield/crest", [0, 255, 0, 255]),
        ("example:entity/banner/unregistered", [255; 4]),
        ("minecraft:entity/banner/flow", [255; 4]),
    ] {
        let (namespace, path) = name.split_once(':').unwrap();
        fixture.put(
            &format!("assets/{namespace}/textures/{path}.png"),
            &png(rgba),
        );
    }
    // 资源包 data/ 不属于世界注册表，即使存在同名定义也不能覆盖来源。
    fixture.put(
        "data/example/banner_pattern/crest.json",
        br#"{"asset_id":"example:crest","translation_key":"wrong.pack"}"#,
    );
    fixture
}
fn put_pack(world: &crate::metadata_fixture::Fixture, pack: &str, path: &str, bytes: &[u8]) {
    let path = world.root.join("datapacks").join(pack).join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn upper(world: &crate::metadata_fixture::Fixture, asset: &str, broken: bool) {
    let file = fs::File::create(world.root.join("datapacks/upper.zip")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name,body) in [
        ("pack.mcmeta",r#"{"pack":{"pack_format":48},"filter":{"block":[{"namespace":"example","path":"banner_pattern/filtered\\.json"},{"namespace":"minecraft","path":"banner_pattern/flow\\.json"}]},"overlays":{"entries":[{"formats":48,"directory":"current"},{"formats":[49,100],"directory":"future"}]}}"#.to_owned()),
        ("data/example/banner_pattern/crest.json",r#"{"asset_id":"cosmetic:alternate","translation_key":"pattern.lower"}"#.into()),
        ("current/data/example/banner_pattern/crest.json",if broken {r#"{"asset_id":7}"#.into()}else{format!(r#"{{"asset_id":"{asset}","translation_key":"pattern.world"}}"#)}),
        ("current/data/minecraft/banner_pattern/border.json",format!(r#"{{"asset_id":"{asset}","translation_key":"pattern.override"}}"#)),
        ("current/data/example/painting_variant/replaced.json",r#"{"width":3,"height":2,"asset_id":"cosmetic:art"}"#.into()),
        ("future/data/example/banner_pattern/crest.json","broken future definition".into()),
    ] {
        zip.start_file(name,options).unwrap();zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}
fn world() -> crate::metadata_fixture::Fixture {
    let mut world = crate::metadata_fixture::Fixture::new(3955);
    put_pack(
        &world,
        "lower",
        "pack.mcmeta",
        br#"{"pack":{"pack_format":48}}"#,
    );
    put_pack(
        &world,
        "lower",
        "data/example/banner_pattern/crest.json",
        b"broken but replaced",
    );
    put_pack(
        &world,
        "lower",
        "data/example/banner_pattern/filtered.json",
        b"broken but filtered",
    );
    put_pack(
        &world,
        "lower",
        "data/example/painting_variant/replaced.json",
        b"broken but replaced",
    );
    put_pack(&world, "disabled", "pack.mcmeta", b"broken disabled pack");
    upper(&world, "cosmetic:emblem", false);
    let mut doc = mcedit2_world::nbt_file::FileDocument::open(
        &world.root,
        &world.root.join("level.dat"),
        false,
    )
    .unwrap()
    .document;
    doc.apply(&Operation::InsertField {
        path: "/Data".into(),
        name: "DataPacks".into(),
        value: compound(vec![field(
            "Enabled",
            Value::List(List {
                element_type: 8,
                items: ["vanilla", "file/lower", "file/upper.zip"]
                    .map(|v| Value::String(v.into()))
                    .to_vec(),
            }),
        )]),
    })
    .unwrap();
    fs::write(
        world.root.join("level.dat"),
        crate::metadata_fixture::gzip(doc.bytes()),
    )
    .unwrap();
    world.world = mcedit2_world::inspect_world(&world.root).unwrap();
    world
}
fn pattern(id: &str) -> Pattern {
    Pattern {
        source: PatternReference::Registry(id.into()),
        color: 0,
    }
}
fn layers(reference: Value) -> Value {
    Value::List(List {
        element_type: 10,
        items: vec![compound(vec![
            field("pattern", reference),
            field("color", Value::String("white".into())),
        ])],
    })
}
fn banner(reference: Value) -> Value {
    compound(vec![
        field("id", Value::String("minecraft:banner".into())),
        field("x", Value::Int(0)),
        field("y", Value::Int(0)),
        field("z", Value::Int(0)),
        field("patterns", layers(reference)),
    ])
}
fn source(value: &Value, state: &str) -> mcedit2_world::banner::Source {
    mcedit2_world::banner::parse(
        &value.payload().unwrap(),
        &mcedit2_world::editing::parse_state(state).unwrap(),
        Some(3955),
    )
    .unwrap()
}
fn snapshot(source: mcedit2_world::banner::Source) -> Arc<Snapshot> {
    let mut marker = tests::actor("minecraft:banner", [0., 0., 0.], 0, BTreeMap::new());
    marker.kind = Kind::BlockEntity;
    marker.block_state = Some(Arc::new(source.state.clone()));
    marker.visual = None;
    marker.block_visual = Some(Arc::new(BlockVisual::Banner(source)));
    tests::snapshot(vec![marker])
}
fn block_frame(
    bank: &TextureBank,
    sources: Arc<Snapshot>,
    previous: Option<&Arc<Frame>>,
) -> Arc<Frame> {
    Arc::new(
        scene::assemble(
            sources,
            bank,
            Some(3955),
            true,
            true,
            &|| false,
            None,
            false,
            &SkinResults::new(),
            &HeadResults::new(),
            crate::language::Language::English,
            previous,
            None,
            None,
            crate::entity_visibility::Visibility::default(),
            None,
        )
        .unwrap(),
    )
}
fn item(bank: &TextureBank, id: &str, reference: Value) -> crate::canvas_raster::Raster {
    let value = compound(vec![
        field("id", Value::String(id.into())),
        field("count", Value::Int(1)),
        field(
            "components",
            compound(vec![field("minecraft:banner_patterns", layers(reference))]),
        ),
    ]);
    crate::item_icons::render(
        bank,
        &Appearance::new(id, Some(3955), Some(&value)),
        &|| false,
    )
    .unwrap()
    .unwrap()
    .0
}
pub(super) fn complete() {
    for (version, raw, count) in [
        (
            1343,
            include_str!("../../mcedit2-world/assets/banner-patterns-1.12.2.json"),
            39,
        ),
        (
            3465,
            include_str!("../../mcedit2-world/assets/banner-patterns-1.20.1.json"),
            41,
        ),
        (
            3955,
            include_str!("../../mcedit2-world/assets/banner-patterns-1.21.1.json"),
            43,
        ),
    ] {
        let native: serde_json::Value = serde_json::from_str(raw).unwrap();
        let catalog = Catalog::builtin(version).unwrap();
        assert_eq!(catalog.0.len(), count);
        assert_eq!(native.as_object().unwrap().len(), count);
        for (id, definition) in native.as_object().unwrap() {
            let actual = catalog
                .resolve(&PatternReference::Registry(id.clone()))
                .unwrap();
            assert_eq!(actual.asset_id, definition["asset_id"].as_str().unwrap());
            assert_eq!(
                actual.legacy_hash.as_deref(),
                definition["legacy_hash"].as_str()
            );
            assert_eq!(
                actual.translation_key.as_deref(),
                definition["translation_key"].as_str()
            );
        }
    }
    let mut world = world();
    let resources = assets();
    let mut bank = resources.open().as_ref().clone();
    let control = crate::textures::LoadControl {
        all_png: false,
        ..Default::default()
    };
    bank.apply_world_data(&world.world, &control).unwrap();
    fs::remove_file(resources.path.join("version.json")).unwrap();
    let mut resource_only = resources.open().as_ref().clone();
    assert!(resource_only.item_data_version().is_none());
    resource_only
        .apply_world_data(&world.world, &control)
        .unwrap();
    assert_eq!(
        resource_only.banner_catalog().unwrap().0["example:crest"].asset_id,
        "cosmetic:emblem"
    );
    let catalog = world.world.world_data.banner_catalog(3955).unwrap();
    assert!(Arc::ptr_eq(
        &catalog,
        &world.world.world_data.banner_catalog(3955).unwrap()
    ));
    assert_eq!(catalog.0["example:crest"].asset_id, "cosmetic:emblem");
    assert_eq!(
        catalog.0["example:crest"].translation_key.as_deref(),
        Some("pattern.world")
    );
    assert!(!catalog.0.contains_key("example:filtered"));
    assert!(!catalog.0.contains_key("minecraft:flow"));
    assert_eq!(
        world.world.world_data.banner_catalog(3465).unwrap().0.len(),
        41
    );
    assert_eq!(
        world.world.world_data.painting_catalog(3955).unwrap().0["example:replaced"].width,
        3
    );
    let reference = Value::String("example:crest".into());
    let value = banner(reference.clone());
    let parsed = source(&value, "minecraft:white_banner[rotation=0]");
    let texture = banner_geometry::prepare(&parsed, &bank, &|| false).unwrap();
    assert_eq!(texture.layers.len(), 3);
    assert_eq!(texture.layers[2].texture, "cosmetic:entity/banner/emblem");
    assert!(texture.textures().next().is_none());
    assert!(Arc::ptr_eq(
        &texture,
        &banner_geometry::prepare(&parsed, &bank, &|| false).unwrap()
    ));
    let shield =
        banner_geometry::prepare_item(0, &parsed.patterns, true, &bank, &|| false).unwrap();
    assert_eq!(shield.layers[2].color, texture.layers[2].color);
    assert_eq!(shield.layers[2].texture, "cosmetic:entity/shield/emblem");
    for id in ["minecraft:border", "example:crest"] {
        assert_eq!(
            banner_geometry::prepare_item(0, &[pattern(id)], false, &bank, &|| false)
                .unwrap()
                .layers
                .last()
                .unwrap()
                .texture,
            texture.layers.last().unwrap().texture
        );
    }
    for id in ["example:filtered", "minecraft:flow", "example:unregistered"] {
        assert!(
            banner_geometry::prepare_item(0, &[pattern(id)], false, &bank, &|| false)
                .unwrap_err()
                .contains("注册数据")
        );
    }
    let mut hidden = vec![pattern("example:crest"); 16];
    hidden.push(pattern("example:unregistered"));
    assert!(
        banner_geometry::prepare_item(0, &hidden, false, &bank, &|| false)
            .unwrap_err()
            .contains("注册数据")
    );
    let inline = compound(vec![
        field("asset_id", Value::String("cosmetic:alternate".into())),
        field("translation_key", Value::String("pattern.inline".into())),
    ]);
    let inline_source = source(
        &banner(inline.clone()),
        "minecraft:white_banner[rotation=0]",
    );
    let inline_texture = banner_geometry::prepare(&inline_source, &bank, &|| false).unwrap();
    assert_eq!(
        inline_texture.layers.last().unwrap().texture,
        "cosmetic:entity/banner/alternate"
    );
    assert!(banner_geometry::prepare(&parsed, &bank, &|| true).is_err());
    let sources = snapshot(parsed.clone());
    let first = block_frame(&bank, Arc::clone(&sources), None);
    assert_eq!(first.missing, 0);
    assert!(!first.chunks.is_empty());
    let reused = block_frame(&bank, Arc::clone(&sources), Some(&first));
    assert_eq!(
        reused
            .assembly
            .as_ref()
            .unwrap()
            .statistics
            .reused_preparations,
        1
    );
    for id in ["minecraft:white_banner", "minecraft:shield"] {
        let registered = item(&bank, id, reference.clone());
        let direct = item(&bank, id, inline.clone());
        assert_ne!(registered.rgba, direct.rgba);
        assert!(registered
            .rgba
            .chunks_exact(4)
            .any(|p| p[0] > p[2] && p[3] > 0));
    }
    for state in [
        "minecraft:white_banner[rotation=7]",
        "minecraft:white_wall_banner[facing=east]",
    ] {
        let source = source(&value, state);
        let material = banner_geometry::prepare(&source, &bank, &|| false).unwrap();
        let atlas = append_runtime_cached(
            bank.atlas().unwrap(),
            material
                .textures()
                .map(|texture| (texture.id.clone(), Arc::clone(texture)))
                .collect(),
            &|| false,
        )
        .unwrap();
        let faces = banner_geometry::quads(&source, &bank, &atlas, &material, true).unwrap();
        assert!(!faces.is_empty());
        assert!(faces
            .iter()
            .flat_map(|q| q.xyz)
            .flatten()
            .all(f32::is_finite));
    }
    preview(&bank, &value, &parsed);
    let old = bank.visibility_id();
    upper(&world, "cosmetic:alternate", false);
    assert!(bank
        .apply_world_data(&world.world, &control)
        .unwrap_err()
        .contains("已变化"));
    world.world = mcedit2_world::inspect_world(&world.root).unwrap();
    let mut newer = bank.clone();
    newer.apply_world_data(&world.world, &control).unwrap();
    assert_ne!(old, newer.visibility_id());
    let changed = banner_geometry::prepare(&parsed, &newer, &|| false).unwrap();
    assert_ne!(changed.layers, texture.layers);
    assert!(Arc::ptr_eq(
        &texture,
        &banner_geometry::prepare(&parsed, &bank, &|| false).unwrap()
    ));
    let next = block_frame(&newer, sources, Some(&reused));
    assert_eq!(next.missing, 0);
    assert_eq!(next.assembly.as_ref().unwrap().statistics.prepared_parts, 1);
    assert_ne!(
        item(&bank, "minecraft:shield", reference.clone()).rgba,
        item(&newer, "minecraft:shield", reference).rgba
    );
    upper(&world, "cosmetic:emblem", true);
    world.world = mcedit2_world::inspect_world(&world.root).unwrap();
    let bad = world.world.world_data.banner_catalog(3955).unwrap_err();
    assert!(bad.contains("example:crest"));
    assert_eq!(
        bad,
        world.world.world_data.banner_catalog(3955).unwrap_err()
    );
    upper(&world, "cosmetic:emblem", false);
    let fresh = mcedit2_world::inspect_world(&world.root).unwrap();
    assert!(fresh.world_data.banner_catalog(3955).is_ok());
    assert!(world.world.world_data.banner_catalog(3955).is_err());
    official_clients();
}
fn preview(bank: &TextureBank, value: &Value, source: &mcedit2_world::banner::Source) {
    let blocks = [source.state.clone()];
    let entity = mcedit2_world::clipboard::OpaqueBlockEntity {
        position: [0; 3],
        compound: value.payload().unwrap(),
    };
    let volume = crate::pending_import::Volume {
        size: [1; 3],
        offset: [0; 3],
        blocks: &blocks,
        mask: None,
        block_entities: std::slice::from_ref(&entity),
        entities: &[],
        biomes: None,
        numeric_biomes: None,
        data_version: Some(3955),
        painting_variants: &BTreeMap::new(),
        metadata: false,
        map_world: None,
        item_environment: None,
    };
    let scene = crate::textures::java_obj_tests::scene(Arc::new(bank.clone()));
    let frame = crate::pending_import::build_geometry_with_budget(
        &volume,
        [-17, 64, -18],
        &scene,
        false,
        crate::language::Language::English,
        Default::default(),
        &|| false,
    )
    .unwrap();
    assert!(frame.diagnostics.is_empty(), "{:?}", frame.diagnostics);
    assert!(!frame.mesh.chunks.is_empty());
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
        let catalog = bank.banner_catalog().unwrap();
        for (id, definition) in &catalog.0 {
            for shield in [false, true] {
                let family = if shield { "shield" } else { "banner" };
                let (namespace, path) = definition.asset_id.split_once(':').unwrap();
                assert!(bank
                    .get(&format!("{namespace}:entity/{family}/{path}"))
                    .is_some());
                let texture =
                    banner_geometry::prepare_item(0, &[pattern(id)], shield, &bank, &|| false)
                        .unwrap();
                if bank.item_data_version() == Some(1343) {
                    let image = texture.textures().next().unwrap();
                    assert!(image.rgba.chunks_exact(4).any(|p| p[3] > 0));
                } else {
                    assert!(texture.textures().next().is_none());
                    assert_eq!(texture.layers.len(), 3);
                    assert_eq!(
                        texture.layers[2].texture,
                        format!("{namespace}:entity/{family}/{path}")
                    );
                }
            }
        }
        eprintln!(
            "准确客户端 {:?}：{} 项旗帜定义与旗帜 / 盾牌实际材质准备通过",
            bank.item_data_version(),
            catalog.0.len()
        );
    }
}
