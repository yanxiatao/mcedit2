//! 直接比较生产投影、原生世界网格和源拾取，不验证源码文本。
use super::*;
use crate::orthographic_framing::Bounds as ModelBounds;

fn cube_scene() -> IsometricSurface {
    let mut view = super::tests::example();
    view.zoom = 1.;
    view.pan_x = 0.;
    view.pan_y = 0.;
    let source = Arc::make_mut(&mut view.surface);
    source.voxels[0].state = State {
        name: "minecraft:stone".into(),
        properties: vec![],
    }
    .into();
    source.voxels[0].kind = VoxelMaterial::Solid;
    view
}
use mcedit2_world::sections::State;

fn inside(point: Point, size: Size) {
    assert!(
        point.x >= -0.001
            && point.x <= size.width + 0.001
            && point.y >= -0.001
            && point.y <= size.height + 0.001,
        "构图裁掉真实几何：{point:?} / {size:?}"
    );
}

fn full_height_near_scene_fits_all_aspects_without_a_minimum_pixel_scale() {
    let mut view = cube_scene();
    let source = Arc::make_mut(&mut view.surface);
    source.x = 0;
    source.z = 0;
    source.width = 32;
    source.depth = 32;
    source.y_min = -64;
    source.y_max = 319;
    source.voxels[0].x = 0;
    source.voxels[0].y = -64;
    source.voxels[0].z = 0;
    let mut upper = source.voxels[0].clone();
    upper.x = 31;
    upper.y = 319;
    upper.z = 31;
    source.voxels.push(upper);
    for yaw in [0, 15, 45, 90, 225] {
        for pitch in [15, 35, 80] {
            view.yaw_degrees = yaw;
            view.pitch_degrees = pitch;
            for size in [
                Size::new(1000., 650.),
                Size::new(180., 900.),
                Size::new(1200., 180.),
                Size::new(1., 1.),
            ] {
                for voxel in &view.surface.voxels {
                    let xyz = [voxel.x, voxel.y, voxel.z];
                    for corner in IsometricSurface::bounds_corners(xyz, xyz) {
                        inside(view.project(corner, size).0, size);
                    }
                }
            }
        }
    }
}

fn unused_source_sections_do_not_shift_or_shrink_a_single_actual_voxel() {
    let mut view = cube_scene();
    let source = Arc::make_mut(&mut view.surface);
    source.x = 0;
    source.z = 0;
    source.width = 64;
    source.depth = 64;
    source.y_min = -64;
    source.y_max = 319;
    source.voxels[0].x = 12;
    source.voxels[0].y = 100;
    source.voxels[0].z = 7;
    let size = Size::new(800., 600.);
    let point = view.project([12., 100., 7.], size).0;
    assert!((point.x - 400.).abs() < 0.001 && (point.y - 300.).abs() < 0.001);
    assert!(view.scale(size) > 100., "单格物体不应被固定 24px 上限压小");
}

