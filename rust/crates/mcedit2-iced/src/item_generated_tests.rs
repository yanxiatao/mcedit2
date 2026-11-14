//! 既有资源类别内的完整轮廓验证：真实 PNG、动画元数据、原客户端元素与 UV、缓存和取消。
use super::*;
use crate::textures::java_obj_tests::Fixture;
use image::ImageEncoder;

fn signatures(quads: &[Quad]) -> Vec<[[i32; 5]; 4]> {
    let mut result: Vec<_> = quads
        .iter()
        .map(|q| {
            assert_eq!(q.tint, Some(3));
            assert!(q.winding == -1. || q.winding == 1.);
            assert!(q.cull);
            let mut corners: [[i32; 5]; 4] = std::array::from_fn(|i| {
                [
                    q.xyz[i][0],
                    q.xyz[i][1],
                    q.xyz[i][2],
                    q.uv[i][0],
                    q.uv[i][1],
                ]
                .map(|v| (v * 1_000_000.).round() as i32)
            });
            if q.winding < 0. {
                corners.swap(1, 3);
            }
            (0..4)
                .map(|rotation| {
                    let mut ordered = corners;
                    ordered.rotate_left(rotation);
                    ordered
                })
                .min()
                .unwrap()
        })
        .collect();
    result.sort();
    result
}
pub(crate) fn complete() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../tests/fixtures/generated-item-1.21.1.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let fixture = Fixture::new();
        let w = case["width"].as_u64().unwrap() as u32;
        let h = case["height"].as_u64().unwrap() as u32;
        let frames = case["alpha"].as_array().unwrap();
        let rgba: Vec<_> = frames
            .iter()
            .flat_map(|frame| {
                frame
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .flat_map(|(i, alpha)| {
                        [50 + (i * 11) as u8, 190, 100, alpha.as_u64().unwrap() as u8]
                    })
            })
            .collect();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &rgba,
                w,
                h * frames.len() as u32,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        fixture.put("assets/demo/textures/item/source.png", &png);
        fixture.put(
            "assets/demo/models/item/source.json",
            br#"{"parent":"builtin/generated","textures":{"layer0":"demo:item/source"}}"#,
        );
        fixture.put(
            "version.json",
            br#"{"id":"1.21.1","name":"1.21.1","world_version":3955,"protocol_version":767}"#,
        );
        if frames.len() > 1 {
            fixture.put("assets/demo/textures/item/source.png.mcmeta",&serde_json::to_vec(&serde_json::json!({"animation":{"width":w,"height":h,"frames":case["indices"]}})).unwrap());
        }
        let bank = fixture.open();
        let mut scene = bank.item_scene("demo:source", &BTreeMap::new()).unwrap();
        scene.layers[0].1 = 3;
        append(&bank, &mut scene, true, &|| false).unwrap();
        let mut native = Scene {
            builtin: false,
            quads: Vec::new(),
            layers: Vec::new(),
            transform: crate::root_transform::Affine::IDENTITY,
            flat: true,
            gui_3d: false,
            display_scale: [1.; 3],
        };
        crate::resource_models::items::append_elements(&mut native,serde_json::json!({"textures":{"layer3":"demo:item/source"},"elements":case["elements"]})).unwrap();
        let shape = signatures(&scene.quads);
        assert_eq!(
            shape,
            signatures(&native.quads),
            "原版轮廓 / UV：{}",
            case["name"]
        );
        let baked: Vec<Quad> = case["quads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|face| {
                let vertices: Vec<[f32; 5]> =
                    serde_json::from_value(face["vertices"].clone()).unwrap();
                let mut q = quad(
                    std::array::from_fn(|i| [vertices[i][0], vertices[i][1], vertices[i][2]]),
                    std::array::from_fn(|i| [vertices[i][3], vertices[i][4]]),
                    "demo:item/source",
                    face["tint"].as_u64().unwrap() as u32,
                );
                q.winding = 1.;
                q
            })
            .collect();
        assert_eq!(
            shape,
            signatures(&baked),
            "原客户端实际烘焙顶点 / UV / 绕序：{}",
            case["name"]
        );
        assert!(scene.quads.len() <= 2 + 2 * (w + h) as usize);
        let pixels = match bank.item_animation("demo:item/source") {
            Some(sprite) => Pixels::Animated(sprite),
            None => Pixels::Static(bank.get("demo:item/source").unwrap()),
        };
        let cached = bank
            .generated_items()
            .get(&bank, "demo:item/source", &pixels, &|| false)
            .unwrap();
        for tick in [0, 1, 2, 7] {
            let snapshot = bank
                .with_runtime_atlas(Arc::new(bank.atlas().unwrap().snapshot_at(tick).unwrap()))
                .unwrap();
            let mut next = snapshot
                .item_scene("demo:source", &BTreeMap::new())
                .unwrap();
            next.layers[0].1 = 3;
            append(&snapshot, &mut next, true, &|| false).unwrap();
            assert_eq!(
                signatures(&next.quads),
                shape,
                "动画不能重建不完整轮廓：{} / {tick}",
                case["name"]
            );
            let reused = snapshot
                .generated_items()
                .get(&snapshot, "demo:item/source", &pixels, &|| false)
                .unwrap();
            assert!(Arc::ptr_eq(&cached, &reused));
        }
        assert!(Cache::default()
            .get(&bank, "demo:item/source", &pixels, &|| true)
            .is_err());
        // 平面 GUI 不输出严格侧向的轮廓，fixed / 三维物品必须保留。
        let mut flat = bank.item_scene("demo:source", &BTreeMap::new()).unwrap();
        append(&bank, &mut flat, false, &|| false).unwrap();
        assert_eq!(flat.quads.len(), 2);
    }
}
