//! 地图原生顶点、三版纹理、所有显示方向及背景资源交接集中验证。
use super::*;
use crate::{map_fixture as f, map_geometry as geometry};
use image::ImageEncoder;
use mcedit2_world::map_decorations as maps;

fn png(
    fixture: &crate::textures::java_obj_tests::Fixture,
    path: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    fixture.put(path, &bytes);
}
pub(crate) fn assets(version: i32) -> crate::textures::java_obj_tests::Fixture {
    let fixture = super::frame_tests::assets(version);
    for (i, (_, definition)) in maps::catalog(version).unwrap().iter().enumerate() {
        let color = [40 + (i % 10) as u8 * 15, 20, 220, 255];
        if let Some(asset) = &definition.asset_id {
            let (ns, path) = asset.split_once(':').unwrap();
            let rgba: Vec<u8> = (0..64)
                .flat_map(|p| if p % 8 < 4 { color } else { [0; 4] })
                .collect();
            png(
                &fixture,
                &format!("assets/{ns}/textures/map/decorations/{path}.png"),
                8,
                8,
                &rgba,
            );
        }
    }
    png(
        &fixture,
        "assets/minecraft/textures/map/map_icons.png",
        128,
        128,
        &[255u8, 100, 40, 255].repeat(128 * 128),
    );
    png(
        &fixture,
        "assets/minecraft/textures/font/ascii.png",
        128,
        128,
        &[255u8; 4].repeat(128 * 128),
    );
    png(
        &fixture,
        "assets/demo/textures/map/glyph.png",
        48,
        8,
        &[255u8; 4].repeat(48 * 8),
    );
    fixture.put("assets/minecraft/font/default.json",br#"{"providers":[{"type":"bitmap","file":"demo:map/glyph.png","height":8,"ascent":7,"chars":["Marker"]}]}"#);
    fixture
}
pub(super) fn complete() {
    let reference = f::reference(3955);
    let catalog = maps::catalog(3955).unwrap();
    for case in reference["geometry"].as_array().unwrap() {
        let marker = maps::Decoration {
            key: "native".into(),
            definition: catalog[case["type"].as_str().unwrap()].clone(),
            x: -17,
            y: 39,
            rotation: case["rotation"].as_u64().unwrap() as u8,
            name: None,
        };
        let vertices = case["vertices"].as_array().unwrap();
        let shown = !case["on_frame"].as_bool().unwrap() || marker.definition.show_on_frame;
        assert_eq!(vertices.len(), if shown { 8 } else { 4 });
        if shown {
            let actual = geometry::marker_points(&marker, 0);
            for (p, v) in actual.iter().zip(&vertices[4..]) {
                for i in 0..3 {
                    assert!(
                        (f64::from(p[i]) - v[i].as_f64().unwrap()).abs() < 0.00003,
                        "{} {}: {p:?} / {v}",
                        case["type"],
                        case["rotation"]
                    );
                }
            }
            for (v, uv) in vertices[4..]
                .iter()
                .zip([[0., 0.], [1., 0.], [1., 1.], [0., 1.]])
            {
                assert_eq!([v[3].as_f64().unwrap(), v[4].as_f64().unwrap()], uv);
            }
        }
    }
    for version in [1343, 3465, 3955] {
        let assets = assets(version);
        let bank = assets.open();
        let fixture = crate::metadata_fixture::Fixture::new(version);
        f::write(&fixture.root, version, f::data(version));
        let image = mcedit2_world::map_data::read(&fixture.root, 7, Some(version)).unwrap();
        let native =
            geometry::prepare(&image, &image.decorations, false, &bank, &|| false).unwrap();
        assert!(native.faces.len() >= 1 + image.decorations.len());
        assert!(native
            .faces
            .iter()
            .skip(1)
            .all(|q| crate::material_mode::pass(q.mode.unwrap())
                == crate::material_mode::Pass::Tripwire));
        let preview = geometry::preview(&image, &bank, &|| false).unwrap();
        let iced::advanced::image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } = preview
        else {
            panic!("地图预览没有真实 RGBA 像素")
        };
        assert_eq!((width, height), (512, 512));
        assert!(
            pixels.chunks_exact(4).all(|p| p[3] == 255),
            "完整不透明底图必须覆盖整个预览"
        );
        for (x, y) in [(0, 0), (511, 0), (0, 511), (511, 511)] {
            assert_eq!(
                &pixels[(y * 512 + x) * 4..(y * 512 + x) * 4 + 4],
                &image.rgba[..4]
            );
        }
        if version != 1343 {
            assert!(
                pixels.chunks_exact(4).any(|p| p != &image.rgba[..4]),
                "标记和名称不能丢失为纯底图"
            );
        }
        assert!(geometry::prepare(&image, &image.decorations, false, &bank, &|| true).is_err());
        for (id, definition) in maps::catalog(version).unwrap() {
            let item = f::decorated_item(version, id, "custom");
            let mut item = item;
            f::change(
                f::item_decoration(&mut item, version),
                "x",
                mcedit2_world::nbt_document::Value::Double(-12.25),
            );
            let mut markers = Vec::new();
            maps::from_item(
                &mut markers,
                &item,
                version,
                image.center,
                image.scale,
                image.unlimited_tracking,
            )
            .unwrap();
            assert_eq!(markers[0].x, -24);
            let map = geometry::prepare(&image, &markers, true, &bank, &|| false).unwrap();
            assert_eq!(map.faces.len(), 1 + usize::from(definition.show_on_frame));
            if definition.show_on_frame {
                let face = &map.faces[1];
                assert!(bank.get(&face.texture).is_some());
                assert!(face.xyz.iter().all(|p| p[2] < map.faces[0].xyz[0][2]));
            }
        }
        // 地图、标签及装饰共用六向 / 八档旋转和不可见框的实际来源。
        for facing in if version == 1343 {
            mcedit2_world::hanging::HORIZONTAL.to_vec()
        } else {
            (0..6).collect()
        } {
            for rotation in 0..8 {
                let mut value = crate::hanging_fixture::frame(
                    version,
                    facing,
                    rotation,
                    true,
                    7,
                    [-19, 72, -17],
                );
                let item = f::decorated_item(version, "minecraft:target_x", "custom");
                f::change(&mut value, "Item", item);
                let prepared = crate::frame_geometry::prepare(
                    &crate::hanging_fixture::visual(&value, version),
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
                let quads = prepared.quads(&atlas, true).unwrap();
                assert!(quads.len() > 23);
                assert!(quads
                    .iter()
                    .all(|q| q.xyz.iter().flatten().all(|p| p.is_finite())));
            }
        }
        if version == 3955 {
            let mut custom = maps::catalog(version).unwrap()["minecraft:target_x"].clone();
            custom.asset_id = Some("demo:unknown_shape".into());
            png(
                &assets,
                "assets/demo/textures/map/decorations/unknown_shape.png",
                3,
                11,
                &[20u8, 230, 40, 255].repeat(33),
            );
            let changed = assets.open();
            let marker = maps::Decoration {
                key: "custom".into(),
                definition: custom,
                x: 0,
                y: 0,
                rotation: 5,
                name: None,
            };
            let rendered = geometry::prepare(
                &image,
                std::slice::from_ref(&marker),
                true,
                &changed,
                &|| false,
            )
            .unwrap();
            assert_eq!(
                rendered.faces[1].texture,
                "demo:map/decorations/unknown_shape"
            );
            assert!(geometry::prepare(&image, &[marker], true, &bank, &|| false)
                .err()
                .unwrap()
                .contains("unknown_shape"));
        }
    }
}

/// 复用已经打开的真实客户端，集中核对全部地图素材及生产光栅化。
pub(super) fn official_bank(bank: &TextureBank) {
    let version = bank.item_data_version().unwrap();
    let fixture = crate::metadata_fixture::Fixture::new(version);
    f::write(&fixture.root, version, f::data(version));
    let mut image = mcedit2_world::map_data::read(&fixture.root, 7, Some(version)).unwrap();
    for (i, definition) in maps::catalog(version).unwrap().values().enumerate() {
        image.decorations.push(maps::Decoration {
            key: format!("official-{i}"),
            definition: definition.clone(),
            x: ((i % 8) as i32 * 24 - 96) as i8,
            y: ((i / 8) as i32 * 24 - 48) as i8,
            rotation: (i % 16) as u8,
            name: None,
        });
    }
    let rendered = geometry::prepare(&image, &image.decorations, false, bank, &|| false).unwrap();
    assert!(rendered.faces.len() >= 1 + maps::catalog(version).unwrap().len());
    let iced::advanced::image::Handle::Rgba { pixels, .. } =
        geometry::preview(&image, bank, &|| false).unwrap()
    else {
        unreachable!()
    };
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| *p != &image.rgba[..4])
            .count()
            > 10
    );
    eprintln!(
        "地图真实客户端 {version}：{} 个定义及实际素材完成生产光栅化",
        maps::catalog(version).unwrap().len()
    );
}
