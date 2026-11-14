//! 用原始 Java 样本和生产网格验证点选、编辑预览与撤销的接线。
use crate::{
    picking::{Intent, Owner, Request},
    spectator::{self, Camera, SpectatorScene, StreamMesh},
    App,
};
use mcedit2_world::{
    block_lookup::{read_world_chunk, Lookup},
    surface::{SurfaceView, SurfaceVoxel, ALL_FACES},
};
use std::{path::Path, sync::Arc};

/// 执行生产异步任务，不启动窗口；不仿造工作线程返回的候选会话。
#[track_caller]
pub(super) fn edit_task_output(task: iced::Task<crate::Message>) -> crate::Message {
    task_output(task).expect("编辑后台任务必须返回应用消息")
}

/// 页面已存在时只可能需要滚动控件；逻辑类别仍处理真实后台返回。
pub(super) fn task_output(task: iced::Task<crate::Message>) -> Option<crate::Message> {
    let mut outputs = task_outputs(task);
    assert!(outputs.len() <= 1, "编辑任务应只返回一条应用消息");
    outputs.pop()
}

pub(super) fn task_outputs(task: iced::Task<crate::Message>) -> Vec<crate::Message> {
    use iced::futures::StreamExt;
    let Some(mut stream) = iced_runtime::task::into_stream(task) else {
        return Vec::new();
    };
    iced::futures::executor::block_on(async move {
        let mut output = Vec::new();
        while let Some(action) = stream.next().await {
            match action {
                iced_runtime::Action::Output(message) => output.push(message),
                // 无窗口的逻辑类别不验收滚动 / 焦点；这些操作由原生窗口类别执行。
                iced_runtime::Action::Widget(_) => {}
                _ => panic!("编辑后台任务包含需要原生运行时的非控件操作"),
            }
        }
        output
    })
}

pub(super) fn finish_edit_preview(app: &mut App) {
    let message = edit_task_output(crate::block_edit::maintain(app));
    let _ = crate::update(app, message);
}
pub(super) fn fixture() -> (App, SpectatorScene, Arc<StreamMesh>, Request) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test_files/AnvilWorld_1.8");
    let world = mcedit2_world::inspect_world(root).unwrap();
    let dimension = world.dimensions[0].name.clone();
    let region =
        mcedit2_world::RegionIndex::open(world.dimensions[0].region_dir.join("r.-1.0.mca"))
            .unwrap();
    let (rx, rz) = (0u8..32)
        .flat_map(|x| (0u8..32).map(move |z| (x, z)))
        .find(|&(x, z)| region.slot(x, z).is_some())
        .unwrap();
    let (cx, cz) = (rx as i32 - 32, rz as i32);
    let chunk = read_world_chunk(&world, &dimension, cx, cz)
        .unwrap()
        .unwrap();
    let mut found = None;
    'sections: for section in &chunk.sections {
        for y in section.y as i32 * 16..section.y as i32 * 16 + 16 {
            for z in 0..16 {
                for x in 0..15 {
                    let a = chunk.block(x, y, z).unwrap();
                    let b = chunk.block(x + 1, y, z).unwrap();
                    let solid = |state: &mcedit2_world::sections::State| {
                        !matches!(
                            state.name.as_str(),
                            "numeric:0"
                                | "minecraft:air"
                                | "minecraft:cave_air"
                                | "minecraft:void_air"
                        ) && mcedit2_world::surface::material(state)
                            == mcedit2_world::surface::VoxelMaterial::Solid
                    };
                    if solid(a) && solid(b) {
                        found = Some(([cx * 16 + x as i32, y, cz * 16 + z as i32], a.clone()));
                        break 'sections;
                    }
                }
            }
        }
    }
    let ([x, y, z], state) = found.expect("样本源区块至少含两个相邻实体方块");
    let surface = Arc::new(SurfaceView {
        biomes: Default::default(),
        x: cx * 16,
        z: cz * 16,
        y_min: y,
        y_max: y + 15,
        width: 16,
        depth: 16,
        voxels: vec![SurfaceVoxel::new(
            [x, y, z],
            mcedit2_world::surface::VoxelData {
                kind: mcedit2_world::surface::material(&state),
                state: state.into(),
                faces: ALL_FACES,
                biome: None,
                staged: false,
                original_block_entity: false,
                span: 1,
                fluid: None,
                light: Default::default(),
            },
        )],
        source_tiles: vec![],
        source_chunks: 1,
        missing_columns: 0,
    });
    let scene = SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface: Arc::clone(&surface),
        camera: Camera {
            position: [x as f64, y as f64, z as f64 - 3.],
            yaw: 0.,
            pitch: 0.,
            zoom: 1.0,
            perspective: true,
        },
        texture_bank: None,
        show_textures: false,
        block_layers: Default::default(),
        quality: Default::default(),
        cache: Default::default(),
        presented: Default::default(),
        presented_view: Default::default(),
        pending: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Default::default(),
        lighting: crate::lighting::Table::unlit(),
    };
    let mesh = Arc::new(spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    let hit = mesh
        .pick(
            scene.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(200., 200.)),
            scene.quality,
        )
        .expect("真实生产网格必须可点选");
    let mut app = App {
        worlds: vec![world],
        workspace: {
            let mut state = crate::workspace_tabs::State::default();
            state.open(0);
            state
        },
        current: Some(0),
        spectator: true,
        surface: Some((0, 0, surface)),
        ..Default::default()
    };
    app.desktop.show_recent = false;
    *app.spectator_presented.lock().unwrap() = Some(Arc::clone(&mesh));
    *app.spectator_presented_view.lock().unwrap() = Some(spectator::PresentedView {
        camera: scene.camera.into(),
        viewport: crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(200., 200.)),
        quality: scene.quality,
        atlas: mesh.atlas.clone(),
        mesh: Arc::clone(&mesh),
    });
    let request = Request {
        key: (0, 0),
        mesh: Arc::clone(&mesh),
        hit,
        intent: Intent::Point,
    };
    (app, scene, mesh, request)
}

