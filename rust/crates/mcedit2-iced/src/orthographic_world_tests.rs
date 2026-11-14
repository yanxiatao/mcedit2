use super::*;
use iced::widget::canvas::Program;

fn view() -> View {
    View {
        angles: (0_f32.to_radians(), 35_f32.to_radians()),
        navigation: Viewport::default(),
        framing: Default::default(),
    }
}

fn source_mesh_bounds_projection_and_parallel_ray_picking_agree_at_all_aspects() {
    let (mut app, scene, mesh, request) = crate::picking_integration_tests::fixture();
    let original = request.hit.owner.unwrap();
    let source = Arc::clone(&mesh.chunks[0].quads());
    for size in [
        Size::new(700., 440.),
        Size::new(90., 900.),
        Size::new(900., 90.),
    ] {
        for yaw in [0_f32, 45., 135., 225.] {
            let view = View {
                angles: (yaw.to_radians(), 35_f32.to_radians()),
                ..view()
            };
            let projection = view.projection(&mesh, size).unwrap().unwrap();
            let mut hits = 0;
            for chunk in &mesh.chunks {
                for quad in chunk.quads().iter() {
                    for vertex in quad.xyz {
                        let p = projection
                            .project(std::array::from_fn(|i| {
                                chunk.origin[i] as f64 + vertex[i] as f64
                            }))
                            .0;
                        assert!(
                            p.x >= -0.01
                                && p.x <= size.width + 0.01
                                && p.y >= -0.01
                                && p.y <= size.height + 0.01
                        );
                    }
                    let point = projection
                        .project(std::array::from_fn(|i| {
                            chunk.origin[i] as f64 + quad.center()[i] as f64
                        }))
                        .0;
                    if let Some(hit) = mesh.pick(
                        projection,
                        point,
                        crate::render_viewport::RenderViewport::one_to_one(size),
                        scene.quality,
                    ) {
                        assert_eq!(hit.owner.as_ref(), Some(&original));
                        hits += 1;
                    }
                }
            }
            assert!(hits > 0);
        }
    }
    assert!(Arc::ptr_eq(&source, &mesh.chunks[0].quads()));
    // 同一类别内继续执行分批提交：增加远处和高处几何不能重新缩放已有方块。
    let view = view();
    let size = Size::new(700., 440.);
    let initial = view.projection(&mesh, size).unwrap().unwrap();
    let mut later_scene = scene.clone();
    let mut later_source = (*scene.surface).clone();
    let mut extra = later_source.voxels[0].clone();
    extra.x += 2048;
    extra.y += 400;
    extra.z += 1024;
    later_source.voxels.push(extra);
    later_scene.surface = Arc::new(later_source);
    later_scene.cache = Default::default();
    later_scene.chunk_cache = Default::default();
    let later_mesh = crate::spectator::build_scene_cancellable(&later_scene, &|| false).unwrap();
    let later = view.projection(&later_mesh, size).unwrap().unwrap();
    let point = original.position.map(f64::from);
    assert_eq!(initial.scale, later.scale);
    assert_eq!(initial.project(point), later.project(point));
    let uniform = later.uniform_at_origin(later_mesh.origin);
    for chunk in &later_mesh.chunks {
        for quad in chunk.quads().iter() {
            for vertex in quad.xyz {
                let relative: [f64; 3] = std::array::from_fn(|i| {
                    chunk.origin[i] as f64 + vertex[i] as f64
                        - later_mesh.origin[i] as f64
                        - uniform.position[i] as f64
                });
                let depth = (0..3)
                    .map(|i| relative[i] * uniform.forward[i] as f64)
                    .sum::<f64>();
                assert!(depth >= uniform.projection[2] as f64);
                assert!(depth <= uniform.projection[3] as f64);
            }
        }
    }
    // 首帧参照不会跟着不可见区块改变；当前画布大小仍能响应布局变化。
    let narrow = Size::new(90., 900.);
    let resized = view.projection(&later_mesh, narrow).unwrap().unwrap();
    let expected = View {
        framing: Default::default(),
        ..view.clone()
    }
    .projection(&mesh, narrow)
    .unwrap()
    .unwrap();
    assert_eq!(resized.project(point), expected.project(point));
    app.spectator = false;
    assert_eq!(app.current_session((0, 0)).unwrap().pending_cells(), 0);
}

