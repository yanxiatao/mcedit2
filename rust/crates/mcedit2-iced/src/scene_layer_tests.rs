//! 完整位置图层流程并入已有投影 / 后台类别，不注册细项入口。
use super::*;
use crate::{
    orthographic_framing::{Bounds as FloatBounds, Projection},
    picking::Ray,
    spectator::Camera,
    world_camera::WorldCamera,
};
use iced::{
    mouse,
    widget::canvas::{self, Program},
    Point, Rectangle, Size,
};

fn marker(kind: Kind, position: [f64; 3], index: usize) -> Marker {
    Marker {
        kind,
        position,
        id: Some(format!("sample:actor_{index}")),
        path: format!("Entities/{index}"),
        source: "sample-nbt".into(),
        chunk: (0, 0),
        sha256: format!("{index:064x}"),
        fields: 2,
        bytes: 64,
        tick: None,
        priority: None,
        visual: None,
        block_state: None,
        visual_error: None,
        block_visual: None,
        geometry: None,
    }
}
fn part() -> Arc<ChunkAnnotations> {
    Arc::new(ChunkAnnotations {
        lighting: Default::default(),
        markers: vec![
            marker(Kind::Entity, [0.5, 0., 3.5], 0),
            marker(Kind::Item, [0.5, 0., 6.5], 1),
            marker(Kind::BlockEntity, [0., 0., 9.], 2),
            marker(Kind::Entity, [0.5, 0., -3.5], 3),
        ],
        diagnostics: Vec::new(),
    })
}
fn camera() -> Camera {
    Camera {
        position: [0.; 3],
        yaw: 0.,
        pitch: 0.,
        zoom: 1.,
        perspective: true,
    }
}
fn geometry_and_read_only_marker_interaction() {
    let size = Size::new(200., 200.);
    let center = Point::new(100., 100.);
    let mut source = part();
    // 大批屏幕外实体同样保留，以空间索引裁剪而非截断实体总数。
    let original = Arc::get_mut(&mut source).unwrap();
    for i in 4..4100 {
        original
            .markers
            .push(marker(Kind::Entity, [3000. + i as f64, 0., 40.], i));
    }
    let snapshot = Arc::new(Snapshot::build(
        (0, 0),
        7,
        std::iter::once(source.clone()),
        None,
    ));
    assert_eq!(snapshot.markers.len(), 4100);
    let perspective = WorldCamera::from(camera());
    let ray = Ray::new(perspective, [0; 3], center, size).unwrap();
    assert_eq!(
        snapshot.tree.pick(&snapshot.markers, ray, u16::MAX),
        Some(0)
    );
    assert_eq!(
        snapshot.tree.pick(&snapshot.markers, ray, Kind::Item.bit()),
        Some(1)
    );
    assert_eq!(
        snapshot
            .tree
            .pick(&snapshot.markers, ray, Kind::BlockEntity.bit()),
        Some(2)
    );
    let visible = snapshot.tree.visible(perspective, size, u16::MAX);
    assert!(visible.contains(&0));
    assert!(!visible.contains(&4099));
    let clipped = Snapshot::build(
        (0, 0),
        8,
        std::iter::once(source),
        Some(Bounds {
            minimum: [-1, -1, 2],
            maximum: [1, 1, 4],
        }),
    );
    assert_eq!(clipped.markers.len(), 1);
    for (yaw, pitch) in [(0_f32, 35_f32), (45., 70.), (132., 20.)] {
        let projection = Projection::fit(
            FloatBounds {
                minimum: [-1., -1., 1.],
                maximum: [1., 1., 5.],
            },
            (yaw.to_radians(), pitch.to_radians()),
            1.,
            [0.; 2],
            size,
        )
        .unwrap();
        let point = projection.project([0., 0., 3.]).0;
        let ray = Ray::new(projection, [0; 3], point, size).unwrap();
        assert_eq!(
            clipped.tree.pick(&clipped.markers, ray, Kind::Entity.bit()),
            Some(0)
        );
        let (a, b) =
            overlay::segment(projection.into(), size, [-0.5, -0.5, 2.5], [0.5, 0.5, 3.5]).unwrap();
        assert!([a.x, a.y, b.x, b.y].into_iter().all(f32::is_finite));
    }
    assert!(overlay::segment(perspective, size, [-1., 0., -2.], [1., 0., -1.]).is_none());
    let (a, b) = overlay::segment(perspective, size, [-1., 0., -2.], [1., 0., 3.]).unwrap();
    assert!([a.x, b.x].into_iter().all(|v| (0. ..=200.).contains(&v)));
    let far = Camera {
        position: [29_999_980., 64., -29_999_980.],
        ..camera()
    };
    let shifted = Arc::new(ChunkAnnotations {
        lighting: Default::default(),
        markers: vec![marker(Kind::Entity, [29_999_980.5, 64., -29_999_976.5], 9)],
        diagnostics: Vec::new(),
    });
    let shifted = Snapshot::build((0, 0), 9, std::iter::once(shifted), None);
    assert_eq!(
        shifted.tree.pick(
            &shifted.markers,
            Ray::new(far, [0; 3], center, size).unwrap(),
            u16::MAX
        ),
        Some(0)
    );
    let mut state = overlay::OverlayState::default();
    let mut view = Overlay {
        snapshot: snapshot.clone(),
        projection: ProjectionSource::Free(camera()),
        mask: u16::MAX,
        inspect: true,
        selected: None,
    };
    let bounds = Rectangle {
        x: 27.,
        y: 19.,
        width: 200.,
        height: 200.,
    };
    let cursor = mouse::Cursor::Available(Point::new(127., 119.));
    let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
    let release = canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
    assert!(view.update(&mut state, &press, bounds, cursor).is_some());
    let action = view.update(&mut state, &release, bounds, cursor).unwrap();
    let (message, _, _) = action.into_inner();
    assert!(matches!(message,Some(Message::SceneMarkerSelected(s,0)) if Arc::ptr_eq(&s,&snapshot)));
    // 按下以后换成新快照，释放不能把旧索引解释成新实体。
    view.update(&mut state, &press, bounds, cursor).unwrap();
    view.snapshot = Arc::new(Snapshot::build((0, 0), 10, std::iter::once(part()), None));
    assert!(view
        .update(&mut state, &release, bounds, cursor)
        .unwrap()
        .into_inner()
        .0
        .is_none());
    view.inspect = false;
    assert!(view.update(&mut state, &press, bounds, cursor).is_none());
    // 限定等轴与普通世界等轴都使用实际构图；空地形世界由真实位置数据构图。
    let fixture = crate::textures::java_obj_tests::fixture();
    let bounded = crate::textures::java_obj_tests::isometric_scene(fixture.open());
    let expected = bounded
        .prepare_projection(size, &|| false)
        .unwrap()
        .unwrap();
    let bounded_snapshot = Snapshot::build(
        (0, 0),
        11,
        std::iter::once(Arc::new(ChunkAnnotations {
            lighting: Default::default(),
            markers: vec![marker(Kind::BlockEntity, [0.; 3], 1)],
            diagnostics: Vec::new(),
        })),
        None,
    );
    let point = expected.project([0.; 3]).0;
    let bounded_camera = ProjectionSource::Bounded(bounded)
        .camera(size, &bounded_snapshot)
        .unwrap();
    assert_eq!(
        bounded_snapshot.tree.pick(
            &bounded_snapshot.markers,
            Ray::new(bounded_camera, [0; 3], point, size).unwrap(),
            u16::MAX
        ),
        Some(0)
    );
    let empty = Arc::new(crate::spectator::StreamMesh {
        terrain: None,
        visibility: None,
        entities: None,
        complete: true,
        chunks: Default::default(),
        origin: [0; 3],
        atlas: None,
    });
    let orthographic = crate::orthographic_world::View {
        angles: (0., 35_f32.to_radians()),
        navigation: Default::default(),
        framing: Default::default(),
    };
    let projected = ProjectionSource::World {
        presented: Arc::new(std::sync::Mutex::new(Some(empty))),
        frame: Default::default(),
        view: orthographic,
    };
    let camera = projected.camera(size, &bounded_snapshot).unwrap();
    assert_eq!(
        bounded_snapshot.tree.pick(
            &bounded_snapshot.markers,
            Ray::new(camera, [0; 3], center, size).unwrap(),
            u16::MAX
        ),
        Some(0)
    );
}
fn streamed_plan_publication_drives_source_layers() {
    use crate::{picking_integration_tests::edit_task_output, spectator_stream::Stream, Message};
    let (mut app, view, _, _) = crate::picking_integration_tests::fixture();
    let session = Arc::new(
        Session::new(
            app.worlds[0].clone(),
            app.worlds[0].dimensions[0].name.clone(),
        )
        .unwrap(),
    );
    app.surface_exposed = true;
    app.show_top_down = false;
    app.entity_models.enabled = false;
    app.scene_layers.mask = Kind::Entity.bit() | Kind::Item.bit() | Kind::BlockEntity.bit();
    let location = view.camera.position;
    app.spectator_stream = Some(Stream::new((0, 0), session, location, 1, 0., 7, false).unwrap());
    // 初始计划还在后台，不能发布零区块的完成快照。
    assert!(iced_runtime::task::into_stream(app.maintain_scene_layers()).is_none());
    assert!(app.scene_layers.scope.is_none());
    assert!(app.scene_layers.snapshot.is_none());
    let task = app
        .spectator_stream
        .as_mut()
        .unwrap()
        .replan(location, 1, 0., 7);
    let Message::SpectatorStreamPlan(nonce, id, result) = edit_task_output(task) else {
        panic!("需要实际后台区块计划")
    };
    let _ = app
        .spectator_stream
        .as_mut()
        .unwrap()
        .receive_plan(nonce, id, result);
    let Message::SceneLayersLoaded(epoch, key, result) =
        edit_task_output(app.maintain_scene_layers())
    else {
        panic!("已发布计划必须触发完整来源读取")
    };
    app.scene_layers.accept(epoch, key, result);
    assert_eq!(app.scene_layers.cache.len(), 9);
    assert!(app.scene_layers.errors.is_empty());
    let previous = app.scene_layers.snapshot.clone().unwrap();
    let before = app.scene_layers.cache.clone();
    // 移动范围时保留上一完整来源；只有实际新计划返回后才安装新代次和增量区块。
    let next = [location[0] + 32., location[1], location[2]];
    let task = app
        .spectator_stream
        .as_mut()
        .unwrap()
        .replan(next, 1, 0., 8);
    assert!(iced_runtime::task::into_stream(app.maintain_scene_layers()).is_none());
    assert!(Arc::ptr_eq(
        app.scene_layers.snapshot.as_ref().unwrap(),
        &previous
    ));
    let Message::SpectatorStreamPlan(nonce, id, result) = edit_task_output(task) else {
        panic!("需要移动后的后台区块计划")
    };
    let _ = app
        .spectator_stream
        .as_mut()
        .unwrap()
        .receive_plan(nonce, id, result);
    let task = app.maintain_scene_layers();
    assert_eq!(app.scene_layers.cache.len(), 3);
    for (coordinate, part) in &app.scene_layers.cache {
        assert!(Arc::ptr_eq(part, &before[coordinate]));
    }
    let Message::SceneLayersLoaded(epoch, key, result) = edit_task_output(task) else {
        panic!("新范围应读取其余实际来源")
    };
    app.scene_layers.accept(epoch, key, result);
    assert_eq!(app.scene_layers.cache.len(), 9);
    assert!(app.scene_layers.errors.is_empty());
    assert!(matches!(
        app.scene_layers.scope,
        Some(Scope::Stream { nonce: 8, .. })
    ));
    app.spectator_stream.as_ref().unwrap().cancel_readers();
}
fn background_cancel_reuse_select_and_source_read() {
    streamed_plan_publication_drives_source_layers();
    let (mut app, view, _, _) = crate::picking_integration_tests::fixture();
    let session = Session::new(
        app.worlds[0].clone(),
        app.worlds[0].dimensions[0].name.clone(),
    )
    .unwrap();
    let root = session.original_world().root.clone();
    let retained = part();
    let mut state = State {
        mask: u16::MAX,
        inspect: true,
        ..Default::default()
    };
    state.replan(
        Scope::Stream {
            key: (0, 0),
            root: root.clone(),
            nonce: 1,
        },
        vec![(0, 0)],
    );
    let old_epoch = state.epoch;
    let old_cancel = Arc::clone(&state.cancelled);
    let first = Snapshot::build((0, 0), old_epoch, std::iter::once(retained.clone()), None);
    let first = Arc::new(Batch {
        records: vec![((0, 0), Ok(retained.clone()))],
        snapshot: Arc::new(first),
        player_catalog: None,
        players: None,
    });
    state.accept(old_epoch, (0, 0), Ok(first));
    let first = state.snapshot.clone().unwrap();
    // 同一代次扩展到空区块时，源标记、索引及选中对象的身份应一起保留。
    let unchanged = Snapshot::build_cancellable(
        (0, 0),
        old_epoch,
        [Arc::clone(&retained), Arc::new(ChunkAnnotations::default())].into_iter(),
        None,
        Arc::default(),
        Some(&first),
        &|| false,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&unchanged, &first));
    let replacement = Arc::new(ChunkAnnotations {
        lighting: Default::default(),
        markers: retained.markers.clone(),
        diagnostics: Vec::new(),
    });
    let replaced = Snapshot::build_cancellable(
        (0, 0),
        old_epoch,
        [replacement].into_iter(),
        None,
        Arc::default(),
        Some(&first),
        &|| false,
    )
    .unwrap();
    assert!(!Arc::ptr_eq(&replaced, &first));
    assert!(replaced.markers[0].same(&first.markers[0]));
    assert!(state.select(&first, 0, Some((0, 0))));
    assert!(state
        .detail(Language::Chinese)
        .unwrap()
        .contains("sample:actor_0"));
    assert!(!state.select(&first, 0, Some((0, 1))));
    state.replan(
        Scope::Stream {
            key: (0, 0),
            root: root.clone(),
            nonce: 2,
        },
        vec![(0, 0), (1, 0)],
    );
    assert!(old_cancel.load(Ordering::Acquire));
    assert!(Arc::ptr_eq(&state.cache[&(0, 0)], &retained));
    assert_eq!(
        state.pending.iter().copied().collect::<Vec<_>>(),
        vec![(1, 0)]
    );
    assert!(!state.select(&first, 0, Some((0, 0))));
    state.accept(old_epoch, (0, 0), Err("旧工作应被忽略".into()));
    assert!(!state.failed);
    let epoch = state.epoch;
    let empty = Arc::new(ChunkAnnotations::default());
    let second = Arc::new(Snapshot::build(
        (0, 0),
        epoch,
        [retained.clone(), empty.clone()].into_iter(),
        None,
    ));
    state.accept(
        epoch,
        (0, 0),
        Ok(Arc::new(Batch {
            records: vec![((1, 0), Ok(Arc::clone(&empty)))],
            snapshot: second.clone(),
            player_catalog: None,
            players: None,
        })),
    );
    assert!(state.selected.is_some());
    assert!(state.select(&second, 0, Some((0, 0))));
    // 编辑会刷新相邻源段的边界；边界外的源段及模型仍沿用原身份。
    state.change_sources(&[(2, 0)].into_iter().collect());
    app.surface = None;
    app.scene_layers = state;
    let _ = app.maintain_scene_layers();
    let mut state = std::mem::take(&mut app.scene_layers);
    assert!(Arc::ptr_eq(&state.cache[&(0, 0)], &retained));
    assert!(!state.cache.contains_key(&(1, 0)));
    state.replan(
        Scope::Stream {
            key: (0, 0),
            root: root.clone(),
            nonce: 3,
        },
        vec![(0, 0), (1, 0)],
    );
    assert!(Arc::ptr_eq(&state.cache[&(0, 0)], &retained));
    assert_eq!(
        state.pending.iter().copied().collect::<Vec<_>>(),
        vec![(1, 0)]
    );
    let epoch = state.epoch;
    let second = Arc::new(Snapshot::build(
        (0, 0),
        epoch,
        [retained.clone(), empty.clone()].into_iter(),
        None,
    ));
    state.accept(
        epoch,
        (0, 0),
        Ok(Arc::new(Batch {
            records: vec![((1, 0), Ok(empty))],
            snapshot: Arc::clone(&second),
            player_catalog: None,
            players: None,
        })),
    );
    state.mask = Kind::Item.bit();
    assert!(!state.select(&second, 0, Some((0, 0))));
    state.accept(epoch, (0, 0), Err("受控读取故障".into()));
    assert!(state.failed);
    assert!(Arc::ptr_eq(state.snapshot.as_ref().unwrap(), &second));
    state.invalidate();
    assert!(!state.failed && state.snapshot.is_none() && state.selected.is_none());
    // 运行实际后台读取，保留所有原始实体元数据且不暂存编辑。
    let point = &view.surface.voxels[0];
    let coordinate = (point.x.div_euclid(16), point.z.div_euclid(16));
    let batch = Job {
        epoch: 3,
        key: (0, 0),
        clip: None,
        session: session.clone(),
        requests: vec![coordinate],
        retained: Vec::new(),
        previous: None,
        players: None,
        entities: true,
        // 本夹具比较请求区块记录与快照；玩家目录有独立来源，不混入该比较。
        players_enabled: false,
        block_visual_numeric_states: Default::default(),
        declarative_block_states: Default::default(),
        layers: Kind::SpawnNight.bit() | Kind::TerrainIncomplete.bit(),
        heightmap: Default::default(),
        cancelled: Arc::new(AtomicBool::new(false)),
    }
    .read()
    .unwrap();
    assert!(batch.records[0].1.is_ok());
    assert!(batch.records[0]
        .1
        .as_ref()
        .unwrap()
        .diagnostics
        .iter()
        .any(|text| text.contains("刷怪候选") && text.contains("缺少 DataVersion")));
    assert_eq!(
        batch.snapshot.markers.len(),
        batch.records[0].1.as_ref().unwrap().markers.len()
    );
    let run_retained = |retained, previous| {
        Job {
            epoch: 3,
            key: (0, 0),
            clip: None,
            session: session.clone(),
            requests: Vec::new(),
            retained,
            previous,
            players: None,
            entities: true,
            players_enabled: false,
            block_visual_numeric_states: Default::default(),
            declarative_block_states: Default::default(),
            layers: 0,
            heightmap: Default::default(),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
        .read()
        .unwrap()
    };
    let a = part();
    let b = part();
    let ordered = run_retained(
        vec![((2, 0), Arc::clone(&b)), ((1, 0), Arc::clone(&a))],
        None,
    );
    let repeated = run_retained(
        vec![
            ((1, 0), a),
            ((2, 0), b),
            ((0, 0), Arc::new(ChunkAnnotations::default())),
        ],
        Some(Arc::clone(&ordered.snapshot)),
    );
    assert!(Arc::ptr_eq(&ordered.snapshot, &repeated.snapshot));
    assert!(!session.has_pending_changes());
    let cancelled = Job {
        epoch: 4,
        key: (0, 0),
        clip: None,
        session,
        requests: vec![coordinate],
        retained: Vec::new(),
        previous: None,
        players: None,
        entities: true,
        players_enabled: true,
        block_visual_numeric_states: Default::default(),
        declarative_block_states: Default::default(),
        layers: 0,
        heightmap: Default::default(),
        cancelled: Arc::new(AtomicBool::new(true)),
    };
    assert!(cancelled.read().is_err());
}
fn installed_scene_annotations_from_actual_world() {
    let path = std::env::var_os("MCEDIT2_TEST_WORLD").expect("实例验收需要 MCEDIT2_TEST_WORLD");
    let world = mcedit2_world::inspect_world(path).unwrap();
    let dimension = world
        .dimensions
        .iter()
        .find(|d| d.name == "minecraft:overworld")
        .expect("实例应包含主世界")
        .name
        .clone();
    let session = Session::new(world.clone(), dimension).unwrap();
    let requests = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| (x, z)))
        .collect();
    let started = std::time::Instant::now();
    let batch = Job {
        epoch: 1,
        key: (0, 0),
        clip: None,
        session: session.clone(),
        requests,
        retained: Vec::new(),
        previous: None,
        players: None,
        entities: true,
        players_enabled: true,
        block_visual_numeric_states: Default::default(),
        declarative_block_states: Default::default(),
        layers: 0,
        heightmap: Default::default(),
        cancelled: Arc::new(AtomicBool::new(false)),
    }
    .read()
    .unwrap();
    for (coordinate, result) in &batch.records {
        assert!(
            result.is_ok(),
            "真实区块 {coordinate:?} 位置数据读取失败：{result:?}"
        );
    }
    let count = |kind| {
        batch
            .snapshot
            .markers
            .iter()
            .filter(|m| m.marker().kind == kind)
            .count()
    };
    assert!(count(Kind::BlockEntity) > 0, "真实建筑范围应有方块实体");
    assert!(!session.has_pending_changes());
    let report = serde_json::json!({"world":world.root,"data_version":world.data_version,"chunks":batch.records.len(),
        "entities":count(Kind::Entity),"items_and_hanging":count(Kind::Item),"block_entities":count(Kind::BlockEntity),
        "block_ticks":count(Kind::BlockTick),"fluid_ticks":count(Kind::FluidTick),
        "diagnostics":batch.records.iter().map(|(_,r)|r.as_ref().unwrap().diagnostics.len()).sum::<usize>(),
        "seconds":started.elapsed().as_secs_f64()});
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/scene-layer-instance-profile.json");
    std::fs::write(target, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    eprintln!("真实存档位置图层：{report}");
}
fn saved_terrain_layers_all_projections() {
    use crate::projected_entities::Mapping;
    use mcedit2_world::scene_annotations::Geometry;
    let size = Size::new(200., 200.);
    let kinds = [Kind::SpawnDark, Kind::SpawnNight, Kind::TerrainIncomplete];
    let source = Arc::new(ChunkAnnotations {
        lighting: Default::default(),
        markers: kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| {
                let mut m = marker(kind, [0.; 3], i);
                let geometry = if kind == Kind::TerrainIncomplete {
                    Geometry::Box {
                        minimum: [-8., -64., -8.],
                        maximum: [8., 320., 8.],
                    }
                } else {
                    Geometry::Horizontal {
                        minimum: [-2., -2.],
                        maximum: [2., 2.],
                        y: 10.5 + i as f64 * 2.,
                    }
                };
                let (min, max) = geometry.bounds();
                m.position = std::array::from_fn(|a| (min[a] + max[a]) * 0.5);
                m.geometry = Some(geometry);
                m
            })
            .collect(),
        diagnostics: Vec::new(),
    });
    let snapshot = Arc::new(Snapshot::build((0, 0), 30, std::iter::once(source), None));
    for (index, kind) in kinds.into_iter().enumerate() {
        let point = snapshot.markers[index].marker().center();
        let camera = WorldCamera::from(Camera {
            position: [point[0], point[1] + 20., point[2] - 20.],
            pitch: 45.,
            ..camera()
        });
        assert_eq!(
            snapshot.tree.pick(
                &snapshot.markers,
                Ray::new(camera, [0; 3], Point::new(100., 100.), size).unwrap(),
                kind.bit()
            ),
            Some(index)
        );
        let view = View {
            snapshot: Arc::clone(&snapshot),
            mask: kind.bit(),
            inspect: true,
            selected: None,
        };
        let state = planar::State::default();
        let mut mappings = Vec::new();
        for (right, down, depth) in [
            ([1., 0., 0.], [0., 0., 1.], [0., -1., 0.]),
            ([1., 0., 0.], [0., -1., 0.], [0., 0., 1.]),
            ([0., 0., 1.], [0., -1., 0.], [1., 0., 0.]),
        ] {
            mappings.push(Mapping::Orthogonal {
                origin: point,
                screen: [100., 100.],
                right,
                down,
                depth,
                scale: 4.,
            });
        }
        for (yaw, pitch) in [(0_f32, 35_f32), (45., 70.), (132., 20.)] {
            let projection = Projection::fit(
                FloatBounds {
                    minimum: [-8., -64., -8.],
                    maximum: [8., 320., 8.],
                },
                (yaw.to_radians(), pitch.to_radians()),
                1.,
                [0.; 2],
                size,
            )
            .unwrap();
            let ray = Ray::new(projection, [0; 3], projection.project(point).0, size).unwrap();
            assert_eq!(
                snapshot.tree.pick(&snapshot.markers, ray, kind.bit()),
                Some(index)
            );
            mappings.push(Mapping::Isometric(projection));
        }
        for mapping in mappings {
            let layout = PlanarLayout {
                mapping,
                size,
                clip: ([-32., -128., -32.], [32., 512., 32.]),
            };
            let at = mapping.project(point).0;
            assert_eq!(view.hit(&state, layout, at), Some(index));
            let bounds = Rectangle {
                x: 0.,
                y: 0.,
                width: size.width,
                height: size.height,
            };
            let action = view
                .update(
                    &state,
                    layout,
                    &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    bounds,
                    mouse::Cursor::Available(at),
                )
                .unwrap();
            assert!(
                matches!(action.into_inner().0, Some(Message::SceneMarkerSelected(ref s, i))
                if Arc::ptr_eq(s, &snapshot) && i == index)
            );
            if kind != Kind::TerrainIncomplete {
                let clipped = PlanarLayout {
                    clip: ([-32., 20., -32.], [32., 32., 32.]),
                    ..layout
                };
                assert_eq!(view.hit(&state, clipped, at), None);
            }
        }
    }
    let none = View {
        snapshot,
        mask: 0,
        inspect: true,
        selected: None,
    };
    assert_eq!(
        none.hit(
            &planar::State::default(),
            PlanarLayout {
                mapping: Mapping::Orthogonal {
                    origin: [0.; 3],
                    screen: [100.; 2],
                    right: [1., 0., 0.],
                    down: [0., -1., 0.],
                    depth: [0., 0., 1.],
                    scale: 4.
                },
                size,
                clip: ([-32., -128., -32.], [32., 512., 32.]),
            },
            Point::new(100., 100.)
        ),
        None
    );
}
fn continuous_saved_terrain_boundary() {
    use crate::metadata_fixture::{define_overworld, Fixture};
    use mcedit2_world::{
        chunk_creation::{empty_chunk, BiomeSource, ChunkCreationProfile},
        chunk_display::{Heightmap, Request},
        nbt_document::{Document, Operation, Value},
        scene_annotations::Reader,
    };
    let area = |faces: Vec<boundary::Face>| {
        faces
            .into_iter()
            .map(|face| {
                let p = face.points();
                let length = |a: [f64; 3], b: [f64; 3]| {
                    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
                };
                length(p[0], p[1]) * length(p[1], p[2])
            })
            .sum::<f64>()
    };
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        let legacy = version == 1343;
        if !legacy {
            define_overworld(&mut fixture);
        }
        let profile = ChunkCreationProfile {
            data_version: version,
            min_y: if legacy { 0 } else { -64 },
            height: if legacy { 256 } else { 384 },
            has_skylight: true,
            dimension_source: "user:连续诊断体积".into(),
            initial_biome: if legacy {
                "numeric:1"
            } else {
                "minecraft:plains"
            }
            .into(),
            biome_source: if legacy {
                BiomeSource::LegacyNumeric {
                    origin: "原版数字群系".into(),
                    id: 1,
                }
            } else {
                BiomeSource::UserSpecified {
                    origin: "验证".into(),
                    entries: ["minecraft:plains".into()].into_iter().collect(),
                }
            },
        };
        let coordinates = [(0, 0), (1, 0), (0, 1), (1, 1)];
        let mut chunks = coordinates
            .into_iter()
            .map(|coordinate| {
                let mut doc = Document::new(empty_chunk(&profile, coordinate).unwrap()).unwrap();
                doc.apply(&Operation::Replace {
                    path: if legacy {
                        "/Level/TerrainPopulated"
                    } else {
                        "/Status"
                    }
                    .into(),
                    value: if legacy {
                        Value::Byte(0)
                    } else {
                        Value::String("minecraft:features".into())
                    },
                })
                .unwrap();
                if legacy {
                    doc.apply(&Operation::Replace {
                        path: "/Level/HeightMap".into(),
                        value: Value::IntArray(vec![32 + coordinate.0 * 16; 256]),
                    })
                    .unwrap();
                }
                (coordinate, doc.bytes().to_vec())
            })
            .collect::<Vec<_>>();
        let path = fixture.root.join("region/r.0.0.mca");
        crate::object_fixture::write_region(&path, &chunks);
        fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let saved = std::fs::read(&path).unwrap();
        let read = |layers| {
            let mut reader = Reader::new(&fixture.world, "minecraft:overworld").unwrap();
            coordinates
                .into_iter()
                .map(|chunk| {
                    let part = reader
                        .read_with_layers_filtered(
                            Request {
                                chunk,
                                layers,
                                heightmap: Heightmap::Source,
                            },
                            None,
                            None,
                            false,
                        )
                        .unwrap();
                    assert!(part.diagnostics.is_empty(), "{:?}", part.diagnostics);
                    Arc::new(part)
                })
                .collect::<Vec<_>>()
        };
        let parts = read(Kind::TerrainIncomplete.bit());
        let snapshot = Arc::new(Snapshot::build((0, 0), 42, parts.clone().into_iter(), None));
        assert_eq!(snapshot.markers.len(), 4);
        assert_eq!(
            (0..4)
                .map(|i| snapshot.surfaces(i, None).len())
                .sum::<usize>(),
            16
        );
        assert_eq!(
            (0..4)
                .map(|i| area(snapshot.surfaces(i, None)))
                .sum::<f64>(),
            2. * (32. * 32. + 64. * f64::from(profile.height))
        );
        // 未读取的邻块没有证明，保留外面；重新读到的邻块才消去公共面。
        let isolated = Snapshot::build((0, 0), 42, parts[..1].iter().cloned(), None);
        assert_eq!(isolated.surfaces(0, None).len(), 6);
        let min_y = f64::from(profile.min_y) - 0.5;
        let max_y = min_y + f64::from(profile.height);
        let clipped = ([-0.5, min_y, -0.5], [15.5, max_y, 31.5]);
        assert!(snapshot.surfaces(1, Some(clipped)).is_empty());
        assert_eq!(
            (0..4)
                .map(|i| area(snapshot.surfaces(i, Some(clipped))))
                .sum::<f64>(),
            2. * (16. * 32. + 48. * f64::from(profile.height))
        );
        let slice = ([15.5, min_y, -0.5], [15.5, max_y, 31.5]);
        assert!(snapshot.surfaces(0, Some(slice)).is_empty());
        assert_eq!(
            (0..4)
                .map(|i| area(snapshot.surfaces(i, Some(slice))))
                .sum::<f64>(),
            32. * f64::from(profile.height)
        );
        planar::verify_saved_surface_projection(&snapshot);
        if legacy {
            let heightmap = Arc::new(Snapshot::build(
                (0, 0),
                43,
                read(Kind::Heightmap.bit()).into_iter(),
                None,
            ));
            planar::verify_saved_surface_projection(&heightmap);
        }
        let calls = std::sync::atomic::AtomicUsize::new(0);
        Snapshot::build_cancellable(
            (0, 0),
            42,
            parts.clone().into_iter(),
            None,
            Arc::default(),
            None,
            &|| {
                calls.fetch_add(1, Ordering::Relaxed);
                false
            },
        )
        .unwrap();
        let limit = calls.load(Ordering::Relaxed) - 2;
        calls.store(0, Ordering::Relaxed);
        assert!(Snapshot::build_cancellable(
            (0, 0),
            42,
            parts.clone().into_iter(),
            None,
            Arc::default(),
            None,
            &|| calls.fetch_add(1, Ordering::Relaxed) >= limit
        )
        .is_none());
        let reused = Snapshot::build_cancellable(
            (0, 0),
            42,
            parts.into_iter(),
            None,
            Arc::default(),
            Some(&snapshot),
            &|| false,
        )
        .unwrap();
        assert!(Arc::ptr_eq(&snapshot, &reused));
        assert_eq!(std::fs::read(&path).unwrap(), saved);
        // 实际邻块完成生成后，新来源恢复外面，旧不可变快照不改变。
        let mut completed = Document::new(chunks[1].1.clone()).unwrap();
        completed
            .apply(&Operation::Replace {
                path: if legacy {
                    "/Level/TerrainPopulated"
                } else {
                    "/Status"
                }
                .into(),
                value: if legacy {
                    Value::Byte(1)
                } else {
                    Value::String("minecraft:full".into())
                },
            })
            .unwrap();
        chunks[1].1 = completed.bytes().to_vec();
        crate::object_fixture::write_region(&path, &chunks);
        let changed = Snapshot::build(
            (0, 0),
            44,
            read(Kind::TerrainIncomplete.bit()).into_iter(),
            None,
        );
        assert_eq!(changed.markers.len(), 3);
        assert_eq!(
            (0..3)
                .map(|i| changed.surfaces(i, None).len())
                .sum::<usize>(),
            14
        );
        assert_eq!(snapshot.surfaces(0, None).len(), 4);
    }
}