/// 与生产呈现合同一致：网格与相机快照同时交接，旧相机快照不能继续保活旧网格。
fn present(app: &App, scene: &SpectatorScene, mesh: Arc<StreamMesh>) {
    *app.spectator_presented_view.lock().unwrap() = Some(spectator::PresentedView {
        camera: scene.camera.into(),
        viewport: crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(200., 200.)),
        quality: scene.quality,
        atlas: mesh.atlas.clone(),
        mesh: Arc::clone(&mesh),
    });
    *app.spectator_presented.lock().unwrap() = Some(mesh);
}
fn previous_frame_is_released_only_after_current_complete_mesh_is_presented() {
    let (mut app, scene, old, request) = fixture();
    drop(request);
    app.spectator_fallback = Some(spectator::CompletedFrame {
        vertices: Arc::new(Vec::new()),
        stream_mesh: Some(Arc::clone(&old)),
        surface: Arc::clone(&scene.surface),
        atlas: None,
    });
    let old_lifetime = Arc::downgrade(&old);
    let mut replacement = (*old).clone();
    replacement.complete = false;
    let partial = Arc::new(replacement.clone());
    *app.spectator_cache.lock().unwrap() = Some(Arc::clone(&partial));
    present(&app, &scene, partial);
    app.retire_spectator_fallback();
    assert!(app.spectator_fallback.is_some());
    replacement.complete = true;
    let complete = Arc::new(replacement);
    *app.spectator_cache.lock().unwrap() = Some(Arc::clone(&complete));
    app.retire_spectator_fallback();
    assert!(
        app.spectator_fallback.is_some(),
        "后台完成但尚未呈现，必须保留旧帧"
    );
    present(&app, &scene, Arc::clone(&old));
    app.retire_spectator_fallback();
    assert!(
        app.spectator_fallback.is_some(),
        "呈现的旧完整帧不能冒充当前帧"
    );
    drop(old);
    present(&app, &scene, complete);
    app.retire_spectator_fallback();
    assert!(app.spectator_fallback.is_none());
    assert!(old_lifetime.upgrade().is_none());

    // 流式快照存在时，不能把局部预览缓存完成误认为整幅场景已完成。
    let stream = crate::spectator_stream::Stream::new(
        (0, 0),
        Arc::new(app.current_session((0, 0)).unwrap().clone()),
        scene.camera.position,
        crate::spectator_stream::DEFAULT_RADIUS,
        scene.camera.yaw,
        1,
        false,
    )
    .unwrap();
    let snapshot = Arc::new(stream.snapshot(Arc::clone(&scene.surface)));
    let prior = app.spectator_presented.lock().unwrap().clone().unwrap();
    app.spectator_fallback = Some(spectator::CompletedFrame {
        vertices: Arc::new(Vec::new()),
        stream_mesh: Some(Arc::clone(&prior)),
        surface: Arc::clone(&scene.surface),
        atlas: None,
    });
    app.spectator_stream_snapshot = Some(Arc::clone(&snapshot));
    app.retire_spectator_fallback();
    assert!(app.spectator_fallback.is_some());
    let next = Arc::new((*prior).clone());
    *snapshot.ready.lock().unwrap() = Some(Arc::clone(&next));
    app.retire_spectator_fallback();
    assert!(app.spectator_fallback.is_some());
    *app.spectator_presented.lock().unwrap() = Some(next);
    app.retire_spectator_fallback();
    assert!(app.spectator_fallback.is_none());
}

