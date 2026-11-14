//! 真实工作线程与 Shader 事件联动；无需鼠标或空转应用消息来显示完成的网格。
use super::*;
use iced::widget::shader::Program;

fn redraw(scene: &GpuScene, size: Size) -> Option<shader::Action<Message>> {
    scene.update(
        &mut State::default(),
        &iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        )),
        Rectangle::with_size(size),
        mouse::Cursor::Unavailable,
    )
}

#[test]
fn shader_requests_redraw_while_isometric_worker_runs_then_stops_when_complete() {
    let fixture = crate::textures::java_obj_tests::fixture();
    let mut view = crate::textures::java_obj_tests::isometric_scene(fixture.open());
    view.gpu_faces = true;
    let size = Size::new(64., 64.);
    // 确定性地模拟已在队列中的工作；事件不能因缓存尚未发布而结束刷新。
    *view.gpu_pending.lock().unwrap() = Some(size);
    let scene = GpuScene(SceneSource::Isometric(view.clone()));
    assert!(
        redraw(&scene, size).is_some(),
        "投影网格尚未就绪时必须安排后续帧"
    );
    *view.gpu_pending.lock().unwrap() = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let action = redraw(&scene, size);
        let ready = view.gpu_frame_cache.lock().unwrap().clone();
        if let Some((cached, vertices)) = ready {
            assert_eq!(cached, size);
            assert!(!vertices.is_empty());
            assert!(
                redraw(&scene, size).is_none(),
                "已完成的静态画面不应持续重绘"
            );
            assert!(!scene
                .draw(
                    &State::default(),
                    mouse::Cursor::Unavailable,
                    Rectangle::with_size(size)
                )
                .vertices
                .is_empty());
            break;
        }
        assert!(action.is_some());
        assert!(std::time::Instant::now() < deadline, "投影工作线程未完成");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn shader_requests_redraw_until_perspective_source_mesh_is_presentable() {
    let fixture = crate::textures::java_obj_tests::fixture();
    let view = crate::textures::java_obj_tests::scene(fixture.open());
    let size = Size::new(694., 654.);
    *view.pending.lock().unwrap() = true;
    let scene = GpuScene(SceneSource::Spectator(view.clone()));
    assert!(
        redraw(&scene, size).is_some(),
        "后台透视网格完成前不能只留下天空"
    );
    *view.pending.lock().unwrap() = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let action = redraw(&scene, size);
        let ready = view.cache.lock().unwrap().clone();
        if let Some(mesh) = ready {
            assert!(mesh.complete);
            assert!(mesh.chunks.iter().any(|c| !c.quads().is_empty()));
            assert!(redraw(&scene, size).is_some());
            let primitive = scene.draw(
                &State::default(),
                mouse::Cursor::Unavailable,
                Rectangle::with_size(size),
            );
            assert!(Arc::ptr_eq(primitive.stream_mesh.as_ref().unwrap(), &mesh));
            // 真实 prepare 提交由 Hardware 类别验证；CPU 就绪仍须安排上传帧。
            *view.presented.lock().unwrap() = Some(mesh);
            let mut state = State::default();
            let event = iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            ));
            let notified = scene
                .update(
                    &mut state,
                    &event,
                    Rectangle::with_size(size),
                    mouse::Cursor::Unavailable,
                )
                .unwrap()
                .into_inner()
                .0;
            assert!(matches!(notified, Some(Message::WorldMeshPresented)));
            assert!(scene
                .update(
                    &mut state,
                    &event,
                    Rectangle::with_size(size),
                    mouse::Cursor::Unavailable
                )
                .is_none());
            break;
        }
        assert!(action.is_some());
        assert!(std::time::Instant::now() < deadline, "透视工作线程未完成");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn incomplete_stream_snapshot_resumes_after_cancellation_despite_an_old_complete_cache() {
    let (app, mut view, mesh, _) = crate::picking_integration_tests::fixture();
    let world = app.worlds[0].clone();
    let dimension = world.dimensions[0].name.clone();
    let source = Arc::new(mcedit2_world::editing::Session::new(world, dimension).unwrap());
    let stream = crate::spectator_stream::Stream::new(
        (0, 0),
        source,
        view.camera.position,
        1,
        view.camera.yaw,
        1,
        false,
    )
    .unwrap();
    let snapshot = Arc::new(stream.snapshot(Arc::clone(&view.surface)));
    let partial = Arc::new(crate::spectator::StreamMesh {
        terrain: None,
        visibility: None,
        entities: None,
        complete: false,
        ..(*mesh).clone()
    });
    *snapshot.ready.lock().unwrap() = Some(Arc::clone(&partial));
    *view.cache.lock().unwrap() = Some(mesh);
    view.snapshot = Some(Arc::clone(&snapshot));
    *view.pending.lock().unwrap() = true;
    let scene = GpuScene(SceneSource::Spectator(view.clone()));
    let size = Size::new(694., 654.);
    assert!(!view.mesh_complete());
    assert!(redraw(&scene, size).is_some());
    *view.pending.lock().unwrap() = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let action = redraw(&scene, size);
        if view.mesh_complete() {
            let ready = snapshot.ready.lock().unwrap().clone().unwrap();
            assert!(!Arc::ptr_eq(&partial, &ready));
            assert!(ready.chunks.iter().any(|c| !c.quads().is_empty()));
            assert!(redraw(&scene, size).is_some());
            let primitive = scene.draw(
                &State::default(),
                mouse::Cursor::Unavailable,
                Rectangle::with_size(size),
            );
            assert!(Arc::ptr_eq(primitive.stream_mesh.as_ref().unwrap(), &ready));
            break;
        }
        assert!(action.is_some());
        assert!(
            std::time::Instant::now() < deadline,
            "部分发布的流式网格没有恢复"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
