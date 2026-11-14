//! 用客户端 Cuboid 顶点、UV 和 Direction 四元数的独立参照核对静态潜影盒。
use super::*;
use crate::{isometric::Face, textures::java_obj_tests::Fixture};
use image::ImageEncoder;
use mcedit2_world::editing::parse_state;

pub(crate) const COLORS: [&str; 17] = [
    "",
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];
pub(crate) fn names(color: &str) -> (String, String) {
    if color.is_empty() {
        ("shulker_box".into(), "shulker".into())
    } else {
        (format!("{color}_shulker_box"), format!("shulker_{color}"))
    }
}
pub(crate) fn fixture() -> Fixture {
    let f = Fixture::new();
    for (i, color) in COLORS.iter().enumerate() {
        let (block, sheet) = names(color);
        for (path, doc) in [
            (
                format!("assets/minecraft/blockstates/{block}.json"),
                serde_json::json!({"variants":{"":{"model":format!("minecraft:block/{block}")}}}),
            ),
            (
                format!("assets/minecraft/models/block/{block}.json"),
                serde_json::json!({"textures":{"particle":format!("minecraft:block/{block}")}}),
            ),
        ] {
            f.put(&path, &serde_json::to_vec(&doc).unwrap());
        }
        let pixels = image::RgbaImage::from_fn(64, 64, |x, y| {
            image::Rgba([x as u8 * 3, y as u8 * 3, i as u8 * 13, 255])
        });
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, 64, 64, image::ExtendedColorType::Rgba8)
            .unwrap();
        f.put(
            &format!("assets/minecraft/textures/entity/shulker/{sheet}.png"),
            &png,
        );
    }
    f
}
fn near(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.000_01)
}
/// Direction.getRotation：客户端 rotationXYZ 使用先 Z 后 X 的矩阵作用顺序。
fn orient([x, y, z]: [f32; 3], facing: &str) -> [f32; 3] {
    match facing {
        "up" => [x, y, z],
        "down" => [x, -y, -z],
        "north" => [-x, -z, -y],
        "south" => [x, -z, y],
        "west" => [-y, -z, x],
        "east" => [y, -z, -x],
        _ => panic!("未知测试朝向"),
    }
}

