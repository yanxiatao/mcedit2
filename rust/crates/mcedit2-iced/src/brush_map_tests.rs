//! 真实世界快照上的投影笔画与读取代次验证，接入已有类别。
use super::*;
use crate::{picking_integration_tests::edit_task_output, view_navigation::QuadPane};
use iced::{Rectangle, Size};
use mcedit2_world::editing::SliceCell;
use tests::{cancellations, paint, two_voxel_fixture};

fn navigation<C: iced::widget::canvas::Program<Message>>(
    view: &C,
    start: Point,
    bounds: Rectangle,
) {
    let mut state = C::State::default();
    let end = Point::new(start.x + 7., start.y + 4.);
    let translation = iced::Vector::new(0., 320.);
    for event in [
        mouse::Event::CursorMoved {
            position: start - translation,
        },
        mouse::Event::ButtonPressed(mouse::Button::Middle),
    ] {
        let event = canvas::Event::Mouse(event);
        let actual = mouse::Cursor::Available(start);
        view.update(&mut state, &event, bounds, actual);
    }
    let event = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
    let actual = mouse::Cursor::Available(start);
    let action = view.update(&mut state, &event, bounds, actual).unwrap();
    assert!(action.into_inner().0.is_none(), "中键导航期间不能启动笔画");
    let event = canvas::Event::Mouse(mouse::Event::CursorMoved {
        position: end - translation,
    });
    let actual = mouse::Cursor::Available(end);
    let action = view.update(&mut state, &event, bounds, actual).unwrap();
    assert!(
        matches!(action.into_inner().0,Some(Message::Navigate(_,crate::view_navigation::Action::Pan(v))) if v==iced::Vector::new(7.,4.)),
        "笔刷不能截断导航移动"
    );
}

