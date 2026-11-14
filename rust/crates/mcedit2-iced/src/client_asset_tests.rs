//! 通用本机资产索引、字体来源与像素验证，集中并入已有资源类别。
use super::*;
use crate::{font_glyphs::Fonts, language::Language, textures::java_obj_tests::Fixture};
use image::ImageEncoder;
use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
};

fn zip(files: BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(&bytes).unwrap();
    }
    archive.finish().unwrap().into_inner()
}
fn image(color: [u8; 4]) -> Vec<u8> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[color, [0, 0, 255, 255]].concat(),
            1,
            2,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    png
}
fn objects(f: &Fixture, files: &BTreeMap<String, Vec<u8>>) {
    let mut entries: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for (i, (name, bytes)) in files.iter().enumerate() {
        let hash = format!("{:040x}", i + 1);
        f.put(&format!("assets/objects/{}/{hash}", &hash[..2]), bytes);
        entries.insert(
            name.clone(),
            serde_json::json!({"hash":hash,"size":bytes.len()}),
        );
    }
    // 不支持的声音不要求实际缓存对象；建立索引不能变成读取全部音频。
    entries.insert(
        "minecraft/sounds/unused.ogg".into(),
        serde_json::json!({"hash":"ffffffffffffffffffffffffffffffffffffffff","size":10}),
    );
    f.put(
        "assets/indexes/correct.json",
        &serde_json::to_vec(&serde_json::json!({"objects":entries})).unwrap(),
    );
    f.put("assets/indexes/other.json", b"not this version");
}
fn installation(prism: bool) -> (Fixture, PathBuf, BTreeMap<String, Vec<u8>>) {
    let f = Fixture::new();
    let path = if prism {
        "libraries/com/mojang/minecraft/1.21.1/renamed.jar"
    } else {
        "versions/1.21.1/1.21.1.jar"
    };
    let meta = if prism {
        "meta/net.minecraft/1.21.1.json"
    } else {
        "versions/1.21.1/1.21.1.json"
    };
    f.put(meta, br#"{"assetIndex":{"id":"correct"}}"#);
    let provider = serde_json::json!({"providers":[{"type":"unihex","hex_file":"test:font/all.zip","size_overrides":[{"from":"通","to":"通","left":0,"right":15}]}]});
    let font = zip(BTreeMap::from([(
        "glyphs.hex".into(),
        format!(
            "901A:{}\n20000:{}\n",
            "0F00".repeat(16),
            "FFFFFFFF".repeat(16)
        )
        .into_bytes(),
    )]));
    let mut files = BTreeMap::from([
        (
            "minecraft/font/include/unifont.json".into(),
            serde_json::to_vec(&provider).unwrap(),
        ),
        ("test/font/all.zip".into(), font),
        (
            "test/lang/zh_cn.json".into(),
            r#"{"block.test.indexed":"通用资源"}"#.as_bytes().to_vec(),
        ),
        (
            "test/textures/block/tile.png".into(),
            image([255, 0, 0, 255]),
        ),
        (
            "test/textures/block/tile.png.mcmeta".into(),
            br#"{"animation":{"frametime":1}}"#.to_vec(),
        ),
        (
            "test/blockstates/indexed.json".into(),
            br#"{"variants":{"":{"model":"test:block/indexed"}}}"#.to_vec(),
        ),
    ]);
    let faces: [(&str, serde_json::Value); 6] = ["up", "down", "east", "west", "north", "south"]
        .map(|s| (s, serde_json::json!({"texture":"#all"})));
    files.insert("test/models/block/indexed.json".into(),serde_json::to_vec(&serde_json::json!({"textures":{"all":"test:block/tile"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":faces.into_iter().collect::<BTreeMap<_,_>>()}]})).unwrap());
    objects(&f, &files);
    f.put(
        path,
        &zip(BTreeMap::from([
            ("net/minecraft/client/main/Main.class".into(), Vec::new()),
            (
                "version.json".into(),
                br#"{"id":"1.21.1","name":"1.21.1","world_version":3955,"protocol_version":0}"#
                    .to_vec(),
            ),
            (
                "assets/minecraft/font/default.json".into(),
                br#"{"providers":[{"type":"reference","id":"minecraft:include/unifont"}]}"#
                    .to_vec(),
            ),
            // 同一个原版包内这些副本必须被索引对象完整替换，不能仍解析坏副本。
            (
                "assets/minecraft/font/include/unifont.json".into(),
                br#"{"providers":"wrong"}"#.to_vec(),
            ),
            (
                "assets/test/textures/block/tile.png".into(),
                b"wrong PNG".to_vec(),
            ),
            (
                "assets/test/models/block/indexed.json".into(),
                b"wrong model".to_vec(),
            ),
        ])),
    );
    let client = f.path.join(path);
    (f, client, files)
}
fn open(paths: &[PathBuf]) -> TextureBank {
    TextureBank::open_stack_java_controlled(
        paths,
        &LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap()
}
fn glyph(bank: &TextureBank, ch: char) -> Arc<crate::font_glyphs::Glyph> {
    Fonts::new(bank, Language::Chinese)
        .glyph("minecraft:default", ch, &|| false)
        .unwrap()
        .expect("字形必须来自实际资源，不使用缺字方框")
}
fn text_pixels(bank: &TextureBank, text: &str) -> crate::canvas_raster::Raster {
    let side = mcedit2_world::sign::Side {
        messages: [
            serde_json::json!(text),
            serde_json::json!(""),
            serde_json::json!(""),
            serde_json::json!(""),
        ],
        ..Default::default()
    };
    let mut fonts = Fonts::new(bank, Language::Chinese);
    let text = crate::sign_text::prepare(
        &side,
        bank,
        &mut fonts,
        Language::Chinese,
        100,
        0,
        0xffffff,
        None,
        &|| false,
    )
    .unwrap()
    .unwrap();
    let materials = text
        .parts
        .iter()
        .map(|p| (p.texture.id.clone(), Arc::clone(&p.texture)))
        .collect();
    let atlas =
        crate::entity_geometry::append_runtime_cached(bank.atlas().unwrap(), materials, &|| false)
            .unwrap();
    let mut vertices = Vec::new();
    for part in text.parts {
        let tile = &atlas.entries[&part.texture.id];
        let [u0, v0, u1, v1] = part.uv;
        let uv = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
        for i in [0, 1, 2, 0, 2, 3] {
            vertices.push(crate::gpu_mesh::Vertex {
                xy: [part.corners[i][0] / 64., 0.5 - part.corners[i][1] / 16.],
                depth: 0.5,
                rgba: [1.; 4],
                uv: tile.sample(uv[i][0], uv[i][1]),
                mode: 2 | 32 | crate::material_mode::NO_MIPMAP,
                xyz: [0.; 3],
                world_mode: 0,
                atlas_page: tile.page,
                light: crate::lighting::FULLBRIGHT,
            });
        }
    }
    let raster = crate::canvas_raster::render(
        &vertices,
        Some(&atlas),
        iced::Size::new(512., 128.),
        crate::gpu_mesh::Quality::Fast,
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert!(raster.rgba.chunks_exact(4).filter(|p| p[3] > 0).count() > 100);
    raster
}
pub(super) fn complete() {
    for prism in [false, true] {
        let (f, client, mut files) = installation(prism);
        let bank = open(std::slice::from_ref(&client));
        assert_eq!(
            bank.block_name("test:indexed", Language::Chinese),
            Some("通用资源")
        );
        assert_eq!(bank.animation_count(), 1);
        assert_eq!(
            bank.item_tile("test:tile").unwrap().pixel(0., 0.),
            [255, 0, 0, 255]
        );
        let original = glyph(&bank, '通');
        assert_eq!(
            (original.width, original.height, original.advance),
            (16, 16, 9.)
        );
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(
                    original.rgba[(y * 16 + x) * 4 + 3],
                    if (4..8).contains(&x) { 255 } else { 0 }
                );
            }
        }
        assert_eq!(glyph(&bank, '𠀀').width, 32);
        let binary = bank.font_resources().binary("test:font/all.zip").unwrap();
        let cache = &bank.font_resources().unihex;
        let first = cache
            .load("test:font/all.zip", binary, |_| Ok(()), &|| false)
            .unwrap();
        let second = cache
            .load("test:font/all.zip", binary, |_| Ok(()), &|| false)
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(cache
            .load(
                "test:font/all.zip",
                binary,
                |_| Err("预算拒绝".into()),
                &|| false
            )
            .is_err());
        assert!(cache
            .load("test:font/all.zip", binary, |_| Ok(()), &|| true)
            .is_err());
        let _ = text_pixels(&bank, "通𠀀");
        let overlay = Fixture::new();
        overlay.put(
            "assets/test/font/all.zip",
            &zip(BTreeMap::from([(
                "override.hex".into(),
                format!(
                    "901A:{}\n20000:{}\n",
                    "8000".repeat(16),
                    "FFFFFFFF".repeat(16)
                )
                .into_bytes(),
            )])),
        );
        overlay.put(
            "assets/test/lang/zh_cn.json",
            r#"{"block.test.indexed":"上层资源"}"#.as_bytes(),
        );
        let changed = open(&[client.clone(), overlay.path.clone()]);
        assert_eq!(
            changed.block_name("test:indexed", Language::Chinese),
            Some("上层资源")
        );
        assert_ne!(glyph(&changed, '通').rgba, original.rgba);
        assert_eq!(glyph(&bank, '通').rgba, original.rgba);
        // 同一完整资源流程同时覆盖标准安装与实际重命名的 Prism 客户端。
        let theme = BTreeMap::from([
            (
                "pack.mcmeta".into(),
                r#"{"pack":{"pack_format":34,"description":"集成资源"}}"#
                    .as_bytes()
                    .to_vec(),
            ),
            (
                "assets/test/textures/block/tile.png".into(),
                image([0, 255, 0, 255]),
            ),
            (
                "assets/test/lang/zh_cn.json".into(),
                r#"{"block.test.indexed":"内置资源"}"#.as_bytes().to_vec(),
            ),
        ]);
        files.insert(
            "minecraft/resourcepacks/editor_theme.zip".into(),
            zip(theme.clone()),
        );
        objects(&f, &files);
        let builtin = crate::client_assets::builtin_pack(&client, "editor_theme")
            .unwrap()
            .unwrap();
        let packed = open(&[client.clone(), builtin]);
        assert_eq!(
            packed.item_tile("test:tile").unwrap().pixel(0., 0.),
            [0, 255, 0, 255]
        );
        assert_eq!(
            packed.block_name("test:indexed", Language::Chinese),
            Some("内置资源")
        );
        assert!(crate::client_assets::builtin_pack(&client, "not_declared")
            .unwrap()
            .is_none());
        let mut bundle = BTreeMap::from([("packs/theme.zip".into(), zip(theme.clone()))]);
        bundle.extend(
            theme
                .into_iter()
                .map(|(name, bytes)| (format!("bundles/theme/{name}"), bytes)),
        );
        overlay.put("bundle.zip", &zip(bundle));
        for member in ["packs/theme.zip", "bundles/theme/"] {
            let nested =
                crate::pack_sources::nested(&overlay.path.join("bundle.zip"), member).unwrap();
            let packed = open(&[client.clone(), nested]);
            assert_eq!(
                packed.item_tile("test:tile").unwrap().pixel(0., 0.),
                [0, 255, 0, 255]
            );
            assert_eq!(
                packed.block_name("test:indexed", Language::Chinese),
                Some("内置资源")
            );
        }
        let index = crate::client_assets::discover(&client, Some("1.21.1"))
            .unwrap()
            .unwrap();
        let path = &index.objects["assets/test/font/all.zip"].file;
        std::fs::remove_file(path).unwrap();
        let error = TextureBank::open_stack_java_controlled(
            std::slice::from_ref(&client),
            &LoadControl::default(),
        )
        .unwrap_err();
        assert!(error.contains("test/font/all.zip") && error.contains("correct.json"));
        objects(&f, &files);
        std::fs::write(path, b"bad").unwrap();
        assert!(TextureBank::open_stack_java_controlled(
            std::slice::from_ref(&client),
            &LoadControl::default()
        )
        .unwrap_err()
        .contains("索引声明"));
        objects(&f, &files);
        files.insert(
            "minecraft/font/include/unifont.json".into(),
            b"invalid JSON".to_vec(),
        );
        objects(&f, &files);
        assert!(TextureBank::open_stack_java_controlled(
            std::slice::from_ref(&client),
            &LoadControl::default()
        )
        .is_err());
    }
    official();
}
fn official() {
    let Some(paths) = std::env::var_os("MCEDIT2_ITEM_CLIENTS") else {
        return;
    };
    for path in std::env::split_paths(&paths) {
        let bank = open(std::slice::from_ref(&path));
        for ch in "通用资源".chars() {
            let glyph = glyph(&bank, ch);
            assert!(glyph.rgba.chunks_exact(4).any(|p| p[3] > 0));
        }
        let raster = text_pixels(&bank, "通用资源");
        eprintln!(
            "客户端 {:?}：实际外部资产、CJK 字形和生产 Canvas 文字像素通过",
            bank.item_data_version()
        );
        if bank.item_data_version() == Some(3955) {
            if let Some(output) = std::env::var_os("MCEDIT2_FONT_PREVIEW") {
                image::save_buffer_with_format(
                    output,
                    &raster.rgba,
                    raster.width,
                    raster.height,
                    image::ColorType::Rgba8,
                    image::ImageFormat::Png,
                )
                .unwrap();
            }
        }
    }
}
