//! 告示牌来源、资源字体、排版与实际场景交接的整组验证；复用特殊模型类别。
use crate::{
    font_glyphs::Fonts,
    language::Language,
    sign_geometry, sign_text,
    textures::{java_obj_tests::Fixture, TextureBank},
};
use image::ImageEncoder;
use mcedit2_world::{
    editing::parse_state,
    nbt_document::{Field, List, Value},
    sign::{Mount, Side, Source, WOODS},
};
use serde_json::json;
use std::{
    io::{Cursor, Write},
    sync::Arc,
};

fn png(f: &Fixture, path: &str, image: image::RgbaImage) {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &image,
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    f.put(path, &bytes);
}
fn assets() -> Fixture {
    let f = Fixture::new();
    for wood in WOODS {
        for (block, model) in [
            (format!("{wood}_sign"), format!("{wood}_sign")),
            (format!("{wood}_wall_sign"), format!("{wood}_sign")),
            (
                format!("{wood}_hanging_sign"),
                format!("{wood}_hanging_sign"),
            ),
            (
                format!("{wood}_wall_hanging_sign"),
                format!("{wood}_hanging_sign"),
            ),
        ] {
            f.put(
                &format!("assets/minecraft/blockstates/{block}.json"),
                &serde_json::to_vec(
                    &json!({"variants":{"":{"model":format!("minecraft:block/{model}")}}}),
                )
                .unwrap(),
            );
            f.put(
                &format!("assets/minecraft/models/block/{model}.json"),
                br#"{"textures":{"particle":"minecraft:block/oak_planks"}}"#,
            );
        }
        for path in [
            format!("entity/signs/{wood}"),
            format!("entity/signs/hanging/{wood}"),
        ] {
            png(
                &f,
                &format!("assets/minecraft/textures/{path}.png"),
                image::RgbaImage::from_pixel(64, 32, image::Rgba([128, 80, 25, 255])),
            );
        }
    }
    png(
        &f,
        "assets/demo/textures/block/glyph.png",
        image::RgbaImage::from_fn(32, 8, |x, y| {
            let width = [5, 3, 0, 7][(x / 8) as usize];
            image::Rgba([255, 255, 255, if x % 8 < width && y < 7 { 255 } else { 0 }])
        }),
    );
    // FontManager 的完整文档合同允许注释；索引与字形读取复用同一解释器。
    f.put("assets/minecraft/font/default.json", r#"{/* 字体定义 */"providers":[{"type":"space","advances":{" ":4,"\u200b":0}},{"type":"bitmap","file":"demo:block/glyph.png","height":8,"ascent":7,"chars":["AB 中"]},{"type":"reference","id":"demo:unicode"}]}"#.as_bytes());
    f.put("assets/demo/font/unicode.json", br#"{"providers":[{"type":"unihex","hex_file":"demo:font/glyphs.zip","size_overrides":[]}]}"#);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("test.hex", zip::write::SimpleFileOptions::default())
        .unwrap();
    writeln!(zip, "0044:{}", "F8".repeat(16)).unwrap();
    writeln!(zip, "0045:{}", "80".repeat(16)).unwrap();
    f.put(
        "assets/demo/font/glyphs.zip",
        &zip.finish().unwrap().into_inner(),
    );
    f.put(
        "assets/demo/font/cycle.json",
        br#"{"providers":[{"type":"reference","id":"demo:cycle"}]}"#,
    );
    f.put(
        "assets/demo/lang/zh_cn.json",
        r#"{"sign.test":"中文 %s"}"#.as_bytes(),
    );
    f.put("assets/demo/lang/en_us.json", br#"{"sign.test":"Text %s"}"#);
    f
}
fn field(name: &str, value: Value) -> Field {
    Field {
        name: name.into(),
        value,
    }
}
fn side(messages: [serde_json::Value; 4], glow: bool, color: &str) -> Value {
    Value::Compound(vec![
        field(
            "messages",
            Value::List(List {
                element_type: 8,
                items: messages
                    .into_iter()
                    .map(|value| Value::String(value.to_string()))
                    .collect(),
            }),
        ),
        field("color", Value::String(color.into())),
        field("has_glowing_text", Value::Byte(i8::from(glow))),
    ])
}
fn source(state: &str, message: serde_json::Value, glow: bool, color: &str) -> Source {
    let empty = || std::array::from_fn(|_| json!(""));
    let mut front = empty();
    front[0] = message;
    let fields = vec![
        field(
            "id",
            Value::String(
                if state.contains("hanging_sign") {
                    "minecraft:hanging_sign"
                } else {
                    "minecraft:sign"
                }
                .into(),
            ),
        ),
        field("front_text", side(front, glow, color)),
        field("back_text", side(empty(), false, "black")),
        field("is_waxed", Value::Byte(1)),
    ];
    let raw = Value::Compound(fields).payload().unwrap();
    mcedit2_world::sign::parse(&raw, &parse_state(state).unwrap(), Some(3955)).unwrap()
}
fn text(
    bank: &TextureBank,
    component: serde_json::Value,
    width: u32,
    outline: bool,
) -> sign_text::Prepared {
    let mut side = Side::default();
    side.messages[0] = component;
    sign_text::prepare(
        &side,
        bank,
        &mut Fonts::new(bank, Language::English),
        Language::English,
        width,
        10,
        0xffffff,
        outline.then_some(0x111111),
        &|| false,
    )
    .unwrap()
    .unwrap()
}
#[test]
fn fonts_layout_and_resources() {
    let fixture = assets();
    let bank = fixture.open();
    let mut fonts = Fonts::new(&bank, Language::English);
    let a = fonts
        .glyph("minecraft:default", 'A', &|| false)
        .unwrap()
        .unwrap();
    assert_eq!((a.advance, a.width, a.height, a.top), (6., 8, 8, 0.));
    let d = fonts
        .glyph("minecraft:default", 'D', &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(
        (d.advance, d.width, d.height, d.shadow_offset),
        (3., 5, 16, 0.5)
    );
    assert_eq!(
        fonts
            .glyph("minecraft:default", 'E', &|| false)
            .unwrap()
            .unwrap()
            .advance,
        1.
    );
    assert!(fonts.glyph("demo:cycle", 'A', &|| false).is_err());
    assert!(fonts.glyph("minecraft:default", 'A', &|| true).is_err());
    // A B 总宽 14，末尾空格超限时必须保留 B，不能回退到前一个空格。
    let line = text(&bank, json!("A B "), 14, false);
    assert_eq!(line.parts.len(), 2);
    assert_eq!(line.parts[0].corners[0], [-7., -20.]);
    assert_eq!(line.parts[1].corners[0], [3., -20.]);
    let zero = text(&bank, json!("\u{200b}A"), 1, false);
    assert_eq!(zero.parts.len(), 1, "零宽字符不能让首个宽字形提前断行");
    let styled = text(
        &bank,
        json!({"text":"AD","bold":true,"italic":true,"underlined":true,"strikethrough":true}),
        90,
        true,
    );
    assert_eq!(
        styled.parts.len(),
        40,
        "正文与描边字形各两笔，装饰线仅正文发射"
    );
    assert!(styled.parts[..32]
        .iter()
        .all(|part| part.outline && part.depth == 0.));
    assert!(styled.parts[32..36]
        .iter()
        .all(|part| !part.outline && part.depth == 0.));
    assert!(styled.parts[36..]
        .iter()
        .all(|part| !part.outline && part.depth == 0.01));
    let minmax = |part: &sign_text::Part, axis: usize| {
        part.corners
            .iter()
            .map(|p| p[axis])
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), x| {
                (a.min(x), b.max(x))
            })
    };
    assert_eq!(minmax(&styled.parts[36], 1), (-16.5, -15.5));
    assert_eq!(minmax(&styled.parts[37], 1), (-12., -11.));
    let outlined = text(&bank, json!("D"), 90, true);
    assert_eq!(outlined.parts.len(), 9);
    let normal = outlined.parts[8].corners[0];
    assert_eq!(
        outlined.parts[0].corners[0],
        [normal[0] - 0.5, normal[1] - 0.5]
    );
    let invalid = Fixture::new();
    invalid.put(
        "assets/minecraft/font/default.json",
        br#"{"providers":[]} trailing"#,
    );
    assert!(TextureBank::open_stack_java_controlled(
        std::slice::from_ref(&invalid.path),
        &Default::default()
    )
    .is_err());
    let overlay = Fixture::new();
    overlay.put(
        "assets/minecraft/font/default.json",
        br#"{"providers":[{"type":"space","advances":{"A":12}}]}"#,
    );
    let stacked = TextureBank::open_stack_java_controlled(
        &[fixture.path.clone(), overlay.path.clone()],
        &crate::textures::LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        Fonts::new(&stacked, Language::English)
            .glyph("minecraft:default", 'A', &|| false)
            .unwrap()
            .unwrap()
            .advance,
        12.
    );
    assert!(
        text(
            &bank,
            json!({"translate":"sign.test","with":["A"]}),
            90,
            false
        )
        .parts
        .len()
            > 1
    );
}
#[test]
fn models_textures_and_source_frames() {
    let fixture = assets();
    let bank = fixture.open();
    let mut fonts = Fonts::new(&bank, Language::English);
    for wood in WOODS {
        for (suffix, properties, mount) in [
            ("sign", "rotation=0", Mount::Standing),
            ("wall_sign", "facing=north", Mount::Wall),
            ("hanging_sign", "rotation=0,attached=false", Mount::Hanging),
            ("hanging_sign", "rotation=0,attached=true", Mount::Hanging),
            ("wall_hanging_sign", "facing=north", Mount::WallHanging),
        ] {
            let state = format!("minecraft:{wood}_{suffix}[{properties}]");
            let mut source = source(&state, json!("AD中"), true, "red");
            source.back.messages[0] = json!({"text":"B","color":"#00ff00"});
            assert_eq!(source.mount, mount);
            assert!(source.waxed);
            let prepared =
                sign_geometry::prepare(&source, &bank, &mut fonts, Language::English, &|| false)
                    .unwrap();
            let again =
                sign_geometry::prepare(&source, &bank, &mut fonts, Language::English, &|| false)
                    .unwrap();
            assert!(Arc::ptr_eq(&prepared, &again));
            assert!(
                sign_geometry::prepare(&source, &bank, &mut fonts, Language::English, &|| true)
                    .is_err()
            );
            let textures = prepared
                .textures()
                .map(|t| (t.id.clone(), Arc::clone(t)))
                .collect();
            let atlas = crate::entity_geometry::append_runtime_cached(
                bank.atlas().unwrap(),
                textures,
                &|| false,
            )
            .unwrap();
            let quads = sign_geometry::quads(&source, &prepared, &bank, &atlas, true).unwrap();
            let body = sign_geometry::quads(&source, &prepared, &bank, &atlas, false).unwrap();
            assert!(!body.is_empty() && quads.len() > body.len());
            assert!(quads
                .iter()
                .flat_map(|q| q.xyz)
                .flatten()
                .all(f32::is_finite));
            assert!(quads[body.len()..]
                .iter()
                .all(|q| q.mode & crate::material_mode::FONT_TEXT != 0));
            assert!(quads
                .iter()
                .any(|q| q.mode & crate::material_mode::SIGN_NEAR_OUTLINE != 0));
            assert!(!crate::material_mode::visible_from(
                crate::material_mode::SIGN_NEAR_OUTLINE,
                [0.; 3],
                [16., 0., 0.]
            ));
            assert!(crate::material_mode::visible_from(
                crate::material_mode::SIGN_NEAR_OUTLINE,
                [0.; 3],
                [15.9, 0., 0.]
            ));
        }
    }
    // 使用真正源标记、生产图集和拾取索引，不将文字冒充地形方块。
    let source = source(
        "minecraft:oak_sign[rotation=0]",
        json!("AB中"),
        true,
        "black",
    );
    let marker = mcedit2_world::scene_annotations::Marker {
        kind: mcedit2_world::scene_annotations::Kind::BlockEntity,
        id: Some("minecraft:sign".into()),
        position: [0., 0., 0.],
        path: "block_entities/0".into(),
        source: "region/r.0.0.mca".into(),
        chunk: (0, 0),
        sha256: "source-sign-nbt".into(),
        fields: 4,
        bytes: 200,
        tick: None,
        priority: None,
        block_state: Some(Arc::new(source.state.clone())),
        visual_error: None,
        visual: None,
        geometry: None,
        block_visual: Some(Arc::new(
            mcedit2_world::scene_annotations::BlockVisual::Sign(source),
        )),
    };
    let snapshot = Arc::new(crate::scene_layers::Snapshot::build(
        (0, 0),
        1,
        std::iter::once(Arc::new(
            mcedit2_world::scene_annotations::ChunkAnnotations {
                lighting: Default::default(),
                markers: vec![marker],
                diagnostics: Vec::new(),
            },
        )),
        None,
    ));
    let frame = crate::entity_geometry::Frame::build(
        Arc::clone(&snapshot),
        &bank,
        Some(3955),
        true,
        &|| false,
    )
    .unwrap();
    assert_eq!((frame.rendered, frame.missing), (1, 0));
    assert!(frame.diagnostics.is_empty());
    assert!(frame.manages_block_visual(
        [0, 0, 0],
        &parse_state("minecraft:oak_sign[rotation=0]").unwrap(),
        &bank,
        &frame.atlas
    ));
    let chunk = &frame.chunks[0];
    let hit = chunk
        .picking
        .cast(
            crate::spectator::Camera {
                position: [0., 0.34, -3.],
                yaw: 0.,
                pitch: 0.,
                zoom: 1.,
                perspective: true,
            },
            iced::Point::new(64., 64.),
            crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(128., 128.)),
            Some(&frame.atlas),
            crate::gpu_mesh::Quality::Fast,
            f64::INFINITY,
        )
        .unwrap();
    assert_eq!(
        frame
            .marker((chunk.key.0, chunk.key.1), hit.group_index)
            .unwrap()
            .marker()
            .sha256,
        "source-sign-nbt"
    );
    assert!(chunk
        .quads()
        .iter()
        .any(|q| q.mode & crate::material_mode::FONT_TEXT != 0));
    // 平面投影走生产联合深度 / 光栅和同一源快照。
    let shared = Arc::new(frame);
    let mapping = crate::projected_entities::Mapping::Orthogonal {
        origin: [0., 0., -3.],
        screen: [64., 100.],
        right: [1., 0., 0.],
        down: [0., -1., 0.],
        depth: [0., 0., -1.],
        scale: 100.,
    };
    let rendered = crate::projected_entities::render_grid(
        Arc::clone(&shared),
        mapping,
        ([-1.; 3], [1.; 3]),
        Vec::new(),
        iced::Size::new(128., 128.),
        crate::gpu_mesh::Quality::Fast,
        &crate::lighting::Table::unlit(),
        &|| false,
    )
    .unwrap()
    .unwrap()
    .0;
    assert!(rendered
        .vertices
        .iter()
        .any(|v| v.mode & crate::material_mode::FONT_TEXT != 0));
    assert!(rendered
        .projected
        .as_ref()
        .unwrap()
        .triangles
        .iter()
        .all(|triangle| triangle.marker.marker().sha256 == "source-sign-nbt"));
    // 待放置结构使用相同文字资源和 NBT，保持无磁盘所有权的独立几何。
    let front = std::array::from_fn(|i| if i == 0 { json!("AB中") } else { json!("") });
    let raw = Value::Compound(vec![
        field("id", Value::String("minecraft:sign".into())),
        field("front_text", side(front, true, "black")),
    ])
    .payload()
    .unwrap();
    let blocks = [parse_state("minecraft:oak_sign[rotation=0]").unwrap()];
    let block_entities = [mcedit2_world::clipboard::OpaqueBlockEntity {
        position: [0; 3],
        compound: raw,
    }];
    let volume = crate::pending_import::Volume {
        size: [1; 3],
        offset: [0; 3],
        blocks: &blocks,
        mask: None,
        block_entities: &block_entities,
        entities: &[],
        biomes: None,
        numeric_biomes: None,
        data_version: Some(3955),
        painting_variants: &std::collections::BTreeMap::new(),
        metadata: false,
        map_world: None,
        item_environment: None,
    };
    let scene = crate::textures::java_obj_tests::scene(Arc::clone(&bank));
    let preview = crate::pending_import::build_geometry_with_budget(
        &volume,
        [0; 3],
        &scene,
        false,
        Language::English,
        Default::default(),
        &|| false,
    )
    .unwrap();
    assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);
    assert!(preview
        .mesh
        .chunks
        .iter()
        .flat_map(|c| c.quads().iter())
        .any(|q| q.mode & crate::material_mode::FONT_TEXT != 0));
    assert!(preview
        .mesh
        .chunks
        .iter()
        .flat_map(|c| c.picking.source_groups())
        .all(|group| group.owner.is_none()));
    let plain = crate::entity_geometry::Frame::build(snapshot, &bank, Some(3955), false, &|| false)
        .unwrap();
    assert!(plain
        .chunks
        .iter()
        .flat_map(|c| c.quads().iter())
        .all(|q| q.mode & crate::material_mode::FONT_TEXT == 0));
}
