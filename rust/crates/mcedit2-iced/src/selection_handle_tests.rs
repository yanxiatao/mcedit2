//! 真实源、五种 Canvas 路径和共享选框手势的集中验证，不新增测试入口。
use super::*;
use crate::{metadata_fixture::Fixture, metadata_tests, selection_edit};
use iced::{widget::canvas::Program, Vector};
fn act(app: &mut App, action: selection_edit::Action) {
    let _ = crate::update(app, Message::SelectionEdit(action));
}
fn event<C: Program<Message>>(
    view: &C,
    state: &mut C::State,
    event: canvas::Event,
    point: Point,
    rect: Rectangle,
) -> Option<Message> {
    view.update(state, &event, rect, mouse::Cursor::Available(point))
        .and_then(|a| a.into_inner().0)
}
fn drag<C: Program<Message>>(view: &C, start: Point, end: Point, rect: Rectangle) -> Message {
    let mut state = C::State::default();
    for (kind, p) in [
        (
            mouse::Event::CursorMoved {
                position: start - Vector::new(0., 320.),
            },
            start,
        ),
        (mouse::Event::ButtonPressed(mouse::Button::Left), start),
        (
            mouse::Event::CursorMoved {
                position: end - Vector::new(0., 320.),
            },
            end,
        ),
    ] {
        let output = event(view, &mut state, canvas::Event::Mouse(kind), p, rect);
        assert!(!matches!(
            output,
            Some(Message::SelectionEdit(selection_edit::Action::Handle(_)))
        ));
    }
    event(
        view,
        &mut state,
        canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        end,
        rect,
    )
    .unwrap()
}
fn accept(app: &mut App, message: Message) -> SelectionBox {
    assert!(matches!(
        &message,
        Message::SelectionEdit(selection_edit::Action::Handle(_))
    ));
    let _ = crate::update(app, message);
    current(app).unwrap()
}
fn world_views() {
    let (mut app, scene, mesh, hit) = crate::picking_integration_tests::fixture();
    app.manual_textures = true;
    app.surface_pitch = 35;
    let key = app.current_key().unwrap();
    let revision = app.current_session(key).unwrap().revision();
    act(&mut app, selection_edit::Action::Mouse(true));
    act(&mut app, selection_edit::Action::Classic(true));
    let rect = Rectangle {
        x: 20.,
        y: 500.,
        width: 200.,
        height: 200.,
    };
    for ortho in [false, true] {
        act(&mut app, selection_edit::Action::Clear);
        if ortho {
            let projection = crate::orthographic_world::View {
                angles: (0_f32.to_radians(), 35_f32.to_radians()),
                navigation: Default::default(),
                framing: Default::default(),
            }
            .projection(&mesh, rect.size())
            .unwrap()
            .unwrap();
            *app.spectator_presented_view.lock().unwrap() = Some(crate::spectator::PresentedView {
                camera: projection.into(),
                viewport: crate::render_viewport::RenderViewport::one_to_one(rect.size()),
                quality: scene.quality,
                atlas: mesh.atlas.clone(),
                mesh: Arc::clone(&mesh),
            });
        }
        let shown = app
            .spectator_presented_view
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        let mut points = Vec::new();
        for y in (2..198).step_by(6) {
            for x in (2..198).step_by(6) {
                let p = Point::new(x as f32, y as f32);
                if shown.pick(p).is_some_and(|h| h.owner.is_some()) {
                    points.push(p + Vector::new(rect.x, rect.y));
                }
            }
        }
        assert!(points.len() > 1);
        let message = if ortho {
            let controls = crate::orthographic_world::Controls {
                selection_handle: config(&app),
                presented: Arc::clone(&app.spectator_presented),
                presented_view: Arc::clone(&app.spectator_presented_view),
                key: Some(key),
                selected: None,
                selection: None,
                show_frame: false,
                show_axes: false,
                show_chunk_grid: false,
                show_section_guides: false,
                brush: None,
            };
            drag(&controls, points[0], *points.last().unwrap(), rect)
        } else {
            let controls = crate::spectator::Controls {
                workplane: None,
                selection_handle: config(&app),
                presented: Arc::clone(&app.spectator_presented),
                presented_view: Arc::clone(&app.spectator_presented_view),
                key: Some(key),
                selected: None,
                selection: None,
                flying: false,
                brush: None,
            };
            drag(&controls, points[0], *points.last().unwrap(), rect)
        };
        let b = accept(&mut app, message);
        assert!(b.bounds().unwrap().is_some());
        assert!(Arc::ptr_eq(&revision, &app.sessions[&key].revision()));
        assert_eq!(app.sessions[&key].undo_depth(), 0);
    }
    let _ = hit;
}
fn planar_views_and_history() {
    let mut f = Fixture::new(3955);
    crate::object_fixture::populate(&mut f);
    let terrain = f.file("region/r.0.0.mca");
    let entities = f.file("entities/r.0.0.mca");
    let mut app = metadata_tests::app(&f);
    app.surface_pitch = 35;
    app.surface_area = "0,0,0;7,7,7".into();
    let _ = crate::update_impl(&mut app, Message::InspectVolume);
    let job = app.near_job.take().unwrap();
    let result = job.read();
    app.near_loaded(job.nonce, job.key, result);
    let key = app.current_key().unwrap();
    let revision = app.current_session(key).unwrap().revision();
    let source = Arc::clone(&app.surface.as_ref().unwrap().2);
    act(&mut app, selection_edit::Action::Mouse(true));
    act(&mut app, selection_edit::Action::Clear);
    let rect = Rectangle {
        x: 30.,
        y: 400.,
        width: 240.,
        height: 240.,
    };
    let top = crate::top_down::TopDown {
        selection_handle: config(&app),
        brush: None,
        brush_key: Some(key),
        layers: None,
        chunk_column: None,
        source: Arc::clone(&source),
        zoom: 1.,
        pan_x: 0.,
        pan_y: 0.,
        selected: None,
        texture_bank: None,
        show_textures: false,
        block_layers: Default::default(),
        entity_frame: None,
        grid_cache: Default::default(),
        lighting: crate::lighting::Table::unlit(),
        quality: Default::default(),
        source_revision: 0,
    };
    let mapping = top.entity_projection(rect.size()).unwrap().0;
    let start = mapping.project([0., 0., 0.]).0 + Vector::new(rect.x, rect.y);
    let result = accept(
        &mut app,
        drag(&top, start, start + Vector::new(20., 30.), rect),
    );
    assert_eq!((result.minimum[1], result.maximum[1]), (1, 1));
    assert!(app.edit_area.is_empty());
    assert!(app.selection_box().is_none());
    act(&mut app, selection_edit::Action::ToggleMode);
    act(&mut app, selection_edit::Action::End(1, "2".into()));
    act(&mut app, selection_edit::Action::Apply);
    assert!(app.selection_plane.is_none());
    assert_eq!(app.selection_box().unwrap().minimum[1], 1);
    assert_eq!(app.selection_box().unwrap().maximum[1], 2);
    act(&mut app, selection_edit::Action::History(false));
    assert_eq!(app.selection_plane, Some(result));
    // 经典黏性创建从俯视跨到水平切片；所有窗格读取同一手势与预览。
    act(&mut app, selection_edit::Action::Clear);
    act(&mut app, selection_edit::Action::Classic(true));
    act(&mut app, selection_edit::Action::Sticky(true));
    let top = crate::top_down::TopDown {
        selection_handle: config(&app),
        ..top
    };
    let mut state = crate::scene_layers::MapState::default();
    event(
        &top,
        &mut state,
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        start,
        rect,
    );
    assert!(event(
        &top,
        &mut state,
        canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        start,
        rect
    )
    .is_none());
    assert!(app.selection_edit.pointer.lock().unwrap().active());
    assert!(current(&app).is_none());
    let slice = Arc::new(
        app.sessions[&key]
            .inspect_slice(Bounds::spatial((0, 0, 0), (7, 0, 7)).unwrap())
            .unwrap(),
    );
    app.slice_grid = Some((key.0, key.1, Arc::clone(&slice)));
    let map = crate::slice_canvas::Map2D {
        selection_handle: config(&app),
        brush: None,
        brush_key: Some(key),
        brush_source: Some(Source::Horizontal(Arc::clone(&slice), Target::Horizontal)),
        layers: None,
        chunk_column: None,
        navigation: Default::default(),
        navigation_target: Target::Horizontal,
        plane: crate::slice_canvas::Plane::Horizontal(&slice),
        selected: None,
        texture_bank: None,
        show_textures: false,
        block_layers: Default::default(),
        entity_frame: None,
        grid_cache: Default::default(),
        lighting: crate::lighting::Table::unlit(),
        quality: Default::default(),
        source_revision: 0,
    };
    let end = Point::new(
        rect.x + rect.width * 0.5 + 3.5 * 28.,
        rect.y + rect.height * 0.5 + 3.5 * 28.,
    );
    let mut mapstate = crate::scene_layers::MapState::default();
    event(
        &map,
        &mut mapstate,
        canvas::Event::Mouse(mouse::Event::CursorMoved {
            position: end - Vector::new(0., 320.),
        }),
        end,
        rect,
    );
    let msg = event(
        &map,
        &mut mapstate,
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        end,
        rect,
    )
    .unwrap();
    let selected = accept(&mut app, msg);
    assert_eq!(selected.minimum, [0, 0, 0]);
    assert_eq!(selected.maximum, [8, 1, 8]);
    assert!(!app.selection_edit.pointer.lock().unwrap().active());
    // 限定等轴同样通过真实源几何拾取创建三维框。
    act(&mut app, selection_edit::Action::Clear);
    act(&mut app, selection_edit::Action::Sticky(false));
    let assets = crate::textures::java_obj_tests::Fixture::new();
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[180; 16], 2, 2, image::ExtendedColorType::Rgba8)
        .unwrap();
    assets.put("assets/minecraft/textures/block/stone.png", &png);
    let mut iso = crate::textures::java_obj_tests::isometric_scene(assets.open());
    iso.surface = Arc::clone(&source);
    iso.texture_bank = None;
    iso.show_textures = false;
    iso.selection_handle = config(&app);
    iso.navigation_target = Target::Isometric;
    let faces = iso.faces_cached(rect.size());
    let point = faces
        .iter()
        .find_map(|f| {
            let p = Point::new(
                f.corners.iter().map(|p| p.x).sum::<f32>() / 4.,
                f.corners.iter().map(|p| p.y).sum::<f32>() / 4.,
            );
            iso.pick_face(p, rect.size()).map(|_| p)
        })
        .unwrap()
        + Vector::new(rect.x, rect.y);
    accept(&mut app, drag(&iso, point, point, rect));
    assert!(Arc::ptr_eq(&revision, &app.sessions[&key].revision()));
    assert_eq!(app.sessions[&key].undo_depth(), 0);
    assert_eq!(f.file("region/r.0.0.mca"), terrain);
    assert_eq!(f.file("entities/r.0.0.mca"), entities);
}
fn resize_move_cancel() {
    let f = Fixture::new(3955);
    let mut app = metadata_tests::app(&f);
    app.surface_pitch = 35;
    act(&mut app, selection_edit::Action::Mouse(true));
    let _ = crate::update(&mut app, Message::SelectionText("0,0,0;3,3,3".into()));
    let size = Size::new(200., 200.);
    let rect = Rectangle::with_size(size);
    let view = View::Planar(
        Mapping::Orthogonal {
            origin: [0.; 3],
            screen: [100.; 2],
            right: [1., 0., 0.],
            down: [0., 0., 1.],
            depth: [0., 1., 0.],
            scale: 10.,
        },
        size,
        ([-10.; 3], [10.; 3]),
    );
    fn send(
        layer: &Layer,
        view: View,
        kind: canvas::Event,
        p: Point,
        rect: Rectangle,
    ) -> Option<Message> {
        layer
            .event(
                Target::TopDown,
                Some(view),
                &kind,
                rect,
                mouse::Cursor::Available(p),
                |_| None,
            )
            .and_then(|a| a.into_inner().0)
    }
    let layer = config(&app).unwrap();
    let start = Point::new(135., 115.);
    let end = Point::new(165., 115.);
    send(
        &layer,
        view,
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        start,
        rect,
    );
    send(
        &layer,
        view,
        canvas::Event::Mouse(mouse::Event::CursorMoved { position: end }),
        end,
        rect,
    );
    assert_eq!(current(&app).unwrap().maximum, [4; 3]);
    let msg = send(
        &layer,
        view,
        canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        end,
        rect,
    )
    .unwrap();
    let b = accept(&mut app, msg);
    assert_eq!(b.maximum, [7, 4, 4]);
    act(&mut app, selection_edit::Action::History(false));
    assert_eq!(current(&app).unwrap().maximum, [4; 3]);
    act(&mut app, selection_edit::Action::History(true));
    assert_eq!(current(&app).unwrap(), b);
    let layer = config(&app).unwrap();
    let start = Point::new(110., 110.);
    let end = Point::new(95., 130.);
    send(
        &layer,
        view,
        canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(
            keyboard::Modifiers::SHIFT,
        )),
        start,
        rect,
    );
    send(
        &layer,
        view,
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        start,
        rect,
    );
    let msg = send(
        &layer,
        view,
        canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        end,
        rect,
    )
    .unwrap();
    let b = accept(&mut app, msg);
    assert_eq!(b.minimum, [-1, 0, 2]);
    assert_eq!(b.maximum, [6, 4, 6]);
    let layer = config(&app).unwrap();
    send(
        &layer,
        view,
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        Point::new(155., 135.),
        rect,
    );
    assert!(app.selection_edit.pointer.lock().unwrap().active());
    let escape = keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Escape),
        modified_key: keyboard::Key::Named(keyboard::key::Named::Escape),
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        text: None,
        repeat: false,
    };
    send(
        &layer,
        view,
        canvas::Event::Keyboard(escape),
        Point::new(155., 135.),
        rect,
    );
    assert!(!app.selection_edit.pointer.lock().unwrap().active());
    assert_eq!(current(&app), Some(b));
    assert!(SelectionBox {
        minimum: [0, 4096, 0],
        maximum: [1, 4096, 1]
    }
    .bounds()
    .unwrap()
    .is_none());
    // 过期所属世界与原范围不得覆盖当前选区。
    act(
        &mut app,
        selection_edit::Action::Handle(Finished {
            config: Config {
                owner: crate::keyboard_bindings::Owner::World(99, 0),
                ..layer.config
            },
            selection: b,
            source: None,
        }),
    );
    assert_eq!(current(&app), Some(b));
    act(&mut app, selection_edit::Action::Classic(true));
    assert!(crate::brush_stroke::config(&app).is_none());
    let _ = crate::brush_stroke::update(&mut app, crate::brush_stroke::Action::Enabled(true));
    assert!(!app.selection_edit.mouse_enabled);
    // 近裁剪面与大坐标仍生成有限的实际显示线段。
    let camera = crate::spectator::Camera {
        position: [29_999_990., 64., 29_999_990.],
        yaw: 35.,
        pitch: 25.,
        ..Default::default()
    };
    let (_, _, forward) = camera.basis();
    let at = |distance: f64| {
        std::array::from_fn(|i| camera.position[i] + f64::from(forward[i]) * distance)
    };
    let projected = View::World(camera.into(), size)
        .segment(at(0.), at(4.))
        .unwrap();
    assert!(
        projected.0.x.is_finite()
            && projected.0.y.is_finite()
            && projected.1.x.is_finite()
            && projected.1.y.is_finite()
    );
    act(&mut app, selection_edit::Action::Mouse(true));
    let retained = Arc::clone(&app.selection_edit.pointer);
    let layer = config(&app).unwrap();
    send(
        &layer,
        view,
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        Point::new(155., 135.),
        rect,
    );
    assert!(retained.lock().unwrap().active());
    let _ = crate::update(
        &mut app,
        Message::Workspace(crate::workspace_tabs::Action::Close(
            crate::workspace_tabs::CloseTarget::World(0),
        )),
    );
    assert!(!retained.lock().unwrap().active());
}
pub(super) fn run(category: crate::test_categories::Category) {
    match category {
        crate::test_categories::Category::Rendering => {
            world_views();
            planar_views_and_history();
            resize_move_cancel();
        }
        _ => (),
    }
}