fn orthographic_uniform_preserves_f64_world_origin_pan_zoom_and_depth() {
    let bounds = Bounds {
        minimum: [29_999_980.5, -64.5, -29_999_980.5],
        maximum: [29_999_985.5, 319.5, -29_999_974.5],
    };
    let origin = [29_999_980, -64, -29_999_980];
    let size = Size::new(220., 900.);
    let projection = Projection::fit(bounds, (0.7, 0.6), 1.5, [32., -17.], size).unwrap();
    let uniform =
        crate::world_camera::WorldCamera::from(projection).uniform_at_origin(origin, size);
    assert_eq!(uniform.position[3], 1.);
    for world in [
        bounds.minimum,
        bounds.maximum,
        [29_999_982., 128., -29_999_977.],
    ] {
        let relative: [f32; 3] =
            std::array::from_fn(|i| (world[i] - origin[i] as f64) as f32 - uniform.position[i]);
        let dot = |axis: [f32; 4]| (0..3).map(|i| relative[i] * axis[i]).sum::<f32>();
        let clip = [
            dot(uniform.right) * uniform.projection[0] + uniform.right[3],
            dot(uniform.up) * uniform.projection[1] + uniform.up[3],
        ];
        let p = projection.project(world).0;
        assert!((p.x - (clip[0] + 1.) * size.width * 0.5).abs() < 0.005);
        assert!((p.y - (1. - clip[1]) * size.height * 0.5).abs() < 0.005);
        let depth = (dot(uniform.forward) - uniform.projection[2])
            / (uniform.projection[3] - uniform.projection[2]);
        assert!((0. ..=1.).contains(&depth));
        let (start, direction, _) = projection.ray(origin, p, size).unwrap();
        let distance = (0..3)
            .map(|i| (world[i] - origin[i] as f64 - start[i]) * direction[i])
            .sum::<f64>();
        for i in 0..3 {
            assert!(
                (start[i] + direction[i] * distance - (world[i] - origin[i] as f64)).abs() < 0.001
            );
        }
    }
}