fn shulker_all_colors_directions_match_client_vertices_and_uvs() {
    let f = fixture();
    let bank = f.open();
    assert!(bank.warnings.is_empty(), "{:?}", bank.warnings);
    assert_eq!(bank.statistics.requested_png, 17);
    assert_eq!(bank.statistics.atlas_png, 17);
    for color in COLORS {
        let (block, sheet) = names(color);
        for facing in ["up", "down", "north", "south", "west", "east"] {
            let state = parse_state(&format!("minecraft:{block}[facing={facing}]")).unwrap();
            let parts = bank.geometry_at(&state, (3, 41, -8)).unwrap();
            assert_eq!(parts.len(), 2, "{block} {facing} 应有盒身和盒盖");
            let texture = format!("minecraft:entity/shulker/{sheet}");
            let slot = &bank.atlas().unwrap().entries[&texture];
            for (index, part) in parts.iter().enumerate() {
                // 客户端 base 是 8px 高，闭合 lid 遮住其中 4px；移除覆盖区域，
                // 参照 UV 仍从完整 Cuboid 裁切，不能将整条 8px 纹理缩到 4px。
                let (min_y, max_y, side_v, end_v, cap) = if index == 0 {
                    (-4., 0., 48., 52., [32., 44., 48., 28.])
                } else {
                    (-16., -4., 16., 28., [16., 0., 32., 16.])
                };
                let vertices = [
                    [-8., min_y, -8.],
                    [8., min_y, -8.],
                    [8., max_y, -8.],
                    [-8., max_y, -8.],
                    [-8., min_y, 8.],
                    [8., min_y, 8.],
                    [8., max_y, 8.],
                    [-8., max_y, 8.],
                ];
                // ModelPart.Cube 原始六面顶点序和 Polygon 的 UV 赋值。
                let mut polygons = vec![
                    ([0, 4, 7, 3], [0., side_v, 16., end_v]),
                    ([1, 0, 3, 2], [16., side_v, 32., end_v]),
                    ([5, 1, 2, 6], [32., side_v, 48., end_v]),
                    ([4, 5, 6, 7], [48., side_v, 64., end_v]),
                ];
                polygons.push((
                    if index == 0 {
                        [2, 3, 7, 6]
                    } else {
                        [5, 4, 0, 1]
                    },
                    cap,
                ));
                let mut matched = 0;
                for face in [
                    Face::Top,
                    Face::Bottom,
                    Face::PosX,
                    Face::NegX,
                    Face::PosZ,
                    Face::NegZ,
                ] {
                    let Some((actual_uv, _, page)) =
                        bank.atlas_face_style_at(&state, face, index, (3, 41, -8))
                    else {
                        continue;
                    };
                    assert_eq!(page, slot.page);
                    let spec = bank
                        .models
                        .element_face_at(&state, face, index, (3, 41, -8))
                        .unwrap();
                    assert_eq!(spec.texture, texture);
                    let actual = part.face_vertices(face.corners());
                    let (ids, uv) = polygons
                        .iter()
                        .find(|(ids, _)| {
                            ids.iter().all(|&id| {
                                let [x, y, z] = vertices[id];
                                let p = orient([x / 16., 1. - (y + 24.) / 16., -z / 16.], facing)
                                    .map(|v| v * 0.9995);
                                actual.iter().any(|v| near(v, &p))
                            })
                        })
                        .unwrap_or_else(|| {
                            panic!("{block} {facing} {index} {face:?} 顶点不符合客户端")
                        });
                    let expected_uv = [
                        [uv[2], uv[1]],
                        [uv[0], uv[1]],
                        [uv[0], uv[3]],
                        [uv[2], uv[3]],
                    ];
                    for (id, uv) in ids.iter().zip(expected_uv) {
                        let [x, y, z] = vertices[*id];
                        let p = orient([x / 16., 1. - (y + 24.) / 16., -z / 16.], facing)
                            .map(|v| v * 0.9995);
                        let k = actual.iter().position(|v| near(v, &p)).unwrap();
                        assert!(
                            near(&actual_uv[k], &slot.sample(uv[0] / 64., uv[1] / 64.)),
                            "{block} {facing} {index} {face:?} UV 错误"
                        );
                    }
                    matched += 1;
                }
                assert_eq!(matched, 5, "闭合部分不得生成重合或内部面");
            }
        }
    }
}

fn shulker_closed_shell_keeps_source_picking_in_all_projections() {
    use iced::{Point, Size};
    let f = fixture();
    let bank = f.open();
    for color in COLORS {
        let (block, _) = names(color);
        for facing in ["up", "down", "north", "south", "west", "east"] {
            let state = parse_state(&format!("minecraft:{block}[facing={facing}]")).unwrap();
            let mut perspective = java_obj_tests::scene(Arc::clone(&bank));
            Arc::make_mut(&mut perspective.surface).voxels[0].state = state.clone().into();
            let mesh = crate::spectator::build_scene_cancellable(&perspective, &|| false).unwrap();
            assert_eq!(
                mesh.chunks.iter().map(|c| c.quads().len()).sum::<usize>(),
                10
            );
            let hit = mesh
                .pick(
                    perspective.camera,
                    Point::new(64., 64.),
                    crate::render_viewport::RenderViewport::one_to_one(Size::new(128., 128.)),
                    perspective.quality,
                )
                .unwrap();
            let owner = hit.owner.unwrap();
            assert_eq!(owner.position, [0, 0, 0]);
            assert_eq!(*owner.state, state);
            perspective.show_textures = false;
            assert_eq!(
                crate::spectator::build_scene_cancellable(&perspective, &|| false)
                    .unwrap()
                    .chunks[0]
                    .quads()
                    .len(),
                10
            );
            let mut iso = java_obj_tests::isometric_scene(Arc::clone(&bank));
            Arc::make_mut(&mut iso.surface).voxels[0].state = state.into();
            let size = Size::new(128., 128.);
            let faces = iso.faces_cached(size);
            assert!(!faces.is_empty());
            assert!(faces
                .iter()
                .all(|f| f.element_index < 2 && f.static_triangle.is_none()));
            let point = Point::new(
                faces[0].corners.iter().map(|p| p.x).sum::<f32>() / 4.,
                faces[0].corners.iter().map(|p| p.y).sum::<f32>() / 4.,
            );
            assert_eq!(iso.pick_face(point, size).unwrap().0, (0, 0, 0));
            assert_eq!(iso.gpu_vertices_cached(size).len(), faces.len() * 6);
        }
    }
}