fn translated_json_model_remains_visible_when_its_original_voxel_is_offscreen() {
    let fixture = crate::canvas_raster::tests::fixture(1, 1, &[255; 4]);
    fixture.put("assets/test/models/block/tile.json",br##"{"transform":{"translation":[32,0,0]},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"test:block/tile"},"down":{"texture":"test:block/tile"},"north":{"texture":"test:block/tile"},"south":{"texture":"test:block/tile"},"east":{"texture":"test:block/tile"},"west":{"texture":"test:block/tile"}}}]}"##);
    let bank = fixture.open();
    let state = State {
        name: "test:tile".into(),
        properties: vec![],
    };
    let part = bank.geometry_at(&state, (0, 0, 0)).unwrap().remove(0);
    let corners: Vec<_> = [Face::Top, Face::Bottom]
        .into_iter()
        .flat_map(|face| part.face_vertices(face.corners()))
        .map(|p| p.map(f64::from))
        .collect();
    let center = std::array::from_fn(|axis| corners.iter().map(|p| p[axis]).sum::<f64>() / 8.);
    let mut view = crate::textures::java_obj_tests::isometric_scene(bank);
    view.yaw_degrees = 0;
    view.zoom = 1.;
    Arc::make_mut(&mut view.surface).voxels[0].state = state.into();
    let size = Size::new(900., 600.);
    for &point in &corners {
        inside(view.project(point, size).0, size);
    }
    view.zoom = 4.;
    let projected = view.project(center, size).0;
    view.pan_x = size.width * 0.5 - projected.x;
    view.pan_y = size.height * 0.5 - projected.y;
    assert!(view.project([0.; 3], size).0.x < -size.width);
    let faces = view.faces_cached(size);
    assert!(
        !faces.is_empty(),
        "不能用原始方块周围的固定半径裁掉移位模型"
    );
    assert_eq!(
        view.pick_face(Point::new(450., 300.), size).unwrap().0,
        (0, 0, 0)
    );
}

fn json_and_obj_composite_vertices_from_the_world_mesh_fit_the_same_projection() {
    let fixture = crate::textures::java_obj_tests::fixture();
    let bank = fixture.open();
    let scene = crate::textures::java_obj_tests::scene(Arc::clone(&bank));
    let mesh = crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap();
    let mut view = crate::textures::java_obj_tests::isometric_scene(bank);
    view.zoom = 1.;
    for yaw in [0, 45, 135] {
        for pitch in [15, 60, 80] {
            view.yaw_degrees = yaw;
            view.pitch_degrees = pitch;
            for size in [
                Size::new(900., 600.),
                Size::new(150., 900.),
                Size::new(900., 150.),
            ] {
                for chunk in &mesh.chunks {
                    for quad in chunk.quads().iter() {
                        for point in quad.xyz {
                            let world =
                                std::array::from_fn(|i| chunk.origin[i] as f64 + point[i] as f64);
                            inside(view.project(world, size).0, size);
                        }
                    }
                }
            }
        }
    }
}

fn bounds_cache_reuses_camera_independent_data_and_never_publishes_canceled_work() {
    let mut view = cube_scene();
    let size = Size::new(800., 600.);
    assert!(view.cached_projection(size).is_none());
    assert!(view.prepare_projection(size, &|| true).is_none());
    assert!(view.cached_bounds().is_none());
    let checks = std::cell::Cell::new(0);
    assert!(view
        .prepare_projection(size, &|| {
            checks.set(checks.get() + 1);
            checks.get() > 2
        })
        .is_none());
    assert!(checks.get() > 2, "必须在实际遍历模型期间取消");
    assert!(view.cached_bounds().is_none());
    view.prepare_projection(size, &|| false).unwrap().unwrap();
    let before = view.cached_bounds().unwrap().unwrap();
    view.yaw_degrees = 75;
    view.zoom = 2.;
    view.pan_x = 44.;
    view.prepare_projection(Size::new(160., 900.), &|| false)
        .unwrap()
        .unwrap();
    assert!(Arc::ptr_eq(
        &before,
        &view.cached_bounds().unwrap().unwrap()
    ));
    Arc::make_mut(&mut view.surface).voxels[0].y += 128;
    assert!(view.cached_bounds().is_none());
    assert!(view.prepare_projection(size, &|| true).is_none());
    view.prepare_projection(size, &|| false).unwrap().unwrap();
    let after = view.cached_bounds().unwrap().unwrap();
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(after.minimum[1], before.minimum[1] + 128.);
}

fn empty_source_uses_its_real_request_box_and_invalid_projection_does_not_create_faces() {
    let mut view = cube_scene();
    Arc::make_mut(&mut view.surface).voxels.clear();
    let size = Size::new(180., 900.);
    for point in view.source_box() {
        inside(view.project(point, size).0, size);
    }
    assert!(view.faces_cached(size).is_empty());
    assert!(view
        .prepare_projection(Size::new(f32::NAN, 600.), &|| false)
        .unwrap()
        .is_err());
    let mut bounds = ModelBounds::UNIT;
    assert!(bounds.include([f64::INFINITY, 0., 0.]).is_err());
    assert_eq!(bounds, ModelBounds::UNIT);
}

fn pointer_hit_testing_does_not_prepare_missing_projection_or_geometry_on_the_ui_thread() {
    let view = cube_scene();
    assert!(view
        .pick_face(Point::new(400., 300.), Size::new(800., 600.))
        .is_none());
    assert!(view.cached_bounds().is_none());
    assert!(view.frame_cache.lock().unwrap().is_none());
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Rendering {
        full_height_near_scene_fits_all_aspects_without_a_minimum_pixel_scale();
        unused_source_sections_do_not_shift_or_shrink_a_single_actual_voxel();
        translated_json_model_remains_visible_when_its_original_voxel_is_offscreen();
        json_and_obj_composite_vertices_from_the_world_mesh_fit_the_same_projection();
        bounds_cache_reuses_camera_independent_data_and_never_publishes_canceled_work();
        empty_source_uses_its_real_request_box_and_invalid_projection_does_not_create_faces();
        pointer_hit_testing_does_not_prepare_missing_projection_or_geometry_on_the_ui_thread();
    }
}