fn next_source_snapshot_does_not_wait_for_previous_gpu_handoff() {
    let (mut app, scene, mesh, _) = fixture();
    app.surface_exposed = true;
    app.use_gpu = true;
    app.spectator_camera = scene.camera;
    app.spectator_radius = 2;
    let _ = app.maintain_spectator_stream();
    let old = Arc::clone(app.spectator_stream_snapshot.as_ref().unwrap());
    *old.ready.lock().unwrap() = Some(Arc::new((*mesh).clone()));
    app.spectator_stream.as_mut().unwrap().dirty = true;
    assert!(!*app.spectator_pending.lock().unwrap());
    let _ = app.maintain_spectator_stream();
    let next = Arc::clone(app.spectator_stream_snapshot.as_ref().unwrap());
    assert!(
        !Arc::ptr_eq(&old, &next),
        "下一批 CPU 来源不应等待前批 GPU 上传"
    );
    assert!(
        Arc::ptr_eq(
            app.spectator_presented.lock().unwrap().as_ref().unwrap(),
            &mesh
        ),
        "仍保留真实已显示网格"
    );
    *app.spectator_pending.lock().unwrap() = true;
    app.spectator_stream.as_mut().unwrap().dirty = true;
    let _ = app.maintain_spectator_stream();
    assert!(
        Arc::ptr_eq(&next, app.spectator_stream_snapshot.as_ref().unwrap()),
        "CPU 建模进行时合并更新，不能反复撤销它"
    );
}