fn outputs(task: Task<Message>) -> Vec<Message> {
    use iced::futures::StreamExt;
    let Some(mut stream) = iced_runtime::task::into_stream(task) else {
        return Vec::new();
    };
    iced::futures::executor::block_on(async move {
        let mut result = Vec::new();
        while let Some(action) = stream.next().await {
            let iced_runtime::Action::Output(message) = action else {
                panic!("无窗口验证只应产生应用消息");
            };
            result.push(message);
        }
        result
    })
}
fn drain(app: &mut App, task: Task<Message>) {
    for message in outputs(task) {
        let task = crate::update(app, message);
        drain(app, task);
    }
}
fn iso(app: &App, view: Arc<SurfaceView>, target: Target) -> crate::isometric::IsometricSurface {
    crate::isometric::IsometricSurface {
        selection_handle: None,
        brush: config(app),
        brush_key: app.current_key(),
        surface: view,
        texture_bank: None,
        show_textures: false,
        block_layers: Default::default(),
        camera_quarters: 0,
        yaw_degrees: 45,
        pitch_degrees: 35,
        zoom: 1.,
        navigation_target: target,
        pan_x: 0.,
        pan_y: 0.,
        selected: None,
        hovered: None,
        selection_box: None,
        show_frame: false,
        show_axes: false,
        show_chunk_grid: false,
        show_section_guides: false,
        gpu_faces: true,
        gpu_quality: Default::default(),
        frame_cache: Default::default(),
        framing_cache: Default::default(),
        canvas_cache: Default::default(),
        gpu_frame_cache: Default::default(),
        gpu_pending: Default::default(),
        entity_frame: None,
        entity_clip: None,
        rendered_cache: Default::default(),
        presented: Default::default(),
        lighting: crate::lighting::Table::unlit(),
    }
}
fn iso_points(view: &crate::isometric::IsometricSurface, size: Size) -> (Point, Point) {
    let faces = view.faces_cached(size);
    let mut points = BTreeMap::<[i32; 3], Point>::new();
    for face in faces.iter().rev() {
        let point = Point::new(
            face.corners.iter().map(|p| p.x).sum::<f32>() / 4.,
            face.corners.iter().map(|p| p.y).sum::<f32>() / 4.,
        );
        if let Some(((x, y, z), _)) = view.pick_face(point, size) {
            let p = [x, y, z];
            if view
                .surface
                .voxels
                .iter()
                .any(|v| v.x == x && v.y == y && v.z == z && v.original_block_entity)
            {
                continue;
            }
            for (other, at) in &points {
                if (0..3).map(|i| (other[i] - p[i]).abs()).max() == Some(1) {
                    return (*at, point);
                }
            }
            points.entry(p).or_insert(point);
        }
    }
    panic!("真实投影样本应含两个相邻可见源方块")
}
fn map_request(app: &App, source: Source) -> Request {
    use crate::slice_canvas::{Map2D, Plane};
    let (plane, target) = match &source {
        Source::Horizontal(p, t) => (Plane::Horizontal(p), *t),
        Source::Vertical(p, t) => (Plane::Vertical(p), *t),
        _ => panic!("切片来源类型错误"),
    };
    let map = Map2D {
        selection_handle: None,
        chunk_column: None,
        brush: config(app),
        brush_key: app.current_key(),
        brush_source: Some(source.clone()),
        layers: None,
        navigation: Default::default(),
        navigation_target: target,
        plane,
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
    let (cols, rows) = match plane {
        Plane::Horizontal(p) => (p.rows[0].len(), p.rows.len()),
        Plane::Vertical(p) => (p.rows[0].len(), p.rows.len()),
    };
    let mut chosen = None;
    for row in 0..rows {
        for col in 0..cols.saturating_sub(1) {
            let cells = match plane {
                Plane::Horizontal(p) => &p.rows,
                Plane::Vertical(p) => &p.rows,
            };
            if matches!(
                &cells[row][col],
                SliceCell::Present {
                    original_block_entity: false,
                    ..
                }
            ) && matches!(
                &cells[row][col + 1],
                SliceCell::Present {
                    original_block_entity: false,
                    ..
                }
            ) {
                chosen = Some((row, col));
                break;
            }
        }
        if chosen.is_some() {
            break;
        }
    }
    let (row, col) = chosen.expect("真实切片应有两个相邻源格");
    let bounds = Rectangle {
        x: 20.,
        y: 500.,
        width: 300.,
        height: 300.,
    };
    let cell = (300. / cols as f32).min(300. / rows as f32).min(28.);
    let x = bounds.x + (300. - cols as f32 * cell) * 0.5 + (col as f32 + 0.5) * cell;
    let y = bounds.y + (300. - rows as f32 * cell) * 0.5 + (row as f32 + 0.5) * cell;
    let start = Point::new(x, y);
    let end = Point::new(x + cell, y);
    navigation(&map, start, bounds);
    cancellations(&map, start, bounds);
    paint(&map, start, end, bounds)
}
fn setup(mode: Mode) -> (App, Arc<SurfaceView>) {
    let (mut app, scene, _, _) = two_voxel_fixture();
    app.manual_textures = true;
    app.block_edit.stroke = Settings {
        enabled: true,
        mode,
        size: "1,1,1".into(),
        offset: "0".into(),
        shape: None,
        ..Default::default()
    };
    app.edit_target = "numeric:41[data=0]".into();
    (app, scene.surface)
}
fn finish(app: &mut App, mut request: Request) {
    let key = request.key;
    let first = request.sources[0].0;
    let world = &app.worlds[key.0];
    let dimension = &world.dimensions[key.1].name;
    let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
        world,
        dimension,
        first[0].div_euclid(16),
        first[2].div_euclid(16),
    )
    .unwrap()
    .unwrap();
    app.edit_filter = mcedit2_world::editing::render_state(&request.sources[0].1);
    app.block_edit.stroke.rules = vec![Rule {
        sources: app.edit_filter.clone(),
        target: app.edit_target.clone(),
    }];
    if request.config.mode == Mode::Biome {
        let id = mcedit2_world::sections::decode_sections(&raw)
            .unwrap()
            .numeric_biome(
                first[0].rem_euclid(16) as u8,
                first[1],
                first[2].rem_euclid(16) as u8,
            )
            .unwrap();
        app.block_edit.biome_target = ((id + 1) % 256).to_string();
    }
    request.config = config(app).unwrap();
    let samples = request.sources.clone();
    let routed = crate::quad_view::scene_message(Message::Brush(Action::Finished(Ok(request))));
    let task = crate::update(app, routed);
    assert!(app.block_edit.draft.is_none());
    assert!(!app.sessions[&key].has_pending_changes());
    drain(app, task);
    assert!(app.block_edit.draft.is_some(), "{}", app.edit_report);
    let preview = app.block_edit.draft.as_ref().unwrap();
    assert!(preview.session.has_pending_changes());
    let task = crate::update(app, Message::ApplyEdit);
    drain(app, task);
    assert!(
        app.sessions[&key].has_pending_changes(),
        "{}",
        app.edit_report
    );
    let task = crate::update(app, Message::UndoEdit);
    drain(app, task);
    assert!(!app.sessions[&key].has_pending_changes());
    let task = crate::update(app, Message::RedoEdit);
    drain(app, task);
    assert!(app.sessions[&key].has_pending_changes());
    for (p, _) in samples {
        assert!(matches!(
            app.sessions[&key]
                .visible_block((p[0], p[1], p[2]))
                .unwrap(),
            mcedit2_world::block_lookup::Lookup::Present(_)
        ));
    }
    let world = &app.worlds[key.0];
    assert_eq!(
        mcedit2_world::block_lookup::read_world_chunk_bytes(
            world,
            &world.dimensions[key.1].name,
            first[0].div_euclid(16),
            first[2].div_euclid(16)
        )
        .unwrap()
        .unwrap(),
        raw
    );
}

