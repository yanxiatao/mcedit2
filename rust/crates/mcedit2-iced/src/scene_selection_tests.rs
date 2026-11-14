//! 直接执行消息处理、源读取与生产拾取，验证视图切换不会改变已应用的窗口。
use super::*;
use std::sync::Arc;

fn app() -> App {
    let world = mcedit2_world::inspect_world(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test_files/AnvilWorld_1.8"),
    )
    .unwrap();
    App {
        worlds: vec![world],
            workspace: { let mut state = crate::workspace_tabs::State::default(); state.open(0); state },
        current: Some(0),
        surface_area: "-4,10,-4;3,17,3".into(),
        ..Default::default()
    }
}
fn publish(app: &mut App) -> Arc<SurfaceView> {
    let job = app.near_job.take().unwrap();
    let result = job.read();
    match &result {
        NearResult::Surface(Ok(_)) => {}
        other => panic!("实际源读取失败：{other:?}"),
    }
    app.near_loaded(job.nonce, job.key, result);
    Arc::clone(&app.surface.as_ref().unwrap().2)
}
fn message(app: &mut App, message: Message) {
    let _ = super::super::update_impl(app, message);
}

fn selected_volume_keeps_source_cut_and_selection_through_all_camera_modes() {
    let mut app = app();
    message(&mut app, Message::InspectVolume);
    publish(&mut app);
    message(&mut app, Message::Cut3DTop(-2));
    let source = publish(&mut app);
    app.selected_surface = Some((0, 12, 0));
    app.corner_a = Some((0, 12, 0));
    app.corner_b = Some((1, 12, 1));
    let selected = app.selected_window();
    let nonce = app.near_nonce;
    for step in [
        Message::ToggleTopDown,
        Message::ToggleTopDown,
        Message::ToggleSpectator,
        Message::ToggleTopDown,
        Message::ToggleTopDown,
        Message::ToggleSpectator,
    ] {
        message(&mut app, step);
        assert_eq!(app.selected_window(), selected);
        assert_eq!(app.surface_cut_top, Some(15));
        assert!(app.near_job.is_none());
        assert_eq!(app.near_nonce, nonce);
        assert!(Arc::ptr_eq(&source, &app.surface.as_ref().unwrap().2));
        assert_eq!(app.selected_surface, Some((0, 12, 0)));
        assert_eq!(app.corner_a, Some((0, 12, 0)));
        assert_eq!(app.corner_b, Some((1, 12, 1)));
    }
}

fn invalid_draft_cannot_cancel_retarget_or_remove_committed_volume() {
    let mut app = app();
    message(&mut app, Message::InspectVolume);
    publish(&mut app);
    message(&mut app, Message::Cut3DTop(-1));
    let selected = app.selected_window();
    let nonce = app.near_nonce;
    app.near_busy = true;
    let _ = super::super::update(
        &mut app,
        Message::SurfaceArea("not applied coordinates".into()),
    );
    assert_eq!(app.near_nonce, nonce);
    assert_eq!(app.near_job.as_ref().unwrap().nonce, nonce);
    assert_eq!(app.selected_window(), selected);
    assert_eq!(app.surface_cut_top, Some(16));
    message(&mut app, Message::ReloadScene);
    assert_eq!(
        app.near_job.as_ref().unwrap().bounds,
        selected.unwrap().bounds
    );
    assert_eq!(app.near_job.as_ref().unwrap().cut, Some(16));
    let nonce = app.near_nonce;
    message(&mut app, Message::InspectVolume);
    assert_eq!(app.selected_window(), selected);
    assert_eq!(app.near_nonce, nonce);
    assert_eq!(app.near_job.as_ref().unwrap().nonce, nonce);
    assert!(app.surface_report.contains("refused"));
    let source = publish(&mut app);
    assert_eq!(
        (source.x, source.z, source.width, source.depth),
        (-4, -4, 8, 8)
    );
    assert_eq!(app.surface_area, "not applied coordinates");
}

fn automatic_world_still_reads_full_height_when_leaving_free_flight() {
    let mut app = app();
    app.open_default_scene(85);
    assert_eq!(app.near_job.as_ref().unwrap().mode, NearMode::Volume);
    assert!(app.selected_window().is_none());
    let source = publish(&mut app);
    message(&mut app, Message::ToggleSpectator);
    assert!(!app.spectator);
    assert!(app.near_job.is_none(), "更换投影不能把完整体积替换成最高列");
    assert!(Arc::ptr_eq(&source, &app.surface.as_ref().unwrap().2));
    let _ = app.maintain_spectator_stream();
    assert!(
        app.spectator_stream.is_some(),
        "世界等轴必须继续完整高度区块读取"
    );
    message(&mut app, Message::ToggleTopDown);
    assert_eq!(app.near_job.as_ref().unwrap().mode, NearMode::Surface);
    message(&mut app, Message::ToggleSpectator);
    assert!(!app.show_top_down);
    assert!(app.spectator);
    assert_eq!(app.near_job.as_ref().unwrap().mode, NearMode::Volume);
    assert!(app.selected_window().is_none());
}