fn real_mesh_pick_enters_preview_stage_and_undo_without_writing_source() {
    let (mut app, scene, _, request) = fixture();
    let file = app.worlds[0].dimensions[0].region_dir.join("r.-1.0.mca");
    let before = std::fs::read(&file).unwrap();
    let source = (*request.hit.owner.as_ref().unwrap().state).clone();
    let [x, y, z] = request.hit.owner.as_ref().unwrap().position;
    app.pick_spectator(request.clone());
    assert_eq!(app.selected_surface, Some((x, y, z)));
    assert_eq!(
        app.selected_surface_face,
        Some(crate::isometric::Face::NegZ)
    );
    assert_eq!(app.edit_area, format!("{x},{y},{z};{x},{y},{z}"));
    assert_eq!(app.current_session((0, 0)).unwrap().pending_cells(), 0);
    app.edit_target = if source.name == "numeric:1" {
        "numeric:2[data=0]"
    } else {
        "numeric:1[data=0]"
    }
    .into();
    let target = mcedit2_world::editing::parse_state(&app.edit_target).unwrap();
    crate::block_edit::start(&mut app, crate::block_edit::Tool::Selection);
    assert!(app.block_edit.busy);
    finish_edit_preview(&mut app);
    assert_eq!(app.prepared.as_ref().unwrap().2.changed(), 1);
    assert_eq!(app.current_session((0, 0)).unwrap().pending_cells(), 0);
    assert_eq!(app.display_session((0, 0)).unwrap().pending_cells(), 1);
    let Lookup::Present(candidate) = app
        .display_session((0, 0))
        .unwrap()
        .visible_block((x, y, z))
        .unwrap()
    else {
        panic!("候选会话的修改应可查询");
    };
    assert_eq!(candidate.state, target);
    assert!(
        !app.pick_spectator(request.clone()),
        "候选场景不能作为真实编辑来源拾取"
    );
    // 正式源读取与生产网格必须包含候选目标，而实际编辑会话及磁盘仍未改变。
    let candidate_view = app
        .display_session((0, 0))
        .unwrap()
        .inspect_exposed_volume(x, z, y, y, 2, 1)
        .unwrap();
    assert!(candidate_view
        .voxels
        .iter()
        .any(|v| v.x == x && *v.state == target && v.staged));
    let candidate_scene = SpectatorScene {
        surface: Arc::new(candidate_view),
        ..scene.clone()
    };
    let candidate_mesh = spectator::build_scene_cancellable(&candidate_scene, &|| false).unwrap();
    assert!(candidate_mesh
        .chunks
        .iter()
        .any(|chunk| !chunk.quads().is_empty()));
    let task = crate::block_edit::apply(&mut app);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert_eq!(app.current_session((0, 0)).unwrap().pending_cells(), 1);
    let Lookup::Present(edited) = app
        .current_session((0, 0))
        .unwrap()
        .visible_block((x, y, z))
        .unwrap()
    else {
        panic!("暂存源方块应可查询");
    };
    assert_eq!(edited.state, target);
    // GPU 尚显示原网格时，不能把已变化的暂存源状态冒充当前点选对象。
    app.query_coordinates = "保持查询位置".into();
    app.pick_spectator(request);
    assert_eq!(app.query_coordinates, "保持查询位置");
    let task = crate::update(&mut app, crate::Message::UndoEdit);
    let message = edit_task_output(task);
    let _ = crate::update(&mut app, message);
    assert_eq!(app.current_session((0, 0)).unwrap().pending_cells(), 0);
    // 同一完整工作流继续审核删除、取消与过期异步返回。
    app.edit_target = "numeric:0[data=0]".into();
    crate::block_edit::start(&mut app, crate::block_edit::Tool::Selection);
    finish_edit_preview(&mut app);
    let removed = app
        .display_session((0, 0))
        .unwrap()
        .inspect_exposed_volume(x, z, y, y, 2, 1)
        .unwrap();
    assert!(removed.voxels.iter().all(|v| v.x != x));
    assert!(
        removed
            .voxels
            .iter()
            .any(|v| v.x == x + 1 && v.faces & mcedit2_world::surface::FACE_NEG_X != 0),
        "删除后相邻原始方块的内面应变为可见面"
    );
    let _ = crate::update(
        &mut app,
        crate::Message::BlockEdit(crate::block_edit::Action::Cancel),
    );
    assert!(app.block_edit.draft.is_none());
    let Lookup::Present(restored) = app
        .display_session((0, 0))
        .unwrap()
        .visible_block((x, y, z))
        .unwrap()
    else {
        panic!("取消应恢复实际会话来源");
    };
    assert_eq!(restored.state, source);
    crate::block_edit::start(&mut app, crate::block_edit::Tool::Selection);
    let stale_task = crate::block_edit::maintain(&mut app);
    let _ = crate::update(
        &mut app,
        crate::Message::TargetText("numeric:1[data=0]".into()),
    );
    let stale = edit_task_output(stale_task);
    let _ = crate::update(&mut app, stale);
    assert!(app.prepared.is_none() && app.block_edit.draft.is_none() && !app.block_edit.busy);
    assert_eq!(app.current_session((0, 0)).unwrap().pending_cells(), 0);
    assert_eq!(std::fs::read(file).unwrap(), before);
}