pub(super) fn projected_view_pipeline() {
    for mode in [Mode::Fill, Mode::Replace, Mode::Biome] {
        for kind in 0..5 {
            let (mut app, surface) = setup(mode);
            let key = app.current_key().unwrap();
            let p = &surface.voxels[0];
            let (x, y, z) = (p.x, p.y, p.z);
            let request = match kind {
                0 => {
                    let view = iso(&app, surface, Target::Isometric);
                    let size = Size::new(300., 300.);
                    let (a, b) = iso_points(&view, size);
                    let bounds = Rectangle {
                        x: 20.,
                        y: 500.,
                        width: 300.,
                        height: 300.,
                    };
                    let a = Point::new(a.x + bounds.x, a.y + bounds.y);
                    let b = Point::new(b.x + bounds.x, b.y + bounds.y);
                    cancellations(&view, a, bounds);
                    navigation(&view, a, bounds);
                    paint(&view, a, b, bounds)
                }
                1 => {
                    let scale = (320. / surface.width as f32).min(320. / surface.depth as f32);
                    let origin = Point::new(
                        (320. - surface.width as f32 * scale) * 0.5,
                        (320. - surface.depth as f32 * scale) * 0.5,
                    );
                    let at = Point::new(
                        origin.x + (x - surface.x) as f32 * scale + scale * 0.5,
                        origin.y + (z - surface.z) as f32 * scale + scale * 0.5,
                    );
                    let view = crate::top_down::TopDown {
                        selection_handle: None,
                        chunk_column: None,
                        brush: config(&app),
                        brush_key: Some(key),
                        layers: None,
                        source: surface,
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
                    let bounds = Rectangle {
                        x: 20.,
                        y: 500.,
                        width: 320.,
                        height: 320.,
                    };
                    let start = Point::new(bounds.x + at.x, bounds.y + at.y);
                    let end = Point::new(start.x + scale, start.y);
                    cancellations(&view, start, bounds);
                    navigation(&view, start, bounds);
                    paint(&view, start, end, bounds)
                }
                2 => {
                    let plane = Arc::new(
                        app.current_session(key)
                            .unwrap()
                            .inspect_slice(Bounds::new((x, y, z), (x + 1, y, z)).unwrap())
                            .unwrap(),
                    );
                    app.slice_grid = Some((key.0, key.1, Arc::clone(&plane)));
                    map_request(&app, Source::Horizontal(plane, Target::Horizontal))
                }
                3 | 4 => {
                    let end = if kind == 3 {
                        (x + 1, y, z)
                    } else {
                        (x, y, z + 1)
                    };
                    let plane = Arc::new(
                        app.current_session(key)
                            .unwrap()
                            .inspect_vertical_slice(Bounds::new((x, y, z), end).unwrap())
                            .unwrap(),
                    );
                    app.vertical_grid = Some((key.0, key.1, Arc::clone(&plane)));
                    map_request(&app, Source::Vertical(plane, Target::Vertical))
                }
                _ => unreachable!(),
            };
            finish(&mut app, request);
        }
        for pane in [
            QuadPane::Horizontal,
            QuadPane::Front,
            QuadPane::Side,
            QuadPane::Isometric,
        ] {
            let (mut app, surface) = setup(mode);
            let p = &surface.voxels[0];
            let _ = crate::quad_view::update(
                &mut app,
                crate::quad_view::Action::Center(format!("{},{},{}", p.x, p.y, p.z)),
            );
            let task = crate::quad_view::update(&mut app, crate::quad_view::Action::Load);
            drain(&mut app, task);
            let source = app.quad.brush_source(pane).unwrap();
            let request = match source {
                Source::Surface(ref view, target) => {
                    let view = iso(&app, Arc::clone(view), target);
                    let size = Size::new(300., 300.);
                    let (a, b) = iso_points(&view, size);
                    let bounds = Rectangle {
                        x: 20.,
                        y: 500.,
                        width: 300.,
                        height: 300.,
                    };
                    paint(
                        &view,
                        Point::new(a.x + bounds.x, a.y + bounds.y),
                        Point::new(b.x + bounds.x, b.y + bounds.y),
                        bounds,
                    )
                }
                _ => map_request(&app, source),
            };
            finish(&mut app, request);
        }
    }
}

pub(super) fn refresh_races() {
    let (mut app, surface) = setup(Mode::Fill);
    let key = app.current_key().unwrap();
    let p = &surface.voxels[0];
    let bounds = Bounds::new((p.x, p.y, p.z), (p.x + 1, p.y, p.z)).unwrap();
    let plane = Arc::new(
        app.current_session(key)
            .unwrap()
            .inspect_slice(bounds)
            .unwrap(),
    );
    app.slice_grid = Some((key.0, key.1, Arc::clone(&plane)));
    let request = map_request(&app, Source::Horizontal(plane, Target::Horizontal));
    let _ = update(&mut app, Action::Finished(Ok(request)));
    let message = edit_task_output(crate::block_edit::maintain(&mut app));
    let task = crate::update(&mut app, message);
    let old = outputs(task);
    assert!(app.slice_grid.is_none());
    let task = crate::update(&mut app, Message::ApplyEdit);
    drain(&mut app, task);
    assert!(app.slice_grid.is_some());
    for message in old {
        let task = crate::update(&mut app, message);
        drain(&mut app, task);
    }
    let SliceCell::Present { state, .. } = &app.slice_grid.as_ref().unwrap().2.rows[0][0] else {
        panic!("切片源不能消失");
    };
    assert_eq!(state.name, "numeric:41");
    let task = crate::update(&mut app, Message::UndoEdit);
    drain(&mut app, task);
    assert!(app.slice_grid.is_some());
    let old_source = Source::Horizontal(
        Arc::clone(&app.slice_grid.as_ref().unwrap().2),
        Target::Horizontal,
    );
    let old_plane = app.slice_grid.as_ref().unwrap().2.as_ref().clone();
    app.slice_grid = Some((key.0, key.1, Arc::new(old_plane)));
    assert!(old_source.verify(&app, key).is_err());

    // 四视图尚未发布候选源时，紧接着暂存仍须重读同一已应用中心。
    let (mut app, surface) = setup(Mode::Fill);
    let p = &surface.voxels[0];
    let _ = crate::quad_view::update(
        &mut app,
        crate::quad_view::Action::Center(format!("{},{},{}", p.x, p.y, p.z)),
    );
    let task = crate::quad_view::update(&mut app, crate::quad_view::Action::Load);
    drain(&mut app, task);
    let source = app.quad.brush_source(QuadPane::Horizontal).unwrap();
    let request = map_request(&app, source);
    let first = request.sources[0].0;
    let _ = update(&mut app, Action::Finished(Ok(request)));
    let message = edit_task_output(crate::block_edit::maintain(&mut app));
    let task = crate::update(&mut app, message);
    let old = outputs(task);
    assert!(app.quad.brush_source(QuadPane::Horizontal).is_none());
    let _ = crate::quad_view::update(
        &mut app,
        crate::quad_view::Action::Center("1000,100,1000".into()),
    );
    let task = crate::update(&mut app, Message::ApplyEdit);
    drain(&mut app, task);
    for message in old {
        let task = crate::update(&mut app, message);
        drain(&mut app, task);
    }
    let Source::Horizontal(plane, _) = app.quad.brush_source(QuadPane::Horizontal).unwrap() else {
        panic!("应重新发布实际水平源");
    };
    assert!(first[0] >= plane.bounds.minimum[0] && first[0] <= plane.bounds.maximum[0]);
    let row = (first[2] - plane.bounds.minimum[2]) as usize;
    let col = (first[0] - plane.bounds.minimum[0]) as usize;
    let SliceCell::Present { state, .. } = &plane.rows[row][col] else {
        panic!("四视图源不能消失");
    };
    assert_eq!(state.name, "numeric:41");
}