fn world_orthographic_reload_retains_volume_and_explicit_far_preview_releases_old_stream_frame() {
    let mut app = app();
    app.open_default_scene(85);
    publish(&mut app);
    message(&mut app, Message::ToggleSpectator);
    assert!(app.uses_world_mesh());
    app.reload_scene();
    assert_eq!(app.near_job.as_ref().unwrap().mode, NearMode::Volume);
    publish(&mut app);
    let _ = app.maintain_spectator_stream();
    assert!(app.spectator_stream.is_some());
    let _ = app.start_far_view(1, Some((0, 0)));
    assert!(!app.uses_world_mesh());
    assert!(!app.spectator);
    assert!(app.far.is_some());
    assert!(app.spectator_stream_snapshot.is_none());
    assert!(app.spectator_presented.lock().unwrap().is_none());
}

fn free_flight_cannot_expand_a_selected_window_or_start_world_streaming() {
    let mut app = app();
    app.spectator = true;
    message(&mut app, Message::InspectVolume);
    publish(&mut app);
    let bounds = app.selected_window().unwrap().bounds;
    let input = app.surface_area.clone();
    let position = app.spectator_camera.position;
    app.spectator_keys.insert('w');
    let start = std::time::Instant::now();
    app.spectator_last_frame = Some(start);
    for frame in 1..=80 {
        message(
            &mut app,
            Message::SpectatorTick(start + std::time::Duration::from_millis(frame * 16)),
        );
    }
    assert_ne!(app.spectator_camera.position, position);
    assert_eq!(app.surface_area, input);
    assert_eq!(app.selected_window().unwrap().bounds, bounds);
    assert!(app.near_job.is_none());
    let _ = app.maintain_spectator_stream();
    assert!(app.spectator_stream.is_none());
    message(&mut app, Message::FollowWorld);
    assert!(app.selected_window().is_none());
    assert!(app.surface_cut_top.is_none());
    let job = app.near_job.as_ref().unwrap();
    assert_eq!(job.mode, NearMode::Volume);
    assert_eq!(job.bounds.maximum[0] - job.bounds.minimum[0] + 1, 32);
    assert_eq!(job.bounds.maximum[1] - job.bounds.minimum[1] + 1, 48);
    publish(&mut app);
    let _ = app.maintain_spectator_stream();
    assert!(app.spectator_stream.is_some());
}

fn cut_shift_resize_and_center_update_the_selected_window_consistently() {
    let mut app = app();
    message(&mut app, Message::InspectVolume);
    message(&mut app, Message::Cut3DTop(-2));
    message(&mut app, Message::Shift3D(0, 16, 0));
    let selected = app.selected_window().unwrap();
    assert_eq!(
        (selected.bounds.minimum[1], selected.bounds.maximum[1]),
        (26, 33)
    );
    assert_eq!(selected.cut, Some(31));
    assert_eq!(app.near_job.as_ref().unwrap().cut, Some(31));
    message(&mut app, Message::ToggleTopDown);
    message(&mut app, Message::ToggleTopDown);
    assert_eq!(app.selected_window(), Some(selected));
    message(&mut app, Message::ScenePreset(16));
    let resized = app.selected_window().unwrap();
    assert_eq!(resized.bounds.maximum, [11, 57, 11]);
    assert!(resized.cut.is_none());
    app.query_coordinates = "3,30,2".into();
    message(&mut app, Message::Center3DOnPicked);
    let centered = app.selected_window().unwrap();
    assert_eq!(centered.bounds.minimum, [0, 22, 0]);
    assert_eq!(centered.bounds.maximum, [15, 37, 15]);
    assert!(centered.cut.is_none());
    assert!(centered.volume);
}

fn changing_world_or_dimension_revokes_the_selected_window() {
    let mut app = app();
    message(&mut app, Message::InspectVolume);
    message(&mut app, Message::Cut3DTop(-1));
    let mut other = app.worlds[0].clone();
    other.level_name = Some("另一测试世界".into());
    app.worlds.push(other);
    message(&mut app, Message::Select(1));
    assert!(app.selected_scene.is_none());
    assert!(app.selected_window().is_none());
    assert!(app.surface_cut_top.is_none());
    message(&mut app, Message::InspectVolume);
    let mut dim = app.worlds[1].dimensions[0].clone();
    dim.name = "test:another_dimension".into();
    dim.region_dir = dim.region_dir.join("missing-test-region");
    app.worlds[1].dimensions.push(dim);
    message(&mut app, Message::Dimension(1));
    assert!(app.selected_scene.is_none());
    assert_eq!(app.near_job.as_ref().unwrap().key, (1, 1));
    message(&mut app, Message::InspectVolume);
    message(&mut app, Message::WorkspaceIdle);
    assert!(app.selected_scene.is_none());
}