fn scrolled_world_orthographic_controls_pick_original_source_and_rotate_by_raw_delta() {
    let (mut app, scene, mesh, request) = crate::picking_integration_tests::fixture();
    *app.spectator_presented_view.lock().unwrap() = Some(crate::spectator::PresentedView {
        camera: view()
            .projection(&mesh, Size::new(200., 200.))
            .unwrap()
            .unwrap()
            .into(),
        viewport: crate::render_viewport::RenderViewport::one_to_one(Size::new(200., 200.)),
        quality: scene.quality,
        atlas: mesh.atlas.clone(),
        mesh: Arc::clone(&mesh),
    });
    let controls = Controls {
        selection_handle: None,
        presented: Arc::clone(&app.spectator_presented),
        presented_view: Arc::clone(&app.spectator_presented_view),
        key: Some((0, 0)),
        selected: None,
        selection: None,
        brush: None,
        show_frame: false,
        show_axes: false,
        show_chunk_grid: false,
        show_section_guides: false,
    };
    let bounds = Rectangle {
        x: 20.,
        y: 500.,
        width: 200.,
        height: 200.,
    };
    let cursor = mouse::Cursor::Available(Point::new(120., 600.));
    let mut state = State::default();
    controls.update(
        &mut state,
        &canvas::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(100., 100.),
        }),
        bounds,
        cursor,
    );
    controls
        .update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            cursor,
        )
        .unwrap();
    let message = controls
        .update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            bounds,
            cursor,
        )
        .unwrap()
        .into_inner()
        .0
        .unwrap();
    let Message::SpectatorPick(pick) = message else {
        panic!("实际源点选应提交生产拾取请求");
    };
    assert_eq!(pick.hit.owner, request.hit.owner);
    assert!(Arc::ptr_eq(&pick.mesh, &mesh));
    app.pick_spectator(pick);
    assert_eq!(
        app.selected_surface,
        request.hit.owner.as_ref().map(|o| {
            let [x, y, z] = o.position;
            (x, y, z)
        })
    );
    controls
        .update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
            bounds,
            cursor,
        )
        .unwrap();
    let action = controls
        .update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(116., 108.),
            }),
            bounds,
            cursor,
        )
        .unwrap();
    assert!(matches!(
        action.into_inner().0,
        Some(Message::OrbitPitchSurface(4, 2))
    ));
    // 用生产消息、规划器和实际源会话走完平移 / 缩放 / 定位 / 重置。
    app.spectator = false;
    app.surface_exposed = true;
    app.surface_pitch = 35;
    app.spectator_radius = 8;
    let flown = app.spectator_camera;
    let source = Arc::clone(&app.surface.as_ref().unwrap().2);
    app.world_orthographic_view()
        .projection(&mesh, bounds.size())
        .unwrap()
        .unwrap();
    let _ = app.maintain_spectator_stream();
    let old_center = app.spectator_stream.as_ref().unwrap().center;
    for yaw in [0, 45, 135, 270] {
        app.surface_yaw = yaw;
        for action in [
            crate::view_navigation::Action::Pan(iced::Vector::new(-14_000., 8_000.)),
            crate::view_navigation::Action::Zoom {
                steps: 2.,
                anchor: Point::new(151., 78.),
                center: Point::new(100., 100.),
            },
        ] {
            let before_view = app.world_orthographic_view();
            let before = before_view
                .projection(&mesh, bounds.size())
                .unwrap()
                .unwrap();
            let mut next = before_view.navigation;
            next.apply(action).unwrap();
            let expected = View {
                navigation: next,
                ..before_view.clone()
            }
            .projection(&mesh, bounds.size())
            .unwrap()
            .unwrap();
            let focus = expected.ground_focus().unwrap();
            let _ = crate::update_impl(&mut app, Message::Navigate(Target::Isometric, action));
            let after = app
                .world_orthographic_view()
                .projection(&mesh, bounds.size())
                .unwrap()
                .unwrap();
            for world in [
                focus,
                request.hit.owner.as_ref().unwrap().position.map(f64::from),
            ] {
                let a = expected.project(world).0;
                let b = after.project(world).0;
                assert!((a.x - b.x).abs() < 0.02 && (a.y - b.y).abs() < 0.02);
            }
            assert_ne!(before.project(focus).0, after.project(focus).0);
            assert_eq!(app.surface_navigation.pan, iced::Vector::ZERO);
            let (location, facing) = app.world_stream_pose().unwrap();
            for i in 0..3 {
                assert!((location[i] - focus[i]).abs() < 1e-7);
            }
            assert_eq!(facing, (180. - yaw as f32).rem_euclid(360.));
            let _ = app.maintain_spectator_stream();
            let stream = app.spectator_stream.as_ref().unwrap();
            assert_eq!(
                stream.center,
                (
                    (focus[0].floor() as i32).div_euclid(16),
                    (focus[2].floor() as i32).div_euclid(16)
                )
            );
            let plan = crate::spectator_stream::plan(location, 8).unwrap();
            assert!(plan
                .iter()
                .any(|p| (p.cx, p.cz) == stream.center && p.span == 1));
            assert_eq!(app.spectator_camera, flown);
            assert!(Arc::ptr_eq(&source, &app.surface.as_ref().unwrap().2));
        }
    }
    assert_ne!(app.spectator_stream.as_ref().unwrap().center, old_center);
    app.center_world_orthographic([29_999_990., 80., -29_999_990.])
        .unwrap();
    let before = app.world_stream_pose().unwrap();
    let navigation = app.surface_navigation;
    assert!(app
        .navigate_world_orthographic(crate::view_navigation::Action::Pan(iced::Vector::new(
            f32::MAX,
            f32::MAX
        )))
        .is_err());
    assert_eq!(app.world_stream_pose().unwrap(), before);
    assert_eq!(app.surface_navigation, navigation);
    let _ = crate::update_impl(&mut app, Message::ResetSurfaceCamera);
    let reset = app
        .world_orthographic_view()
        .projection(&mesh, bounds.size())
        .unwrap()
        .unwrap();
    assert_eq!(
        reset
            .project(request.hit.owner.as_ref().unwrap().position.map(f64::from))
            .0,
        Point::new(100., 100.)
    );
    let [x, y, z] = request.hit.owner.as_ref().unwrap().position;
    app.query_coordinates = format!("{},{},{}", x + 160, y + 2, z - 160);
    app.selected_surface = None;
    let _ = crate::update_impl(&mut app, Message::Center3DOnPicked);
    assert_eq!(
        app.world_stream_pose().unwrap().0,
        [(x + 160) as f64, (y + 2) as f64, (z - 160) as f64]
    );
    assert_eq!(app.spectator_camera, flown);
    assert_eq!(app.current_session((0, 0)).unwrap().pending_cells(), 0);
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Rendering {
        source_mesh_bounds_projection_and_parallel_ray_picking_agree_at_all_aspects();
        orthographic_uniform_preserves_f64_world_origin_pan_zoom_and_depth();
        scrolled_world_orthographic_controls_pick_original_source_and_rotate_by_raw_delta();
    }
}