fn shulker_explicit_empty_custom_and_foreign_models_are_preserved() {
    let f = fixture();
    f.put(
        "assets/minecraft/models/block/white_shulker_box.json",
        br#"{"elements":[]}"#,
    );
    f.put(
        "assets/minecraft/blockstates/red_shulker_box.json",
        br#"{"variants":{"":{"model":"test:block/custom"}}}"#,
    );
    f.put("assets/test/models/block/custom.json",br##"{"textures":{"all":"minecraft:entity/shulker/shulker_red"},"elements":[{"from":[2,4,6],"to":[8,10,12],"faces":{"up":{"texture":"#all"}}}]}"##);
    f.put(
        "assets/test/blockstates/shulker_box.json",
        br#"{"variants":{"":{"model":"minecraft:block/white_shulker_box"}}}"#,
    );
    let bank = f.open();
    assert!(bank
        .geometry_at(
            &parse_state("minecraft:white_shulker_box[facing=up]").unwrap(),
            (0, 0, 0)
        )
        .unwrap()
        .is_empty());
    assert!(bank
        .geometry_at(
            &parse_state("test:shulker_box[facing=up]").unwrap(),
            (0, 0, 0)
        )
        .unwrap()
        .is_empty());
    let custom = bank
        .geometry_at(
            &parse_state("minecraft:red_shulker_box[facing=down]").unwrap(),
            (0, 0, 0),
        )
        .unwrap();
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].bounds.min, [-0.375, -0.25, -0.125]);
    let normal = bank
        .geometry_at(
            &parse_state("minecraft:blue_shulker_box[facing=east]").unwrap(),
            (0, 0, 0),
        )
        .unwrap();
    assert_eq!(normal.len(), 2);
}

fn shulker_entity_overlay_is_used_instead_of_particle_and_missing_sheet_is_diagnosed() {
    let f = fixture();
    let overlay = Fixture::new();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[61, 137, 23, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    overlay.put(
        "assets/minecraft/textures/entity/shulker/shulker_red.png",
        &png,
    );
    fs::remove_file(
        f.path
            .join("assets/minecraft/textures/entity/shulker/shulker_blue.png"),
    )
    .unwrap();
    let bank = TextureBank::open_stack_java_controlled(
        &[f.path.clone(), overlay.path.clone()],
        &LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(bank.statistics.decoded_png, 16);
    assert!(bank
        .statistics
        .missing_references
        .contains(&"minecraft:entity/shulker/shulker_blue".into()));
    assert!(!bank.warnings.is_empty());
    for facing in ["up", "down", "north", "south", "west", "east"] {
        let state = parse_state(&format!("minecraft:red_shulker_box[facing={facing}]")).unwrap();
        for (index, face) in [
            (0, Face::Bottom),
            (1, Face::Top),
            (0, Face::PosX),
            (1, Face::NegZ),
        ] {
            let (uv, _, page) = bank
                .atlas_face_style_at(&state, face, index, (0, 0, 0))
                .unwrap();
            let middle = std::array::from_fn(|i| uv.iter().map(|p| p[i]).sum::<f32>() / 4.);
            assert_eq!(bank.atlas_pixel(middle, page), Some([61, 137, 23, 255]));
        }
    }
}

fn shulker_reserved_model_collision_is_an_explicit_load_error() {
    let f = fixture();
    f.put(
        "assets/mcedit2/models/builtin/shulker/shulker_box.json",
        br#"{"elements":[]}"#,
    );
    let error = TextureBank::open_stack_java_controlled(
        std::slice::from_ref(&f.path),
        &LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        error.contains("mcedit2:builtin/shulker/shulker_box"),
        "{error}"
    );
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::SpecialModels {
        shulker_all_colors_directions_match_client_vertices_and_uvs();
        shulker_closed_shell_keeps_source_picking_in_all_projections();
        shulker_explicit_empty_custom_and_foreign_models_are_preserved();
        shulker_entity_overlay_is_used_instead_of_particle_and_missing_sheet_is_diagnosed();
        shulker_reserved_model_collision_is_an_explicit_load_error();
    }
}
