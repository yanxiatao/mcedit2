//! 使用真实样本、生产网格和 Canvas 事件走完整条笔画，不创建额外测试入口。
use super::*;
use crate::picking_integration_tests::{edit_task_output, finish_edit_preview, fixture};
use iced::{widget::canvas::Program, Rectangle, Size};

pub(super) fn paint<C: Program<Message>>(
    controls: &C,
    start: Point,
    end: Point,
    bounds: Rectangle,
) -> Request {
    let mut state = C::State::default();
    let translation = iced::Vector::new(0., 320.);
    let cursor = mouse::Cursor::Available(end);
    let mut actual = mouse::Cursor::Available(start);
    for event in [
        mouse::Event::CursorMoved {
            position: start - translation,
        },
        mouse::Event::ButtonPressed(mouse::Button::Left),
        mouse::Event::CursorMoved {
            position: end - translation,
        },
    ] {
        let event = canvas::Event::Mouse(event);
        if let canvas::Event::Mouse(mouse::Event::CursorMoved { position }) = &event {
            actual = mouse::Cursor::Available(*position + translation);
        }
        let event_cursor = actual;
        let action = controls.update(&mut state, &event, bounds, event_cursor);
        if let Some(action) = action {
            assert!(!matches!(
                action.into_inner().0,
                Some(Message::Brush(Action::Finished(_)))
            ));
        }
    }
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
    let Message::Brush(Action::Finished(result)) = message else {
        panic!("应提交完整笔画");
    };
    let request = result.unwrap();
    assert_eq!(request.sources.len(), 2);
    assert_eq!(request.strokes().unwrap().count(), 2);
    request
}

pub(super) fn cancellations<C: Program<Message>>(controls: &C, start: Point, bounds: Rectangle) {
    for event in [
        canvas::Event::Mouse(mouse::Event::CursorLeft),
        canvas::Event::Window(iced::window::Event::Unfocused),
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
        canvas::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0., y: 1. },
        }),
    ] {
        let mut state = C::State::default();
        let cursor = mouse::Cursor::Available(start);
        let motion = canvas::Event::Mouse(mouse::Event::CursorMoved { position: start });
        let event_cursor = cursor;
        controls.update(&mut state, &motion, bounds, event_cursor);
        controls
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds,
                cursor,
            )
            .unwrap();
        let action = controls.update(&mut state, &event, bounds, cursor).unwrap();
        assert!(matches!(
            action.into_inner().0,
            Some(Message::Brush(Action::Finished(Err(_))))
        ));
        let release = controls.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            bounds,
            cursor,
        );
        assert!(release.is_none_or(|a| !matches!(
            a.into_inner().0,
            Some(Message::Brush(Action::Finished(Ok(_))))
        )));
    }
}