fn camera_flight_survives_projection_switches_and_reset_reframes_the_selected_source() {
    let mut app = app();
    app.spectator = true;
    message(&mut app, Message::InspectVolume);
    publish(&mut app);
    let initial = app.spectator_camera;
    app.spectator_camera.look(90., -30.);
    app.spectator_keys.insert('d');
    let start = std::time::Instant::now();
    app.spectator_last_frame = Some(start);
    for frame in 1..=30 {
        message(
            &mut app,
            Message::SpectatorTick(start + std::time::Duration::from_millis(frame * 16)),
        );
    }
    let flown = app.spectator_camera;
    assert_ne!(flown, initial);
    for step in [
        Message::ToggleSpectator,
        Message::ToggleTopDown,
        Message::ToggleTopDown,
        Message::ToggleSpectator,
    ] {
        message(&mut app, step);
    }
    assert_eq!(app.spectator_camera, flown);
    message(&mut app, Message::ResetSurfaceCamera);
    assert_eq!(app.spectator_camera, initial);
    assert!(app.spectator_keys.is_empty());
}

fn bounded_source_read_and_perspective_pick_keep_actual_original_block_identity() {
    let (mut app, scene, _, _) = crate::picking_integration_tests::fixture();
    let original = scene.surface.voxels[0].clone();
    let bounds = Bounds {
        minimum: [original.x, original.y, original.z],
        maximum: [original.x, original.y, original.z],
    };
    app.surface_area = coordinates(bounds);
    message(&mut app, Message::InspectVolume);
    let source = publish(&mut app);
    assert_eq!(source.voxels.len(), 1);
    assert_eq!(source.voxels[0].state, original.state);
    assert_eq!((source.y_min, source.y_max), (original.y, original.y));
    for step in [
        Message::ToggleSpectator,
        Message::ToggleTopDown,
        Message::ToggleTopDown,
        Message::ToggleSpectator,
    ] {
        message(&mut app, step);
        assert!(Arc::ptr_eq(&source, &app.surface.as_ref().unwrap().2));
    }
    let mut view = scene;
    view.surface = source;
    view.camera = app.spectator_camera;
    view.cache = Default::default();
    view.chunk_cache = Default::default();
    let mesh = spectator::build_scene_cancellable(&view, &|| false).unwrap();
    let owner = mesh
        .pick(
            view.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(200., 200.)),
            view.quality,
        )
        .unwrap()
        .owner
        .unwrap();
    assert_eq!(owner.position, [original.x, original.y, original.z]);
    assert_eq!(owner.state, original.state);
}

fn framed_window_keeps_source_corners_visible_at_world_border_and_narrow_aspect() {
    let mut app = app();
    for bounds in [
        Bounds {
            minimum: [29_999_930, 4000, -29_999_999],
            maximum: [29_999_993, 4047, -29_999_936],
        },
        Bounds {
            minimum: [-5, -64, 17],
            maximum: [-5, -64, 17],
        },
    ] {
        app.frame_selected_window(Window {
            key: (0, 0),
            bounds,
            volume: true,
            cut: None,
        });
        for size in [iced::Size::new(800., 600.), iced::Size::new(400., 800.)] {
            let uniform = app.spectator_camera.uniform_at_origin(bounds.minimum, size);
            for bits in 0..8 {
                let point: [f64; 3] = std::array::from_fn(|i| {
                    if bits & (1 << i) == 0 {
                        f64::from(bounds.minimum[i]) - 0.5
                    } else {
                        f64::from(bounds.maximum[i]) + 0.5
                    }
                });
                let delta: [f64; 3] =
                    std::array::from_fn(|i| point[i] - app.spectator_camera.position[i]);
                let dot =
                    |axis: [f32; 4]| (0..3).map(|i| delta[i] * f64::from(axis[i])).sum::<f64>();
                let depth = dot(uniform.forward);
                assert!(depth > f64::from(uniform.projection[2]));
                assert!((dot(uniform.right) * f64::from(uniform.projection[0]) / depth).abs() < 1.);
                assert!((dot(uniform.up) * f64::from(uniform.projection[1]) / depth).abs() < 1.);
            }
        }
    }
}