fn source_mesh_world_proof_and_drag_anchors_are_revalidated() {
    let (mut app, _, mesh, mut request) = fixture();
    request.key = (1, 0);
    app.pick_spectator(request.clone());
    assert!(app.selected_surface.is_none());
    request.key = (0, 0);
    *app.spectator_presented.lock().unwrap() = Some(Arc::new((*mesh).clone()));
    app.pick_spectator(request.clone());
    assert!(app.selected_surface.is_none());
    *app.spectator_presented.lock().unwrap() = Some(mesh);
    let [x, y, z] = request.hit.owner.as_ref().unwrap().position;
    let Lookup::Present(block) = app
        .current_session((0, 0))
        .unwrap()
        .visible_block((x + 1, y, z))
        .unwrap()
    else {
        panic!("相邻原始方块必须存在");
    };
    request.intent = Intent::Box(Owner {
        position: [x + 1, y, z],
        state: Arc::new(block.state),
    });
    app.pick_spectator(request);
    assert_eq!(app.corner_a, Some((x + 1, y, z)));
    assert_eq!(app.corner_b, Some((x, y, z)));
    assert_eq!(
        mcedit2_world::editing::parse_bounds(&app.edit_area)
            .unwrap()
            .volume()
            .unwrap(),
        2
    );
    app.inspect_surface(true);
    assert_eq!(
        app.corner_a,
        Some((x + 1, y, z)),
        "自由飞行的近景刷新须保留已选区"
    );
    app.corner_b = Some((100, 100, 100));
    app.use_3d_box();
    assert!(app.selection_box().is_some(), "大选框可继续显示");
    let selected = mcedit2_world::editing::parse_work_bounds(&app.edit_area).unwrap();
    assert_eq!(
        selected,
        mcedit2_world::editing::Bounds::spatial((x + 1, y, z), (100, 100, 100)).unwrap()
    );
    assert!(selected.volume().unwrap() > 4096);
    assert!(app.prepared.is_none());
}