pub(super) fn continuous_stroke_pipeline() {
    for ortho in [false, true] {
        for mode in [Mode::Fill, Mode::Replace, Mode::Biome] {
            let (mut app, scene, mesh, original) = two_voxel_fixture();
            app.manual_textures = true;
            app.block_edit.stroke = Settings {
                enabled: true,
                mode,
                size: "1,1,1".into(),
                offset: "0".into(),
                shape: None,
                hollow: ortho,
                ..Default::default()
            };
            app.edit_target = "numeric:41[data=0]".into();
            app.edit_filter =
                mcedit2_world::editing::render_state(&original.hit.owner.as_ref().unwrap().state);
            app.block_edit.stroke.rules = vec![
                Rule {
                    sources: format!("{} | numeric:1[data=0]", app.edit_filter),
                    target: "numeric:42[data=0]".into(),
                },
                Rule {
                    sources: app.edit_filter.clone(),
                    target: app.edit_target.clone(),
                },
                Rule {
                    sources: app.edit_target.clone(),
                    target: "numeric:43[data=0]".into(),
                },
            ];
            let key = app.current_key().unwrap();
            let position = original.hit.owner.as_ref().unwrap().position;
            let world = &app.worlds[key.0];
            let dimension = &world.dimensions[key.1].name;
            let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
                world,
                dimension,
                position[0].div_euclid(16),
                position[2].div_euclid(16),
            )
            .unwrap()
            .unwrap();
            let decoded = mcedit2_world::sections::decode_sections(&raw).unwrap();
            let old = decoded
                .numeric_biome(
                    position[0].rem_euclid(16) as u8,
                    position[1],
                    position[2].rem_euclid(16) as u8,
                )
                .unwrap();
            let biome_target = (old + 1) % 256;
            app.block_edit.biome_target = biome_target.to_string();
            if ortho {
                let projection = crate::orthographic_world::View {
                    angles: (0_f32.to_radians(), 35_f32.to_radians()),
                    navigation: Default::default(),
                    framing: Default::default(),
                }
                .projection(&mesh, Size::new(200., 200.))
                .unwrap()
                .unwrap();
                *app.spectator_presented_view.lock().unwrap() = Some(PresentedView {
                    camera: projection.into(),
                    viewport: crate::render_viewport::RenderViewport::one_to_one(Size::new(
                        200., 200.,
                    )),
                    quality: scene.quality,
                    atlas: mesh.atlas.clone(),
                    mesh: Arc::clone(&mesh),
                });
            }
            let frame = app
                .spectator_presented_view
                .lock()
                .unwrap()
                .clone()
                .unwrap();
            let mut hits = BTreeMap::new();
            for y in (2..198).step_by(4) {
                for x in (2..198).step_by(4) {
                    let point = Point::new(x as f32, y as f32);
                    if let Some(owner) = mesh
                        .pick(frame.camera, point, frame.viewport, frame.quality)
                        .and_then(|h| h.owner)
                    {
                        hits.entry(owner.position).or_insert(point);
                    }
                }
            }
            assert_eq!(hits.len(), 2);
            let bounds = Rectangle {
                x: 20.,
                y: 500.,
                width: 200.,
                height: 200.,
            };
            let mut points = hits
                .values()
                .map(|p| Point::new(p.x + bounds.x, p.y + bounds.y));
            let start = points.next().unwrap();
            let end = points.next().unwrap();
            let request = if ortho {
                let controls = crate::orthographic_world::Controls {
                    selection_handle: None,
                    presented: Arc::clone(&app.spectator_presented),
                    presented_view: Arc::clone(&app.spectator_presented_view),
                    key: Some(key),
                    selected: None,
                    selection: None,
                    show_frame: false,
                    show_axes: false,
                    show_chunk_grid: false,
                    show_section_guides: false,
                    brush: config(&app),
                };
                cancellations(&controls, start, bounds);
                paint(&controls, start, end, bounds)
            } else {
                let controls = crate::spectator::Controls {
                    workplane: None,
                    selection_handle: None,
                    presented: Arc::clone(&app.spectator_presented),
                    presented_view: Arc::clone(&app.spectator_presented_view),
                    key: Some(key),
                    selected: None,
                    selection: None,
                    flying: false,
                    brush: config(&app),
                };
                cancellations(&controls, start, bounds);
                paint(&controls, start, end, bounds)
            };
            let saved_request = request.clone();
            let _ = update(&mut app, Action::Finished(Ok(request)));
            assert!(app.block_edit.busy);
            finish_edit_preview(&mut app);
            assert!(app.block_edit.draft.is_some(), "{}", app.edit_report);
            assert!(app.block_edit.has_prepared());
            assert!(!app.sessions[&key].has_pending_changes());
            let candidate = app.display_session(key).unwrap();
            if mode == Mode::Biome {
                let staged = candidate
                    .staged_biome_chunk(position[0].div_euclid(16), position[2].div_euclid(16))
                    .unwrap()
                    .unwrap();
                let data = mcedit2_world::sections::decode_sections(staged).unwrap();
                for bounds in saved_request.strokes().unwrap() {
                    let bounds = bounds.unwrap();
                    assert_eq!(
                        data.numeric_biome(
                            bounds.minimum[0].rem_euclid(16) as u8,
                            bounds.minimum[1],
                            bounds.minimum[2].rem_euclid(16) as u8
                        ),
                        Some(biome_target)
                    );
                }
            } else {
                assert!(candidate.pending_cells() > 0);
            }
            let task = crate::update(&mut app, Message::ApplyEdit);
            let message = edit_task_output(task);
            let _ = crate::update(&mut app, message);
            assert!(app.sessions[&key].has_pending_changes());
            app.sessions.get_mut(&key).unwrap().undo().unwrap();
            assert!(!app.sessions[&key].has_pending_changes());
            app.sessions.get_mut(&key).unwrap().redo().unwrap();
            assert!(app.sessions[&key].has_pending_changes());
            let world = &app.worlds[key.0];
            assert_eq!(
                mcedit2_world::block_lookup::read_world_chunk_bytes(
                    world,
                    &world.dimensions[key.1].name,
                    position[0].div_euclid(16),
                    position[2].div_euclid(16)
                )
                .unwrap()
                .unwrap(),
                raw
            );
        }
    }
    stale_sources_and_parameter_changes();
}