pub(super) fn run_test_category(category: crate::test_categories::Category) {
    match category {
        crate::test_categories::Category::Rendering => {
            geometry_and_read_only_marker_interaction();
            saved_terrain_layers_all_projections();
            continuous_saved_terrain_boundary();
        }
        crate::test_categories::Category::Background => {
            background_cancel_reuse_select_and_source_read()
        }
        crate::test_categories::Category::Installed => {
            installed_scene_annotations_from_actual_world()
        }
        _ => (),
    }
}

/// 通过正式后台批次读取不可变动态模型与群系来源；供已有特殊模型类别共用。
pub(crate) fn model_snapshot(
    session: Session,
    bank: &crate::textures::TextureBank,
    previous: Option<Arc<Snapshot>>,
    clip: Option<Bounds>,
) -> Arc<Snapshot> {
    let retained = previous.as_ref().map_or_else(Vec::new, |snapshot| {
        snapshot
            .markers
            .first()
            .map(|source| vec![(source.marker().chunk, Arc::clone(&source.chunk))])
            .unwrap_or_default()
    });
    Job {
        key: (0, 0),
        epoch: 1,
        clip,
        session,
        requests: if previous.is_some() {
            vec![]
        } else {
            vec![(-2, -2)]
        },
        retained,
        previous,
        players: None,
        entities: false,
        players_enabled: false,
        block_visual_numeric_states: Arc::new(bank.block_visual_numeric_states()),
        declarative_block_states: Arc::new(BTreeSet::new()),
        cancelled: Arc::new(AtomicBool::new(false)),
        layers: Kind::BlockEntity.bit(),
        heightmap: mcedit2_world::chunk_display::Heightmap::Source,
    }
    .read()
    .unwrap()
    .snapshot
}