fn cached_pick_geometry_preserves_world_position_when_mesh_origin_moves() {
    let (_, scene, first, request) = fixture();
    let style = crate::chunk_mesh_cache::ChunkMeshCache::style(None, false, Default::default());
    let templates = scene.chunk_cache.lock().unwrap().templates(&style);
    assert!(!templates.is_empty());
    let mut shifted = (*scene.surface).clone();
    shifted.x -= 16;
    shifted.z -= 16;
    shifted.y_min -= 8;
    let shifted = SpectatorScene {
        surface: Arc::new(shifted),
        ..scene
    };
    let next = spectator::build_scene_cancellable(&shifted, &|| false).unwrap();
    assert!(Arc::ptr_eq(
        &first.chunks[0].picking,
        &next.chunks[0].picking
    ));
    let hit = next
        .pick(
            shifted.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(200., 200.)),
            shifted.quality,
        )
        .unwrap();
    assert_eq!(hit.owner, request.hit.owner);
    assert_eq!(hit.outline, request.hit.outline);
    assert_eq!(hit.depth, request.hit.depth);
    // 新位置必须重新构建源几何，同时复用真实模型模板，拾取仍绑定新坐标。
    let mut translated = (*shifted.surface).clone();
    translated.x += 32;
    for voxel in &mut translated.voxels {
        voxel.x += 32;
    }
    let mut camera = shifted.camera;
    camera.position[0] += 32.;
    let moved = SpectatorScene {
        surface: Arc::new(translated),
        camera,
        ..shifted
    };
    let rendered = spectator::build_scene_cancellable(&moved, &|| false).unwrap();
    let hit = rendered
        .pick(
            moved.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(200., 200.)),
            moved.quality,
        )
        .unwrap();
    assert_eq!(
        hit.owner.unwrap().position[0],
        request.hit.owner.unwrap().position[0] + 32
    );
    let reused = moved.chunk_cache.lock().unwrap().templates(&style);
    let mut count = 0;
    for (state, variants) in templates.iter() {
        if let Some(next) = reused.get(state) {
            for (key, template) in variants {
                if let Some(next) = next.get(key) {
                    assert!(Arc::ptr_eq(template, next));
                    count += 1;
                }
            }
        }
    }
    assert!(count > 0, "跨批次须复用实际模板对象");
    // 交替经过另一种真实方块后，返回原模型仍应复用模板，而不是每一批清空热集。
    let mut alternate = (*moved.surface).clone();
    let mut palette = mcedit2_world::surface::VoxelPalette::default();
    for voxel in &mut alternate.voxels {
        *voxel = palette.updated(voxel, |data| {
            data.state = Arc::new(mcedit2_world::sections::State {
                name: "minecraft:dirt".into(),
                properties: vec![],
            });
        });
    }
    let alternate = SpectatorScene {
        surface: Arc::new(alternate),
        ..moved.clone()
    };
    let different = spectator::build_scene_cancellable(&alternate, &|| false).unwrap();
    assert!(!different.chunks.is_empty());
    let returned = spectator::build_scene_cancellable(&moved, &|| false).unwrap();
    assert!(!returned.chunks.is_empty());
    let retained = moved.chunk_cache.lock().unwrap().templates(&style);
    for (state, variants) in reused.iter() {
        for (key, template) in variants {
            assert!(Arc::ptr_eq(template, &retained[state][key]));
        }
    }
    // 源状态改变但模型字节相同：GPU 几何复用，拾取仍须绑定最新源状态。
    let mut metadata = (*moved.surface).clone();
    let mut palette = mcedit2_world::surface::VoxelPalette::default();
    for voxel in &mut metadata.voxels {
        *voxel = palette.updated(voxel, |data| {
            Arc::make_mut(&mut data.state)
                .properties
                .push(("source_tag".into(), "updated".into()));
        });
    }
    let metadata = SpectatorScene {
        surface: Arc::new(metadata),
        presented: Arc::new(std::sync::Mutex::new(Some(Arc::new(rendered.clone())))),
        ..moved.clone()
    };
    let updated = spectator::build_scene_cancellable(&metadata, &|| false).unwrap();
    assert!(Arc::ptr_eq(
        updated.chunks[0].geometry(),
        rendered.chunks[0].geometry()
    ));
    assert!(!Arc::ptr_eq(
        &updated.chunks[0].picking,
        &rendered.chunks[0].picking
    ));
    let hit = updated
        .pick(
            metadata.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(200., 200.)),
            metadata.quality,
        )
        .unwrap();
    assert!(hit
        .owner
        .unwrap()
        .state
        .properties
        .iter()
        .any(|(key, value)| key == "source_tag" && value == "updated"));
    let new_style = crate::chunk_mesh_cache::ChunkMeshCache::style(None, true, Default::default());
    assert!(
        moved
            .chunk_cache
            .lock()
            .unwrap()
            .templates(&new_style)
            .is_empty(),
        "显示样式改变必须使旧模板失效"
    );
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Rendering {
        if !crate::spectator_stream::serial_reference() {
            next_source_snapshot_does_not_wait_for_previous_gpu_handoff();
        }
        previous_frame_is_released_only_after_current_complete_mesh_is_presented();
        real_mesh_pick_enters_preview_stage_and_undo_without_writing_source();
        source_mesh_world_proof_and_drag_anchors_are_revalidated();
        cached_pick_geometry_preserves_world_position_when_mesh_origin_moves();
    }
}