pub(super) fn two_voxel_fixture() -> (
    App,
    crate::spectator::SpectatorScene,
    Arc<StreamMesh>,
    crate::picking::Request,
) {
    use mcedit2_world::surface::{SurfaceVoxel, VoxelData, ALL_FACES};
    let (mut app, mut scene, _, mut original) = fixture();
    let [x, y, z] = original.hit.owner.as_ref().unwrap().position;
    let world = &app.worlds[0];
    let chunk = mcedit2_world::block_lookup::read_world_chunk(
        world,
        &world.dimensions[0].name,
        x.div_euclid(16),
        z.div_euclid(16),
    )
    .unwrap()
    .unwrap();
    let state = chunk
        .block((x + 1).rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
        .unwrap()
        .clone();
    Arc::make_mut(&mut scene.surface)
        .voxels
        .push(SurfaceVoxel::new(
            [x + 1, y, z],
            VoxelData {
                kind: mcedit2_world::surface::material(&state),
                state: state.into(),
                faces: ALL_FACES,
                biome: None,
                staged: false,
                original_block_entity: chunk.has_block_entity(x + 1, y, z),
                span: 1,
                fluid: None,
                light: Default::default(),
            },
        ));
    scene.camera.position[0] += 0.5;
    scene.chunk_cache = Default::default();
    scene.cache = Default::default();
    let mesh = Arc::new(crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    let viewport = crate::render_viewport::RenderViewport::one_to_one(Size::new(200., 200.));
    original.hit = mesh
        .pick(
            scene.camera,
            Point::new(100., 100.),
            viewport,
            scene.quality,
        )
        .unwrap();
    original.mesh = Arc::clone(&mesh);
    app.surface = Some((0, 0, Arc::clone(&scene.surface)));
    *app.spectator_presented.lock().unwrap() = Some(Arc::clone(&mesh));
    *app.spectator_presented_view.lock().unwrap() = Some(PresentedView {
        camera: scene.camera.into(),
        viewport,
        quality: scene.quality,
        atlas: mesh.atlas.clone(),
        mesh: Arc::clone(&mesh),
    });
    (app, scene, mesh, original)
}

fn stale_sources_and_parameter_changes() {
    let (mut app, _scene, mesh, original) = fixture();
    app.manual_textures = true;
    app.block_edit.stroke = Settings {
        enabled: true,
        mode: Mode::Fill,
        size: "1,1,1".into(),
        offset: "0".into(),
        shape: None,
        ..Default::default()
    };
    app.edit_target = "numeric:41[data=0]".into();
    let config = config(&app).unwrap();
    let frame = app
        .spectator_presented_view
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let sample = Sample::from_hit(&original.hit).unwrap();
    let large = Config {
        size: "32,8,32".into(),
        ..config.clone()
    };
    let large = Drag::start(
        large,
        (0, 0),
        Source::World(Arc::clone(&frame.mesh)),
        &sample,
    )
    .unwrap()
    .finish(Source::World(Arc::clone(&frame.mesh)))
    .unwrap();
    assert_eq!(large.path.segment_count(), 1);
    assert_eq!(
        large
            .strokes()
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .volume()
            .unwrap(),
        8192
    );
    let mut long_path = BrushPath::default();
    long_path.push([0, 64, 0]).unwrap();
    long_path.push([100_000, 64, 0]).unwrap();
    assert_eq!(long_path.segment_count(), 2);
    assert_eq!(long_path.points().count(), 100_001);
    let mut drag = Some(
        Drag::start(
            config.clone(),
            (0, 0),
            Source::World(Arc::clone(&frame.mesh)),
            &sample,
        )
        .unwrap(),
    );
    let changed = Config {
        offset: "1".into(),
        ..config.clone()
    };
    let action = event(
        Some(&changed),
        &mut drag,
        Some((0, 0)),
        &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        Some(Point::new(100., 100.)),
        keyboard::Modifiers::default(),
        || Some(InputFrame::world(frame.clone(), Point::new(100., 100.))),
    )
    .unwrap();
    assert!(matches!(
        action.into_inner().0,
        Some(Message::Brush(Action::Finished(Err(_))))
    ));
    assert!(drag.is_none());
    let request = Drag::start(
        config,
        (0, 0),
        Source::World(Arc::clone(&frame.mesh)),
        &sample,
    )
    .unwrap()
    .finish(Source::World(mesh))
    .unwrap();
    let owner = original.hit.owner.as_ref().unwrap();
    let [x, y, z] = owner.position;
    let source = app.current_session((0, 0)).unwrap();
    let p = source
        .prepare(
            Bounds::new((x, y, z), (x, y, z)).unwrap(),
            &mcedit2_world::editing::parse_state("numeric:41[data=0]").unwrap(),
            None,
        )
        .unwrap();
    source.commit(p).unwrap();
    let _ = update(&mut app, Action::Finished(Ok(request)));
    finish_edit_preview(&mut app);
    assert!(app.block_edit.draft.is_none());
    assert_eq!(app.sessions[&(0, 0)].pending_cells(), 1);
}
