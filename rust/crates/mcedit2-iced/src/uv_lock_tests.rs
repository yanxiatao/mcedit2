//! 使用真实客户端 FaceBakery 的输出核对生产模型解释，而非复刻实现自证。
use super::*;
use image::ImageEncoder;
use serde_json::json;
use std::sync::Arc;

const FACES: [(Face, &str); 6] = [
    (Face::Bottom, "down"),
    (Face::Top, "up"),
    (Face::NegZ, "north"),
    (Face::PosZ, "south"),
    (Face::NegX, "west"),
    (Face::PosX, "east"),
];

fn uv_lock_matches_actual_java_client_for_roots_crops_mirrors_and_all_variant_turns() {
    let reference: Value =
        serde_json::from_str(include_str!("../tests/fixtures/uvlock-1.21.1.json")).unwrap();
    let roots = reference["roots"].as_array().unwrap();
    let cases = reference["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3840);
    for case in cases {
        let root = case[0].as_u64().unwrap() as usize;
        let x = case[1].as_i64().unwrap();
        let y = case[2].as_i64().unwrap();
        let (face, name) = FACES[case[3].as_u64().unwrap() as usize];
        let rotation = case[4].as_u64().unwrap();
        let rows = roots[root]
            .as_array()
            .unwrap()
            .chunks_exact(4)
            .map(|r| Value::Array(r.to_vec()))
            .collect::<Vec<_>>();
        let mut bank = ModelBank::default();
        bank.insert(
            "assets/test/blockstates/locked.json",
            &serde_json::to_vec(
                &json!({"variants":{"":{"model":"test:block/locked","x":x,"y":y,"uvlock":true}}}),
            )
            .unwrap(),
        )
        .unwrap();
        bank.insert("assets/test/models/block/locked.json", &serde_json::to_vec(&json!({"transform":{"matrix":rows},"elements":[{"from":[1,2,3],"to":[15,14,13],"faces":{name:{"texture":"test:block/uv","uv":case[5],"rotation":rotation}}}]})).unwrap()).unwrap();
        let state = State {
            name: "test:locked".into(),
            properties: vec![],
        };
        let spec = bank.element_face_at(&state, face, 0, (0, 0, 0)).unwrap();
        let actual = uv_corners(spec.uv, spec.rotation, spec.uvlock);
        let expected_rect = std::array::from_fn(|i| case[6][i].as_f64().unwrap() as f32);
        let mut indices = face.java_uv_indices();
        if bank.parts_at(&state, (0, 0, 0)).unwrap()[0]
            .root
            .is_some_and(|r| r.determinant() < 0.)
        {
            indices.swap(1, 3);
        }
        let expected = uv_corners(expected_rect, case[7].as_u64().unwrap() as u16, indices);
        for (a, b) in actual.iter().flatten().zip(expected.iter().flatten()) {
            assert!((a-b).abs()<0.000_1,"客户端参照不一致 root={root},x={x},y={y},face={face:?},rotation={rotation}: {actual:?} != {expected:?}");
        }
    }
}

fn uv_corners([u0, v0, u1, v1]: [f32; 4], rotation: u16, indices: [u8; 4]) -> [[f32; 2]; 4] {
    let mut c = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
    for _ in 0..rotation / 90 {
        c.rotate_right(1);
    }
    indices.map(|i| c[i as usize])
}

fn variant_geometry_and_inverse_faces_match_actual_java_block_model_rotations() {
    let reference: Value =
        serde_json::from_str(include_str!("../tests/fixtures/uvlock-1.21.1.json")).unwrap();
    for (i, row) in reference["variants"].as_array().unwrap().iter().enumerate() {
        let x = (i / 4 * 90) as i32;
        let y = (i % 4 * 90) as i32;
        let part = Part {
            variant_x: x,
            variant_y: y,
            ..Part::cube(Box3::FULL)
        };
        for p in [[0.125, -0.25, 0.375], [0., 1., 0.], [0., 0., -1.]] {
            let expected: [f32; 3] = std::array::from_fn(|r| {
                (0..3)
                    .map(|c| row[r * 4 + c].as_f64().unwrap() as f32 * p[c])
                    .sum()
            });
            let actual = part.point(p);
            for (a, b) in actual.into_iter().zip(expected) {
                assert!(
                    (a - b).abs() < 0.000_001,
                    "X={x},Y={y}：{actual:?} != {expected:?}"
                );
            }
        }
        for (local, _) in FACES {
            let n = local.normal();
            let p = [n.0 as f32, n.1 as f32, n.2 as f32];
            let expected: [f32; 3] = std::array::from_fn(|r| {
                (0..3)
                    .map(|c| row[r * 4 + c].as_f64().unwrap() as f32 * p[c])
                    .sum()
            });
            let target = FACES
                .iter()
                .find(|(face, _)| {
                    let n = face.normal();
                    expected
                        .into_iter()
                        .zip([n.0, n.1, n.2])
                        .all(|(a, b)| (a - b as f32).abs() < 0.000_001)
                })
                .unwrap()
                .0;
            assert_eq!(target.undo_variant_rotation(x, y), local, "X={x},Y={y}");
        }
    }
}

fn singular_root_is_rejected_for_locked_variants_and_preserved_when_unlocked() {
    let mut bank = ModelBank::default();
    bank.insert("assets/test/models/block/flat.json",br##"{"transform":{"scale":[1,0,1],"origin":"center"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"test:block/uv"}}}]}"##).unwrap();
    bank.insert(
        "assets/test/blockstates/flat.json",
        br#"{"variants":{"":{"model":"test:block/flat","uvlock":true}}}"#,
    )
    .unwrap();
    let state = State {
        name: "test:flat".into(),
        properties: vec![],
    };
    let errors = bank.validate_builtin_loaders();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("不可逆"));
    assert!(bank.parts_at(&state, (0, 0, 0)).is_none());
    assert!(bank
        .element_face_at(&state, Face::Top, 0, (0, 0, 0))
        .is_none());
    assert_eq!(bank.validate_builtin_loaders(), errors);
    bank.insert(
        "assets/test/blockstates/flat.json",
        br#"{"variants":{"":{"model":"test:block/flat","uvlock":false}}}"#,
    )
    .unwrap();
    assert!(bank.validate_builtin_loaders().is_empty());
    assert_eq!(bank.parts_at(&state, (0, 0, 0)).unwrap().len(), 1);
    assert!(bank
        .element_face_at(&state, Face::Top, 0, (0, 0, 0))
        .is_some());
}

fn uv_lock_resolves_each_composite_child_transform_before_variant_rotation() {
    let mut bank = ModelBank::default();
    for name in ["assembly", "left", "right"] {
        bank.insert(&format!("assets/test/blockstates/{name}.json"),&serde_json::to_vec(&json!({"variants":{"":{"model":format!("test:block/{name}"),"x":90,"y":270,"uvlock":true}}})).unwrap()).unwrap();
    }
    bank.insert("assets/test/models/block/base.json",br##"{"elements":[{"from":[1,2,3],"to":[15,14,13],"faces":{"north":{"texture":"test:block/uv","uv":[2,3,11,14],"rotation":90}}}]}"##).unwrap();
    bank.insert("assets/test/models/block/assembly.json",br#"{"loader":"neoforge:composite","transform":{"translation":[0.25,0,0]},"children":{"left":{"parent":"test:block/base","transform":{"scale":[-1,1,1],"origin":"center"}},"right":{"parent":"test:block/base","transform":{"scale":[2,1,1],"origin":"center"}}}}"#).unwrap();
    bank.insert("assets/test/models/block/left.json",br#"{"parent":"test:block/base","transform":{"matrix":[[-1,0,0,1.25],[0,1,0,0],[0,0,1,0]]}}"#).unwrap();
    bank.insert("assets/test/models/block/right.json",br#"{"parent":"test:block/base","transform":{"matrix":[[2,0,0,-0.25],[0,1,0,0],[0,0,1,0]]}}"#).unwrap();
    let assembly = State {
        name: "test:assembly".into(),
        properties: vec![],
    };
    let parts = bank.parts_at(&assembly, (0, 0, 0)).unwrap();
    assert_eq!(parts.len(), 2);
    for (i, name) in ["left", "right"].into_iter().enumerate() {
        let standalone = State {
            name: format!("test:{name}"),
            properties: vec![],
        };
        let expected = bank
            .element_face_at(&standalone, Face::NegZ, 0, (0, 0, 0))
            .unwrap();
        assert_eq!(
            bank.element_face_at(&assembly, Face::NegZ, i, (0, 0, 0))
                .unwrap(),
            expected
        );
        assert_eq!(
            parts[i].face_vertices(Face::NegZ.corners()),
            bank.parts_at(&standalone, (0, 0, 0)).unwrap()[0].face_vertices(Face::NegZ.corners())
        );
    }
}

/// 三条绘制路径共用的实际资源包：十种根变换、全部状态旋转及非对称实体图片。
pub(crate) fn fixture() -> crate::textures::java_obj_tests::Fixture {
    let f = crate::textures::java_obj_tests::Fixture::new();
    let reference: Value =
        serde_json::from_str(include_str!("../tests/fixtures/uvlock-1.21.1.json")).unwrap();
    let pixels = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba([x as u8 * 3, y as u8 * 3, (x ^ y) as u8 * 3, 255])
    });
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, 64, 64, image::ExtendedColorType::Rgba8)
        .unwrap();
    f.put("assets/test/textures/block/uv.png", &png);
    for (i, root) in reference["roots"].as_array().unwrap().iter().enumerate() {
        let mut variants = serde_json::Map::new();
        for x in [0, 90, 180, 270] {
            for y in [0, 90, 180, 270] {
                variants.insert(
                    format!("turn={x}_{y}"),
                    json!({"model":format!("test:block/root_{i}"),"x":x,"y":y,"uvlock":true}),
                );
            }
        }
        f.put(
            &format!("assets/test/blockstates/root_{i}.json"),
            &serde_json::to_vec(&json!({"variants":variants})).unwrap(),
        );
        let rows = root
            .as_array()
            .unwrap()
            .chunks_exact(4)
            .map(|r| Value::Array(r.to_vec()))
            .collect::<Vec<_>>();
        let faces: FMap = FACES
            .iter()
            .enumerate()
            .map(|(i, (_, name))| {
                (
                    (*name).into(),
                    json!({"texture":"test:block/uv","uv":[2,3,11,14],"rotation":i%4*90}),
                )
            })
            .collect();
        f.put(&format!("assets/test/models/block/root_{i}.json"),&serde_json::to_vec(&json!({"transform":{"matrix":rows},"elements":[{"from":[1,2,3],"to":[15,14,13],"faces":faces}]})).unwrap());
    }
    f
}
type FMap = serde_json::Map<String, Value>;
pub(crate) fn states() -> Vec<State> {
    let mut states = Vec::new();
    for i in 0..10 {
        for x in [0, 90, 180, 270] {
            for y in [0, 90, 180, 270] {
                states.push(State {
                    name: format!("test:root_{i}"),
                    properties: vec![("turn".into(), format!("{x}_{y}"))],
                });
            }
        }
    }
    states
}

fn uv_lock_mesh_projection_and_source_picking_keep_the_original_state() {
    use crate::textures::java_obj_tests;
    use iced::{Point, Size};
    let f = fixture();
    let bank = f.open();
    assert!(bank.warnings.is_empty(), "{:?}", bank.warnings);
    for state in states() {
        let mut view = java_obj_tests::scene(Arc::clone(&bank));
        Arc::make_mut(&mut view.surface).voxels[0].state = state.clone().into();
        let mesh = crate::spectator::build_scene_cancellable(&view, &|| false).unwrap();
        let size = Size::new(32., 32.);
        let owner = (0..32)
            .flat_map(|y| (0..32).map(move |x| Point::new(x as f32 + 0.5, y as f32 + 0.5)))
            .find_map(|p| {
                mesh.pick(
                    view.camera,
                    p,
                    crate::render_viewport::RenderViewport::one_to_one(size),
                    view.quality,
                )
            })
            .unwrap()
            .owner
            .unwrap();
        assert_eq!(owner.position, [0; 3]);
        assert_eq!(*owner.state, state);
        let mut iso = java_obj_tests::isometric_scene(Arc::clone(&bank));
        Arc::make_mut(&mut iso.surface).voxels[0].state = state.into();
        let faces = iso.faces_cached(Size::new(128., 128.));
        assert!(!faces.is_empty());
        let center = Point::new(
            faces[0].corners.iter().map(|p| p.x).sum::<f32>() / 4.,
            faces[0].corners.iter().map(|p| p.y).sum::<f32>() / 4.,
        );
        assert_eq!(
            iso.pick_face(center, Size::new(128., 128.)).unwrap().0,
            (0, 0, 0)
        );
        assert!(!iso.gpu_vertices_cached(Size::new(128., 128.)).is_empty());
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Models {
        uv_lock_matches_actual_java_client_for_roots_crops_mirrors_and_all_variant_turns();
        variant_geometry_and_inverse_faces_match_actual_java_block_model_rotations();
        singular_root_is_rejected_for_locked_variants_and_preserved_when_unlocked();
        uv_lock_resolves_each_composite_child_transform_before_variant_rotation();
        uv_lock_mesh_projection_and_source_picking_keep_the_original_state();
    }
}
