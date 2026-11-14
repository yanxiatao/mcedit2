use super::*;
use crate::{App, Message};

fn zoom_keeps_the_anchor_source_point_even_when_clamped_and_rejects_invalid_input() {
    let anchor = Point::new(713.25, 105.5);
    let center = Point::new(400., 300.);
    for zoom in [MIN_ZOOM, 0.5, 1., 12., MAX_ZOOM] {
        for steps in [-100., -3.5, 0.25, 4., 100.] {
            let mut view = Viewport {
                zoom,
                pan: Vector::new(123.5, -64.25),
            };
            let source = (anchor - center - view.pan) * (1. / view.zoom);
            view.apply(Action::Zoom {
                steps,
                anchor,
                center,
            })
            .unwrap();
            let screen = center + view.pan + source * view.zoom;
            assert!((screen.x - anchor.x).abs() < 0.02 && (screen.y - anchor.y).abs() < 0.02);
            assert!((MIN_ZOOM..=MAX_ZOOM).contains(&view.zoom));
        }
    }
    let mut view = Viewport::default();
    let original = view;
    for action in [
        Action::Pan(Vector::new(f32::INFINITY, 0.)),
        Action::centered_zoom(f32::NAN),
        Action::Zoom {
            steps: 1.,
            anchor: Point::new(f32::NAN, 0.),
            center,
        },
    ] {
        assert!(view.apply(action).is_err());
        assert_eq!(view, original);
    }
}

fn active_projection_zoom_and_reset_do_not_modify_other_views_or_source() {
    let (mut app, _, _, _) = crate::picking_integration_tests::fixture();
    let source = app.surface.clone();
    app.spectator = false;
    app.surface_yaw = 45;
    app.surface_pitch = 60;
    let _ = crate::update(&mut app, Message::ZoomSurface(4));
    let _ = crate::update(&mut app, Message::PanSurface(70, -25));
    let iso = app.surface_navigation;
    app.show_top_down = true;
    let _ = crate::update(&mut app, Message::ZoomSurface(-3));
    let _ = crate::update(&mut app, Message::PanSurface(-20, 40));
    let top = app.top_down_navigation;
    let cached_iso = std::sync::Arc::clone(&app.render_face_cache);
    app.show_top_down = false;
    app.spectator = true;
    let camera = app.spectator_camera;
    let _ = crate::update(&mut app, Message::ZoomSurface(3));
    assert!(app.spectator_camera.zoom > camera.zoom);
    assert_eq!(app.spectator_camera.position, camera.position);
    assert_eq!(app.spectator_camera.yaw, camera.yaw);
    assert_eq!(app.surface_navigation, iso);
    assert_eq!(app.top_down_navigation, top);
    assert!(std::sync::Arc::ptr_eq(&cached_iso, &app.render_face_cache));
    let zoomed_camera = app.spectator_camera;
    app.show_top_down = true;
    let _ = crate::update(&mut app, Message::ResetSurfaceCamera);
    assert_eq!(app.top_down_navigation, Viewport::default());
    assert_eq!(app.surface_navigation, iso);
    assert_eq!((app.surface_yaw, app.surface_pitch), (45, 60));
    assert_eq!(app.spectator_camera, zoomed_camera);
    if let Some((_, _, original)) = source {
        assert!(std::sync::Arc::ptr_eq(
            &original,
            &app.surface.as_ref().unwrap().2
        ));
    }
}