fn selected_surface_reads_full_height_frames_real_terrain_once_and_resumes_world_from_2d() {
    let (mut app, scene, _, _) = crate::picking_integration_tests::fixture();
    let voxel = &scene.surface.voxels[0];
    let bounds = Bounds {
        minimum: [voxel.x, 100, voxel.z],
        maximum: [voxel.x, 101, voxel.z],
    };
    app.surface_area = coordinates(bounds);
    message(&mut app, Message::InspectSurface);
    assert!(!app.selected_window().unwrap().volume);
    let source = publish(&mut app);
    assert_eq!((source.y_min, source.y_max), (-2048, 2047));
    assert_eq!(source.voxels.len(), 1);
    assert!(source.voxels[0].y >= voxel.y);
    assert!(app.window_camera.is_some());
    app.spectator_camera.look(60., 20.);
    let moved = app.spectator_camera;
    message(&mut app, Message::ReloadScene);
    publish(&mut app);
    assert_eq!(app.spectator_camera, moved);
    let current = Arc::clone(&app.surface.as_ref().unwrap().2);
    message(&mut app, Message::ToggleTopDown);
    assert!(Arc::ptr_eq(&current, &app.surface.as_ref().unwrap().2));
    message(&mut app, Message::FollowWorld);
    assert!(app.selected_window().is_none());
    let job = app.near_job.as_ref().unwrap();
    assert_eq!(job.mode, NearMode::Surface);
    assert_eq!(job.bounds.minimum[0], voxel.x - 16);
    assert_eq!(job.bounds.minimum[2], voxel.z - 16);
    let nonce = app.near_nonce;
    message(&mut app, Message::FollowWorld);
    assert_eq!(app.near_nonce, nonce);
}

fn projected_selection_updates_the_snapshot_details_and_revokes_an_old_face() {
    let (mut app, scene, _, _) = crate::picking_integration_tests::fixture();
    let first = scene.surface.voxels[0].clone();
    let mut next = first.clone();
    next.x += 1;
    let current = app
        .current_session((0, 0))
        .unwrap()
        .visible_block((next.x, next.y, next.z))
        .unwrap();
    let mcedit2_world::block_lookup::Lookup::Present(block) = current else {
        panic!("实际样本中的相邻固体方块必须存在")
    };
    next.state = block.state.into();
    let mut source = (*scene.surface).clone();
    source.voxels = vec![first.clone(), next.clone()];
    app.surface = Some((0, 0, Arc::new(source)));
    message(
        &mut app,
        Message::PickSurfaceFace(first.x, first.y, first.z, Face::PosX),
    );
    let old = app.block_result.clone();
    assert_eq!(app.selected_surface_face, Some(Face::PosX));
    assert!(app.pick_surface(next.x, next.y, next.z));
    assert_eq!(app.selected_surface, Some((next.x, next.y, next.z)));
    assert!(app.selected_surface_face.is_none());
    assert_ne!(app.block_result, old);
    assert!(app.block_result.contains(&next.state.name));
    assert!(app
        .block_result
        .contains(&format!("({},{},{})", next.x, next.y, next.z)));
    message(
        &mut app,
        Message::PickSurfaceFace(next.x, next.y, next.z, Face::Top),
    );
    assert_eq!(app.selected_surface_face, Some(Face::Top));
    Arc::make_mut(&mut app.surface.as_mut().unwrap().2).voxels[1].span = 2;
    message(
        &mut app,
        Message::PickSurfaceFace(next.x, next.y, next.z, Face::Top),
    );
    assert!(app.selected_surface.is_none());
    assert!(app.selected_surface_face.is_none());
    assert!(app.block_result.is_empty());
    assert!(app.surface_report.contains("changed"));
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Navigation {
        selected_volume_keeps_source_cut_and_selection_through_all_camera_modes();
        invalid_draft_cannot_cancel_retarget_or_remove_committed_volume();
        automatic_world_still_reads_full_height_when_leaving_free_flight();
        world_orthographic_reload_retains_volume_and_explicit_far_preview_releases_old_stream_frame(
        );
        free_flight_cannot_expand_a_selected_window_or_start_world_streaming();
        cut_shift_resize_and_center_update_the_selected_window_consistently();
        changing_world_or_dimension_revokes_the_selected_window();
        camera_flight_survives_projection_switches_and_reset_reframes_the_selected_source();
        bounded_source_read_and_perspective_pick_keep_actual_original_block_identity();
        framed_window_keeps_source_corners_visible_at_world_border_and_narrow_aspect();
        selected_surface_reads_full_height_frames_real_terrain_once_and_resumes_world_from_2d();
        projected_selection_updates_the_snapshot_details_and_revokes_an_old_face();
    }
}