fn queued_middle_drag_uses_each_event_position_and_fractional_wheel_preserves_target() {
    let bounds = Rectangle {
        x: 20.,
        y: 40.,
        width: 200.,
        height: 100.,
    };
    let end_cursor = mouse::Cursor::Available(Point::new(120., 90.));
    let target = Target::Quad(QuadPane::Front);
    let mut gesture = Gesture::default();
    for event in [
        mouse::Event::CursorMoved {
            position: Point::new(30., 50.),
        },
        mouse::Event::ButtonPressed(mouse::Button::Middle),
    ] {
        gesture.update(target, &iced::Event::Mouse(event), bounds, end_cursor);
    }
    let action = gesture
        .update(
            target,
            &iced::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(50., 65.),
            }),
            bounds,
            end_cursor,
        )
        .unwrap();
    assert!(
        matches!(action.into_inner().0, Some(Message::Navigate(t, Action::Pan(v))) if t==target && v==Vector::new(20.,15.))
    );
    let click = gesture
        .update(
            target,
            &iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            end_cursor,
        )
        .unwrap();
    assert!(click.into_inner().0.is_none(), "平移过程中不能点选源方块");
    gesture.update(
        target,
        &iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)),
        bounds,
        mouse::Cursor::Unavailable,
    );
    assert!(!gesture.dragging());
    let action = gesture
        .update(
            target,
            &iced::Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0., y: 0.25 },
            }),
            bounds,
            end_cursor,
        )
        .unwrap();
    assert!(
        matches!(action.into_inner().0, Some(Message::Navigate(t,Action::Zoom {steps,anchor,center})) if t==target && steps==0.25 && anchor==Point::new(100.,50.) && center==Point::new(100.,50.))
    );
    gesture.update(
        target,
        &iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)),
        bounds,
        end_cursor,
    );
    gesture.update(
        target,
        &iced::Event::Window(iced::window::Event::Unfocused),
        bounds,
        end_cursor,
    );
    assert!(!gesture.dragging());
}

fn perspective_zoom_updates_the_same_projection_used_by_source_picking() {
    let (_, mut scene, mesh, original) = crate::picking_integration_tests::fixture();
    let owner = original.hit.owner.unwrap();
    let [x, y, z] = owner.position;
    let point = [x as f64 + 0.2, y as f64 + 0.2, z as f64 - 0.5];
    let size = iced::Size::new(800., 600.);
    for zoom in [0.25, 1., 4., 8.] {
        scene.camera.zoom = zoom;
        let uniform = scene.camera.uniform_at_origin(mesh.origin, size);
        let delta: [f64; 3] = std::array::from_fn(|i| point[i] - scene.camera.position[i]);
        let dot = |axis: [f32; 4]| (0..3).map(|i| delta[i] * f64::from(axis[i])).sum::<f64>();
        let depth = dot(uniform.forward);
        let screen = Point::new(
            (size.width as f64
                * 0.5
                * (1. + dot(uniform.right) * uniform.projection[0] as f64 / depth))
                as f32,
            (size.height as f64
                * 0.5
                * (1. - dot(uniform.up) * uniform.projection[1] as f64 / depth)) as f32,
        );
        let hit = mesh
            .pick(
                scene.camera,
                screen,
                crate::render_viewport::RenderViewport::one_to_one(size),
                scene.quality,
            )
            .unwrap();
        assert_eq!(hit.owner.as_ref(), Some(&owner));
    }
}

fn standalone_slice_viewports_are_independent_of_each_other_and_the_main_3d_view() {
    let mut app = App::default();
    app.navigate(Target::Horizontal, Action::centered_zoom(4.));
    app.navigate(Target::Vertical, Action::Pan(Vector::new(13., -27.)));
    assert!(app.slice_navigation.zoom > 1.);
    assert_eq!(app.slice_navigation.pan, Vector::ZERO);
    assert_eq!(app.vertical_navigation.zoom, 1.);
    assert_eq!(app.vertical_navigation.pan, Vector::new(13., -27.));
    assert_eq!(app.surface_navigation, Viewport::default());
    app.navigate(Target::Horizontal, Action::Reset);
    assert_eq!(app.slice_navigation, Viewport::default());
    assert_eq!(app.vertical_navigation.pan, Vector::new(13., -27.));
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Navigation {
        zoom_keeps_the_anchor_source_point_even_when_clamped_and_rejects_invalid_input();
        active_projection_zoom_and_reset_do_not_modify_other_views_or_source();
        queued_middle_drag_uses_each_event_position_and_fractional_wheel_preserves_target();
        perspective_zoom_updates_the_same_projection_used_by_source_picking();
        standalone_slice_viewports_are_independent_of_each_other_and_the_main_3d_view();
    }
}
