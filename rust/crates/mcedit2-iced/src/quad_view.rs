//! 共用同一编辑快照的四视图；三个正交切面和等距视图具有独立绘制缓存。
use crate::{
    gpu_scene::{GpuScene, SceneSource},
    isometric::{FrameCache, IsometricSurface, RenderFace},
    slice_canvas::{Map2D, Plane},
    view_navigation::{QuadPane, Target},
    App, Message,
};
use iced::{
    widget::{button, canvas, column, container, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::{
    block_lookup::parse_xyz,
    editing::{Bounds, Session, Slice, SliceCell, VerticalSlice},
    surface::{inspect_volume_with_visibility, SurfaceView, VolumeRequest},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    center: [i32; 3],
    horizontal: Arc<Slice>,
    front: Arc<VerticalSlice>,
    side: Arc<VerticalSlice>,
    surface: Arc<SurfaceView>,
    assets: crate::SceneAssets,
    show_textures: bool,
    block_layers: crate::block_visibility::Layers,
}

fn bounds(center: [i32; 3]) -> Result<Bounds, String> {
    let minimum = center.map(|v| v.checked_sub(15));
    let maximum = center.map(|v| v.checked_add(16));
    let ([Some(x1), Some(y1), Some(z1)], [Some(x2), Some(y2), Some(z2)]) = (minimum, maximum)
    else {
        return Err("四视图坐标溢出".into());
    };
    Bounds::new((x1, y1, z1), (x2, y2, z2)).map_err(|e| e.to_string())
}

fn read(
    session: &Session,
    center: [i32; 3],
    assets: crate::SceneAssets,
    show_textures: bool,
    block_layers: crate::block_visibility::Layers,
    cancelled: &AtomicBool,
) -> Result<Snapshot, String> {
    let b = bounds(center)?;
    let check = || {
        if cancelled.load(Ordering::Acquire) {
            Err("四视图读取已取消".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let horizontal = session
        .inspect_slice(Bounds {
            minimum: [b.minimum[0], center[1], b.minimum[2]],
            maximum: [b.maximum[0], center[1], b.maximum[2]],
        })
        .map_err(|e| e.to_string())?;
    check()?;
    let front = session
        .inspect_vertical_slice(Bounds {
            minimum: [b.minimum[0], b.minimum[1], center[2]],
            maximum: [b.maximum[0], b.maximum[1], center[2]],
        })
        .map_err(|e| e.to_string())?;
    check()?;
    let side = session
        .inspect_vertical_slice(Bounds {
            minimum: [center[0], b.minimum[1], b.minimum[2]],
            maximum: [center[0], b.maximum[1], b.maximum[2]],
        })
        .map_err(|e| e.to_string())?;
    check()?;
    let mut visibility = crate::source_visibility::Reader::new(assets.clone(), show_textures)
        .with_layers(block_layers);
    let surface = inspect_volume_with_visibility(
        session,
        VolumeRequest {
            x: b.minimum[0],
            z: b.minimum[2],
            vertical: b.minimum[1]..=b.maximum[1],
            width: 32,
            depth: 32,
            top_cut: None,
        },
        &mut visibility,
        &|| cancelled.load(Ordering::Acquire),
    )
    .map_err(|e| e.to_string())?;
    check()?;
    Ok(Snapshot {
        center,
        horizontal: Arc::new(horizontal),
        front: Arc::new(front),
        side: Arc::new(side),
        surface: Arc::new(surface),
        assets,
        show_textures,
        block_layers,
    })
}

#[derive(Default)]
pub(crate) struct Panel {
    center: String,
    snapshot: Option<Snapshot>,
    key: Option<(usize, usize)>,
    nonce: u64,
    source_generation: Arc<()>,
    busy: bool,
    requested_center: Option<((usize, usize), [i32; 3])>,
    cancelled: Arc<AtomicBool>,
    resource_reload_attempt: Option<([i32; 3], crate::source_visibility::Identity)>,
    pub(crate) report: String,
    yaw: i32,
    pitch: i32,
    navigation: [crate::view_navigation::Viewport; 4],
    navigation_key: Option<(usize, usize)>,
    saved_center: Option<[i32; 3]>,
    face_cache: FrameCache<RenderFace>,
    framing_cache: crate::orthographic_framing::Cache,
    canvas_cache: crate::canvas_raster::Cache,
    mesh_cache: FrameCache<crate::gpu_mesh::Vertex>,
    pending: Arc<Mutex<Option<iced::Size>>>,
    pub(crate) rendered_cache: Arc<Mutex<Option<Arc<crate::projected_entities::Rendered>>>>,
    pub(crate) presented: Arc<Mutex<Option<Arc<crate::projected_entities::Rendered>>>>,
    pub(crate) grid_caches: [Arc<crate::projected_entities::GridCache>; 3],
}

#[derive(Debug, Clone)]
pub(crate) enum Action {
    Center(String),
    UseSelection,
    Load,
    Loaded(u64, (usize, usize), Result<Box<Snapshot>, String>),
    Pick(u8, usize, usize),
    Scene(Box<Message>),
    Move(i32, i32, i32),
}

impl Panel {
    pub(crate) fn brush_source(&self, pane: QuadPane) -> Option<crate::brush_stroke::Source> {
        use crate::brush_stroke::Source;
        let snapshot = self.snapshot.as_ref()?;
        let target = Target::Quad(pane);
        Some(match pane {
            QuadPane::Horizontal => Source::Horizontal(Arc::clone(&snapshot.horizontal), target),
            QuadPane::Front => Source::Vertical(Arc::clone(&snapshot.front), target),
            QuadPane::Side => Source::Vertical(Arc::clone(&snapshot.side), target),
            QuadPane::Isometric => Source::Surface(Arc::clone(&snapshot.surface), target),
        })
    }
    pub(crate) fn accepts_brush_source(
        &self,
        key: (usize, usize),
        source: &crate::brush_stroke::Source,
    ) -> bool {
        if self.key != Some(key) {
            return false;
        }
        let Some(snapshot) = &self.snapshot else {
            return false;
        };
        use crate::brush_stroke::Source;
        match source {
            Source::Surface(s, Target::Quad(QuadPane::Isometric)) => {
                Arc::ptr_eq(s, &snapshot.surface)
            }
            Source::Horizontal(s, Target::Quad(QuadPane::Horizontal)) => {
                Arc::ptr_eq(s, &snapshot.horizontal)
            }
            Source::Vertical(s, Target::Quad(QuadPane::Front)) => Arc::ptr_eq(s, &snapshot.front),
            Source::Vertical(s, Target::Quad(QuadPane::Side)) => Arc::ptr_eq(s, &snapshot.side),
            _ => false,
        }
    }
    pub(crate) fn saved_navigation(&self, key: (usize, usize)) -> Option<crate::view_state::Quad> {
        (self.navigation_key == Some(key)).then(|| crate::view_state::Quad {
            center: self
                .snapshot
                .as_ref()
                .map(|s| s.center)
                .or(self.saved_center),
            yaw: self.yaw,
            pitch: self.pitch,
            navigation: self.navigation.map(crate::view_state::Viewport::from),
        })
    }
    pub(crate) fn restore_navigation(
        &mut self,
        key: (usize, usize),
        saved: Option<&crate::view_state::Quad>,
    ) {
        self.navigation_key = Some(key);
        self.saved_center = saved.and_then(|s| s.center);
        self.navigation = saved
            .map(|s| s.navigation.map(Into::into))
            .unwrap_or_default();
        self.yaw = saved.map_or(0, |s| s.yaw);
        self.pitch = saved.map_or(30, |s| s.pitch);
        self.center = saved
            .and_then(|s| s.center)
            .map(|[x, y, z]| format!("{x},{y},{z}"))
            .unwrap_or_default();
        self.reset_projection();
    }
    pub(crate) fn source_generation(&self) -> Arc<()> {
        Arc::clone(&self.source_generation)
    }
    /// 四个实际源窗口分别提供范围，调用方按区块并集去重，不合并成大包围盒。
    pub(crate) fn source_scope(&self) -> Option<((usize, usize), Vec<Bounds>)> {
        let key = self.key?;
        let snapshot = self.snapshot.as_ref()?;
        Some((
            key,
            vec![
                snapshot.horizontal.bounds,
                snapshot.front.bounds,
                snapshot.side.bounds,
                bounds(snapshot.center).ok()?,
            ],
        ))
    }
    pub(crate) fn has_presented(
        &self,
        rendered: &Arc<crate::projected_entities::Rendered>,
    ) -> bool {
        self.grid_caches
            .iter()
            .any(|cache| cache.is_presented(rendered))
            || self
                .presented
                .lock()
                .expect("四视图呈现状态锁被毒化")
                .as_ref()
                .is_some_and(|shown| Arc::ptr_eq(shown, rendered))
    }
    pub(crate) fn reset_framing(&mut self) {
        self.framing_cache = Default::default();
    }
    fn cancel_load(&mut self) {
        self.requested_center = None;
        self.cancelled.store(true, Ordering::Release);
        self.nonce = self.nonce.wrapping_add(1);
        self.busy = false;
    }
    pub(crate) fn cancel_projection(&self) {
        crate::mesh_async::cancel_isometric(&self.pending);
    }
    pub(crate) fn reset_projection(&mut self) {
        self.cancel_projection();
        self.face_cache = Default::default();
        self.canvas_cache = Default::default();
        self.mesh_cache = Default::default();
        self.pending = Default::default();
        self.rendered_cache = Default::default();
        self.presented = Default::default();
        for cache in &self.grid_caches {
            cache.cancel();
        }
        self.grid_caches = Default::default();
    }
    pub(crate) fn invalidate(&mut self) {
        self.reset_framing();
        self.cancel_load();
        self.snapshot = None;
        self.key = None;
        self.resource_reload_attempt = None;
        self.reset_projection();
    }
    /// 显隐改变后撤下旧几何与拾取，保留真实已应用中心供资源维护重读。
    pub(crate) fn invalidate_resources(&mut self) {
        let requested = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| self.key.map(|key| (key, snapshot.center)))
            .or(self.requested_center);
        self.invalidate();
        self.requested_center = requested;
    }
    pub(crate) fn appearance(&self) -> Option<(crate::SceneAssets, bool)> {
        self.snapshot
            .as_ref()
            .map(|snapshot| (snapshot.assets.clone(), snapshot.show_textures))
    }
}

/// 始终读取明确的已应用坐标；自动资源刷新不得提交输入框里的未确认草稿。
fn start_read(app: &mut App, center: [i32; 3]) -> Task<Message> {
    app.quad.cancel_load();
    let source = (|| {
        bounds(center)?;
        let key = app.current_key().ok_or("请先选择世界")?;
        Ok::<_, String>((key, app.display_session(key)?.clone()))
    })();
    let (key, source) = match source {
        Ok(source) => source,
        Err(error) => {
            log::error!("[view] 四视图读取请求被拒绝：{error}");
            app.quad.report = error;
            return Task::none();
        }
    };
    app.quad.requested_center = Some((key, center));
    app.quad.cancelled = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::clone(&app.quad.cancelled);
    let assets = app.texture_bank.clone();
    let show_textures = app.show_textures;
    let block_layers = app.entity_models.visibility.block_layers();
    app.quad.resource_reload_attempt = Some((
        center,
        crate::source_visibility::identity(assets.as_ref(), show_textures, block_layers),
    ));
    let nonce = app.quad.nonce;
    app.quad.busy = true;
    app.quad.report = "正在后台读取四个共用选点的视图…".into();
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::Builder::new()
                .name("quad-source".into())
                .spawn(move || {
                    if tx
                        .send(
                            read(
                                &source,
                                center,
                                assets,
                                show_textures,
                                block_layers,
                                &cancelled,
                            )
                            .map(Box::new),
                        )
                        .is_err()
                    {
                        log::error!("四视图读取完成，但界面已关闭");
                    }
                })
                .map_err(|error| format!("无法启动四视图读取线程：{error}"))?;
            rx.await
                .map_err(|error| format!("四视图读取线程异常：{error}"))?
        },
        move |result| Message::Quad(Action::Loaded(nonce, key, result)),
    )
}

/// 同一已应用中心切换候选或真实来源，不采纳尚未确认的中心输入。
pub(crate) fn source_changed(app: &mut App) -> Task<Message> {
    let center = app.quad.snapshot.as_ref().map(|s| s.center).or_else(|| {
        app.quad
            .requested_center
            .filter(|(key, _)| Some(*key) == app.current_key())
            .map(|(_, center)| center)
    });
    app.quad.invalidate();
    if let Some(center) = center {
        start_read(app, center)
    } else {
        Task::none()
    }
}

/// 每份资源对当前已应用窗口只自动尝试一次；失败保留旧源及其原始材质。
pub(crate) fn maintain_resources(app: &mut App) -> Task<Message> {
    if app.quad.busy {
        return Task::none();
    }
    let center = if let Some(snapshot) = app.quad.snapshot.as_ref() {
        (app.quad.key == app.current_key()).then_some(snapshot.center)
    } else {
        app.quad
            .requested_center
            .filter(|(key, _)| Some(*key) == app.current_key())
            .map(|(_, center)| center)
    };
    let Some(center) = center else {
        return Task::none();
    };
    let identity = crate::source_visibility::identity(
        app.texture_bank.as_ref(),
        app.show_textures,
        app.entity_models.visibility.block_layers(),
    );
    let request = (center, identity);
    if app.quad.snapshot.as_ref().is_some_and(|snapshot| {
        crate::source_visibility::identity(
            snapshot.assets.as_ref(),
            snapshot.show_textures,
            snapshot.block_layers,
        ) == request.1
    }) || app.quad.resource_reload_attempt.as_ref() == Some(&request)
    {
        return Task::none();
    }
    let center = request.0;
    app.quad.resource_reload_attempt = Some(request);
    start_read(app, center)
}

fn select_with_tool(app: &mut App, xyz: (i32, i32, i32), pane: usize) -> Task<Message> {
    if app.reviewing_block_preview() {
        return Task::none();
    }
    let viewport = app.quad.navigation[pane];
    let (name, yaw, pitch) = match pane {
        0 => ("quad_horizontal", 0.0, 90.0),
        1 => ("quad_front", 0.0, 0.0),
        2 => ("quad_side", 90.0, 0.0),
        _ => ("quad_isometric", app.quad.yaw as f32, app.quad.pitch as f32),
    };
    let selection = select(app, xyz);
    if app.object_inspector.picking_chunks {
        return selection;
    }
    let click = crate::plugin_ui::tool_click_projected(
        app,
        name,
        yaw,
        pitch,
        viewport.zoom,
        [viewport.pan.x, viewport.pan.y],
    );
    Task::batch([selection, click])
}

fn select(app: &mut App, xyz: (i32, i32, i32)) -> Task<Message> {
    crate::plugin_ui::invalidate(&mut app.plugins);
    app.query_coordinates = format!("{}, {}, {}", xyz.0, xyz.1, xyz.2);
    app.selected_block = app.quad.snapshot.as_ref().and_then(|s| {
        let surface = s
            .surface
            .voxels
            .iter()
            .find(|v| (v.x, v.y, v.z) == xyz && v.span == 1)
            .map(|v| v.state.as_ref().clone());
        let horizontal = || {
            if xyz.1 != s.center[1] {
                return None;
            }
            let col = usize::try_from(xyz.0 as i64 - s.horizontal.bounds.minimum[0] as i64).ok()?;
            let row = usize::try_from(xyz.2 as i64 - s.horizontal.bounds.minimum[2] as i64).ok()?;
            match s.horizontal.rows.get(row)?.get(col)? {
                SliceCell::Present { state, .. } => Some(state.clone()),
                _ => None,
            }
        };
        let vertical = || {
            [&s.front, &s.side].into_iter().find_map(|plane| {
                plane.rows.iter().enumerate().find_map(|(row, cells)| {
                    cells.iter().enumerate().find_map(|(col, cell)| match cell {
                        SliceCell::Present { state, .. }
                            if plane.coordinate(row, col) == Some(xyz) =>
                        {
                            Some(state.clone())
                        }
                        _ => None,
                    })
                })
            })
        };
        Some(crate::block_names::Selection {
            key: app.current_key()?,
            position: xyz,
            state: surface.or_else(horizontal).or_else(vertical)?,
        })
    });
    app.block_result = app
        .selected_block
        .as_ref()
        .map_or_else(String::new, |selected| {
            format!(
                "源快照 {:?} => {}",
                xyz,
                mcedit2_world::editing::render_state(&selected.state)
            )
        });
    if let Some(task) = crate::object_inspector::picked(app) {
        return task;
    }
    app.edit_area = format!(
        "{},{},{};{},{},{}",
        xyz.0, xyz.1, xyz.2, xyz.0, xyz.1, xyz.2
    );
    app.clear_block_preview();
    app.be_prepared = None;
    app.save_preview = None;
    app.quad.center = app.query_coordinates.clone();
    update(app, Action::Load)
}

pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Center(value) => {
            let requested = app.quad.requested_center;
            app.quad.cancel_load();
            app.quad.requested_center = requested;
            app.quad.center = value;
        }
        Action::UseSelection => {
            app.quad.center = app.query_coordinates.clone();
            return update(app, Action::Load);
        }
        Action::Move(dx, dy, dz) => {
            let result = parse_xyz(&app.quad.center)
                .map_err(|e| e.to_string())
                .and_then(|(x, y, z)| {
                    match (x.checked_add(dx), y.checked_add(dy), z.checked_add(dz)) {
                        (Some(x), Some(y), Some(z)) => Ok((x, y, z)),
                        _ => Err("四视图移动坐标溢出".into()),
                    }
                });
            match result {
                Ok((x, y, z)) => {
                    app.quad.center = format!("{x},{y},{z}");
                    return update(app, Action::Load);
                }
                Err(e) => {
                    log::error!("[view] 四视图移动被拒绝：{e}");
                    app.quad.report = e;
                }
            }
        }
        Action::Load => {
            // 同一世界的旧源快照仍可显示；清空会使 Scrollable 折叠并跳回顶部。
            // nonce 先作废旧读取，只有当前请求的完整结果才替换快照。
            match parse_xyz(&app.quad.center).map_err(|e| e.to_string()) {
                Err(e) => {
                    app.quad.cancel_load();
                    log::error!("[view] 四视图读取请求被拒绝：{e}");
                    app.quad.report = e;
                }
                Ok(center) => return start_read(app, center.into()),
            }
        }
        Action::Loaded(nonce, key, result) => {
            if nonce != app.quad.nonce || app.current_key() != Some(key) {
                return Task::none();
            }
            app.quad.busy = false;
            app.quad.requested_center = None;
            match result {
                Ok(snapshot) => {
                    app.quad.saved_center = Some(snapshot.center);
                    app.quad.report=format!("共同中心 {:?}；点击任一已存在方块可同步切面，紫色与棕色分别表示缺失区块和区段。",snapshot.center);
                    app.quad.snapshot = Some(*snapshot);
                    app.quad.source_generation = Arc::new(());
                    app.quad.key = Some(key);
                    if app.quad.navigation_key != Some(key) {
                        app.quad.navigation = Default::default();
                        app.quad.pitch = 30;
                        app.quad.yaw = 0;
                        app.quad.navigation_key = Some(key);
                    }
                    app.quad.reset_projection();
                    app.quad.reset_framing();
                    log::error!(
                        "[view] 四视图已提交：中心 {:?}，倍率 {:?}，平移 {:?}",
                        app.quad.snapshot.as_ref().unwrap().center,
                        app.quad.navigation.map(|v| v.zoom),
                        app.quad.navigation.map(|v| [v.pan.x, v.pan.y])
                    );
                }
                Err(e) => {
                    log::error!("[view] 四视图读取失败：{e}");
                    app.quad.report = format!("四视图读取失败：{e}");
                }
            }
        }
        Action::Pick(plane, row, col) => {
            if app.quad.key != app.current_key() {
                return Task::none();
            }
            let Some(snapshot) = app.quad.snapshot.as_ref() else {
                return Task::none();
            };
            let coordinate = match plane {
                0 => snapshot
                    .horizontal
                    .rows
                    .get(row)
                    .and_then(|r| r.get(col))
                    .and_then(|cell| {
                        matches!(cell, SliceCell::Present { .. }).then(|| {
                            (
                                snapshot.horizontal.bounds.minimum[0] + col as i32,
                                snapshot.center[1],
                                snapshot.horizontal.bounds.minimum[2] + row as i32,
                            )
                        })
                    }),
                1 | 2 => {
                    let view = if plane == 1 {
                        &snapshot.front
                    } else {
                        &snapshot.side
                    };
                    view.rows
                        .get(row)
                        .and_then(|r| r.get(col))
                        .filter(|c| matches!(c, SliceCell::Present { .. }))
                        .and_then(|_| view.coordinate(row, col))
                }
                _ => None,
            };
            if let Some(xyz) = coordinate {
                return select_with_tool(app, xyz, plane as usize);
            }
        }
        Action::Scene(message) => match *message {
            Message::PickSurfaceFace(x, y, z, _) => {
                if app.quad.key == app.current_key()
                    && app.quad.snapshot.as_ref().is_some_and(|s| {
                        s.surface
                            .voxels
                            .iter()
                            .any(|v| (v.x, v.y, v.z) == (x, y, z))
                    })
                {
                    return select_with_tool(app, (x, y, z), 3);
                }
            }
            Message::OrbitPitchSurface(yaw, pitch) => {
                app.quad.yaw = (app.quad.yaw + yaw).rem_euclid(360);
                app.quad.pitch = (app.quad.pitch + pitch).clamp(15, 80);
                app.quad.reset_projection();
            }
            Message::PanSurface(x, y) => {
                navigate(
                    app,
                    crate::view_navigation::QuadPane::Isometric,
                    crate::view_navigation::Action::Pan(iced::Vector::new(x as f32, y as f32)),
                );
            }
            Message::ZoomSurface(delta) => {
                navigate(
                    app,
                    crate::view_navigation::QuadPane::Isometric,
                    crate::view_navigation::Action::centered_zoom(delta as f32),
                );
            }
            Message::HoverSurface(view, position) => {
                if app.quad.key == app.current_key() {
                    crate::world_hover::hover(app, view, position);
                }
            }
            other => {
                app.quad.report = format!("四视图收到不支持的操作：{other:?}");
            }
        },
    }
    Task::none()
}

pub(crate) fn navigate(
    app: &mut App,
    pane: crate::view_navigation::QuadPane,
    action: crate::view_navigation::Action,
) {
    use crate::view_navigation::{Action, QuadPane};
    match app.quad.navigation[pane.index()].apply(action) {
        Ok(changed) => {
            let reset_orbit = pane == QuadPane::Isometric && matches!(action, Action::Reset);
            if reset_orbit {
                app.quad.yaw = 0;
                app.quad.pitch = 30;
            }
            if pane == QuadPane::Isometric && (changed || reset_orbit) {
                app.quad.reset_projection();
            }
        }
        Err(error) => {
            log::error!("[view] 四视图 {pane:?} 导航失败：{error}");
            app.quad.report = error;
        }
    }
}

pub(crate) fn controls(app: &App) -> Element<'_, Message> {
    let panel = &app.quad;
    let control = |label, action| button(label).on_press(Message::Quad(action));
    let mut body = column![
        text("四视图：水平、正面、侧面与等距观察").size(22),
        row![
            text_input("共同中心 X,Y,Z", &panel.center)
                .on_input(|s| Message::Quad(Action::Center(s))),
            control("读取四视图", Action::Load),
            control("使用当前选点", Action::UseSelection)
        ]
        .spacing(8)
        .wrap(),
        row![
            control("X − 16", Action::Move(-16, 0, 0)),
            control("X + 16", Action::Move(16, 0, 0)),
            control("Z − 16", Action::Move(0, 0, -16)),
            control("Z + 16", Action::Move(0, 0, 16)),
            control("Y − 1", Action::Move(0, -1, 0)),
            control("Y + 1", Action::Move(0, 1, 0))
        ]
        .spacing(5)
        .wrap(),
    ]
    .spacing(8);
    if let Some(name) = app.block_name_info() {
        body = body.push(text(name));
    }
    body.into()
}

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    use crate::view_navigation::{QuadPane, Target};
    let panel = &app.quad;
    let mut body = column![]
        .spacing(8)
        .width(Length::Fill)
        .height(Length::Fill);
    if let Some(snapshot) = panel
        .snapshot
        .as_ref()
        .filter(|_| panel.key == app.current_key())
    {
        let lighting = &crate::lighting::Table::unlit();
        let quad_layers = app.scene_layers.view_for(
            app.current_key(),
            crate::scene_layers::SourceRef::Quad(&panel.source_generation),
        );
        let map = |plane, number, pane: QuadPane| {
            let canvas: Element<'_, Message> = Map2D {
                selection_handle: crate::selection_handle::config(app),
                chunk_column: crate::object_inspector::selected_column(app),
                brush: crate::brush_cursor::with_assets(
                    crate::brush_stroke::config(app),
                    snapshot.assets.clone(),
                    snapshot.show_textures,
                ),
                brush_key: app.current_key(),
                brush_source: panel.brush_source(pane),
                layers: quad_layers.clone(),
                plane,
                navigation: panel.navigation[pane.index()],
                navigation_target: Target::Quad(pane),
                selected: Some(snapshot.center.into()),
                texture_bank: snapshot.assets.as_deref(),
                show_textures: snapshot.show_textures,
                block_layers: snapshot.block_layers,
                entity_frame: app
                    .entity_models
                    .frame
                    .as_ref()
                    .filter(|frame| {
                        frame.displays(app.entity_models.enabled, app.entity_models.visibility)
                            && frame.show_textures == snapshot.show_textures
                            && snapshot
                                .assets
                                .as_ref()
                                .and_then(|bank| bank.atlas())
                                .is_some_and(|atlas| {
                                    crate::textures::atlas_compatible(&frame.atlas, atlas)
                                })
                            && Some(frame.snapshot.key) == app.current_key()
                            && app
                                .scene_layers
                                .snapshot
                                .as_ref()
                                .is_some_and(|source| Arc::ptr_eq(source, &frame.snapshot))
                    })
                    .cloned(),
                grid_cache: Arc::clone(&panel.grid_caches[number as usize]),
                lighting: std::sync::Arc::clone(lighting),
                quality: app.gpu_quality,
                source_revision: panel.nonce,
            }
            .element(app, Length::Fill);
            let canvas = canvas.map(move |message| match message {
                Message::PickSlice(x, _, z) => Message::Quad(Action::Pick(
                    number,
                    (z - snapshot.horizontal.bounds.minimum[2]) as usize,
                    (x - snapshot.horizontal.bounds.minimum[0]) as usize,
                )),
                Message::PickVertical(row, col) => Message::Quad(Action::Pick(number, row, col)),
                other => other,
            });
            crate::world_ruler::plane(
                crate::navigation_bindings::wrap(
                    app,
                    Target::Quad(pane),
                    crate::brush_cursor::overlay(
                        canvas,
                        crate::brush_stroke::config(app),
                        Target::Quad(pane),
                        Length::Fill,
                    ),
                ),
                plane,
                panel.navigation[pane.index()],
                Length::Fill,
                app.language,
                app.minimap.capture(app.current_key(), Target::Quad(pane)),
                1.,
            )
        };
        let surface = IsometricSurface {
            selection_handle: crate::selection_handle::config(app),
            brush: crate::brush_cursor::with_assets(
                crate::brush_stroke::config(app),
                snapshot.assets.clone(),
                snapshot.show_textures,
            ),
            brush_key: app.current_key(),
            surface: Arc::clone(&snapshot.surface),
            texture_bank: snapshot.assets.clone(),
            show_textures: snapshot.show_textures,
            block_layers: snapshot.block_layers,
            camera_quarters: 0,
            yaw_degrees: panel.yaw,
            pitch_degrees: panel.pitch,
            zoom: panel.navigation[QuadPane::Isometric.index()].zoom,
            pan_x: panel.navigation[QuadPane::Isometric.index()].pan.x,
            pan_y: panel.navigation[QuadPane::Isometric.index()].pan.y,
            navigation_target: Target::Quad(QuadPane::Isometric),
            selected: Some(snapshot.center.into()),
            hovered: None,
            selection_box: app.selection_box(),
            show_frame: true,
            show_axes: true,
            show_chunk_grid: app.surface_chunk_grid,
            show_section_guides: false,
            gpu_faces: app.use_gpu,
            gpu_quality: app.gpu_quality,
            frame_cache: Arc::clone(&panel.face_cache),
            framing_cache: Arc::clone(&panel.framing_cache),
            canvas_cache: Arc::clone(&panel.canvas_cache),
            gpu_frame_cache: Arc::clone(&panel.mesh_cache),
            gpu_pending: Arc::clone(&panel.pending),
            entity_frame: app
                .entity_models
                .frame
                .as_ref()
                .filter(|frame| {
                    frame.displays(app.entity_models.enabled, app.entity_models.visibility)
                        && frame.show_textures == snapshot.show_textures
                        && snapshot
                            .assets
                            .as_ref()
                            .and_then(|bank| bank.atlas())
                            .is_some_and(|atlas| {
                                crate::textures::atlas_compatible(&frame.atlas, atlas)
                            })
                        && Some(frame.snapshot.key) == app.current_key()
                        && app
                            .scene_layers
                            .snapshot
                            .as_ref()
                            .is_some_and(|source| Arc::ptr_eq(source, &frame.snapshot))
                })
                .cloned(),
            entity_clip: bounds(snapshot.center).ok().map(|b| {
                (
                    b.minimum.map(|n| n as f64 - 0.5),
                    b.maximum.map(|n| n as f64 + 0.5),
                )
            }),
            rendered_cache: Arc::clone(&panel.rendered_cache),
            presented: Arc::clone(&panel.presented),
            lighting: Arc::clone(lighting),
        };
        let drop_surface = surface.clone();
        let annotation_projection = crate::scene_layers::ProjectionSource::Bounded(surface.clone());
        let scene: Element<'_, Message> = if app.use_gpu {
            iced::widget::stack![
                iced::widget::shader(GpuScene(SceneSource::Isometric(surface.clone())))
                    .width(Length::Fill)
                    .height(Length::Fill),
                canvas(surface).width(Length::Fill).height(Length::Fill)
            ]
            .into()
        } else {
            iced::widget::stack![
                canvas(crate::canvas_raster::ImageLayer(surface.clone()))
                    .width(Length::Fill)
                    .height(Length::Fill),
                canvas(surface).width(Length::Fill).height(Length::Fill)
            ]
            .into()
        };
        let scene = crate::brush_cursor::overlay(
            scene,
            crate::brush_stroke::config(app),
            Target::Quad(QuadPane::Isometric),
            Length::Fill,
        );
        let scene = crate::minimap::observe(
            app,
            Target::Quad(QuadPane::Isometric),
            scene,
            annotation_projection.clone(),
            Length::Fill,
        );
        let scene =
            crate::chunk_outline::overlay(app, scene, annotation_projection.clone(), Length::Fill);
        let scene = if let Some(layers) = quad_layers.clone() {
            iced::widget::stack![
                scene,
                canvas(layers.overlay(annotation_projection.clone()))
                    .width(Length::Fill)
                    .height(Length::Fill)
            ]
            .into()
        } else {
            scene
        };
        let scene = crate::view_compass::overlay(scene, annotation_projection, Length::Fill);
        let scene = crate::sky_background::wrap(scene, Length::Fill, 1.);
        let projection = drop_surface.clone();
        let scene = crate::library::scene_target(
            app,
            Target::Quad(QuadPane::Isometric),
            scene.map(scene_message),
            move |point, size| {
                let hit = drop_surface.pick_face(point, size);
                Some(crate::brush_stroke::InputFrame::surface(
                    Arc::clone(&drop_surface.surface),
                    Target::Quad(QuadPane::Isometric),
                    hit,
                ))
            },
            move |size| projection.selection_view(size),
        );
        let scene = crate::navigation_bindings::wrap(app, Target::Quad(QuadPane::Isometric), scene);

        let title = |label, pane: QuadPane| {
            column![
                row![
                    text(label),
                    text(format!("{:.2}×", panel.navigation[pane.index()].zoom))
                ]
                .spacing(8),
                crate::view_navigation::controls(Target::Quad(pane)).wrap()
            ]
            .spacing(4)
        };
        body = body.push(
            row![
                column![
                    title("水平 X/Z（滚轮缩放，中键平移）", QuadPane::Horizontal),
                    map(
                        Plane::Horizontal(&snapshot.horizontal),
                        0,
                        QuadPane::Horizontal
                    )
                ]
                .width(Length::Fill)
                .height(Length::Fill),
                column![title("等距 3D（右键旋转）", QuadPane::Isometric), scene]
                    .width(Length::Fill)
                    .height(Length::Fill)
            ]
            .spacing(8)
            .height(Length::FillPortion(1)),
        );
        body = body.push(
            row![
                column![
                    title("正面 X/Y", QuadPane::Front),
                    map(Plane::Vertical(&snapshot.front), 1, QuadPane::Front)
                ]
                .width(Length::Fill)
                .height(Length::Fill),
                column![
                    title("侧面 Z/Y", QuadPane::Side),
                    map(Plane::Vertical(&snapshot.side), 2, QuadPane::Side)
                ]
                .width(Length::Fill)
                .height(Length::Fill)
            ]
            .spacing(8)
            .height(Length::FillPortion(1)),
        );
    } else {
        let (label, action) = if app.quad.center.is_empty() {
            (
                app.language.pick(
                    "以当前坐标读取四视图",
                    "Read four views at current coordinates",
                ),
                Action::UseSelection,
            )
        } else {
            (
                app.language.pick("读取四视图", "Read four views"),
                Action::Load,
            )
        };
        body = body.push(
            container(button(label).on_press(Message::Quad(action)))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        );
    }
    body.width(Length::Fill).height(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_preserve_negative_coordinates_and_refuse_overflow() {
        let b = bounds([-80, 0, 32]).unwrap();
        assert_eq!(b.minimum, [-95, -15, 17]);
        assert_eq!(b.maximum, [-64, 16, 48]);
        assert_eq!(b.volume().unwrap(), 32 * 32 * 32);
        assert!(bounds([i32::MAX, 0, 0]).is_err());
        assert!(bounds([0, -2040, 0]).is_err());
    }

    #[test]
    fn linked_source_reload_preserves_four_independent_viewports_and_orbit() {
        use crate::view_navigation::{Action as NavAction, QuadPane, Target};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../test_files/AnvilWorld_1.8");
        let world = mcedit2_world::inspect_world(root).unwrap();
        let session = Session::new(world.clone(), world.dimensions[0].name.clone()).unwrap();
        let snapshot = read(
            &session,
            [-80, 16, 32],
            None,
            false,
            Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut app = App {
            current: Some(0),
            worlds: vec![world],
            workspace: {
                let mut state = crate::workspace_tabs::State::default();
                state.open(0);
                state
            },
            ..Default::default()
        };
        let _ = update(&mut app, Action::Loaded(0, (0, 0), Ok(Box::new(snapshot))));
        for (i, pane) in [
            QuadPane::Horizontal,
            QuadPane::Front,
            QuadPane::Side,
            QuadPane::Isometric,
        ]
        .into_iter()
        .enumerate()
        {
            app.navigate(Target::Quad(pane), NavAction::centered_zoom(i as f32 + 1.));
            app.navigate(
                Target::Quad(pane),
                NavAction::Pan(iced::Vector::new(i as f32 * 12., -8.)),
            );
        }
        let _ = update(
            &mut app,
            Action::Scene(Box::new(Message::OrbitPitchSurface(45, 20))),
        );
        let previous = app.quad.navigation;
        let orbit = (app.quad.yaw, app.quad.pitch);
        let snapshot = read(
            &session,
            [-79, 16, 32],
            None,
            false,
            Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let _ = update(&mut app, Action::Loaded(0, (0, 0), Ok(Box::new(snapshot))));
        assert_eq!(app.quad.snapshot.as_ref().unwrap().center, [-79, 16, 32]);
        assert_eq!(app.quad.navigation, previous);
        assert_eq!((app.quad.yaw, app.quad.pitch), orbit);
        app.navigate(Target::Quad(QuadPane::Front), NavAction::Reset);
        for (i, actual) in app.quad.navigation.iter().enumerate() {
            if i == QuadPane::Front.index() {
                assert_eq!(*actual, Default::default());
            } else {
                assert_eq!(*actual, previous[i]);
            }
        }
        assert_eq!((app.quad.yaw, app.quad.pitch), orbit);
        app.navigate(Target::Quad(QuadPane::Isometric), NavAction::Reset);
        assert_eq!((app.quad.yaw, app.quad.pitch), (0, 30));
        assert_eq!(
            app.quad.navigation[QuadPane::Isometric.index()],
            Default::default()
        );
    }

    #[test]
    fn drafts_failed_loads_and_stale_results_keep_the_last_valid_quad_snapshot() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../test_files/AnvilWorld_1.8");
        let world = mcedit2_world::inspect_world(root).unwrap();
        let session = Session::new(world.clone(), world.dimensions[0].name.clone()).unwrap();
        let snapshot = read(
            &session,
            [-80, 16, 32],
            None,
            false,
            Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let mut app = App {
            current: Some(0),
            worlds: vec![world],
            workspace: {
                let mut state = crate::workspace_tabs::State::default();
                state.open(0);
                state
            },
            ..Default::default()
        };
        let _ = update(&mut app, Action::Loaded(0, (0, 0), Ok(Box::new(snapshot))));
        let source = Arc::clone(&app.quad.snapshot.as_ref().unwrap().surface);
        let old_nonce = app.quad.nonce;
        let _ = update(&mut app, Action::Center("无效草稿".into()));
        let _ = update(&mut app, Action::Load);
        assert!(Arc::ptr_eq(
            &source,
            &app.quad.snapshot.as_ref().unwrap().surface
        ));
        assert_eq!(app.quad.key, Some((0, 0)));
        let snapshot = read(
            &session,
            [-79, 16, 32],
            None,
            false,
            Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let _ = update(
            &mut app,
            Action::Loaded(old_nonce, (0, 0), Ok(Box::new(snapshot.clone()))),
        );
        assert!(Arc::ptr_eq(
            &source,
            &app.quad.snapshot.as_ref().unwrap().surface
        ));
        let _ = update(&mut app, Action::Center("-79,16,32".into()));
        let _ = update(&mut app, Action::Load);
        assert!(app.quad.busy);
        assert!(Arc::ptr_eq(
            &source,
            &app.quad.snapshot.as_ref().unwrap().surface
        ));
        let nonce = app.quad.nonce;
        let _ = update(
            &mut app,
            Action::Loaded(nonce, (0, 0), Ok(Box::new(snapshot))),
        );
        assert!(!app.quad.busy);
        assert_eq!(app.quad.snapshot.as_ref().unwrap().center, [-79, 16, 32]);
        assert!(!Arc::ptr_eq(
            &source,
            &app.quad.snapshot.as_ref().unwrap().surface
        ));
    }

    #[test]
    fn real_source_planes_share_center_and_stale_load_cannot_publish() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../test_files/AnvilWorld_1.8");
        let world = mcedit2_world::inspect_world(root).unwrap();
        let session = Session::new(world.clone(), world.dimensions[0].name.clone()).unwrap();
        let snapshot = read(
            &session,
            [-80, 16, 32],
            None,
            false,
            Default::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        let horizontal = &snapshot.horizontal.rows[15][15];
        assert_eq!(snapshot.front.coordinate(16, 15), Some((-80, 16, 32)));
        assert_eq!(snapshot.side.coordinate(16, 15), Some((-80, 16, 32)));
        assert_eq!(
            format!("{horizontal:?}"),
            format!("{:?}", snapshot.front.rows[16][15])
        );
        assert_eq!(
            format!("{horizontal:?}"),
            format!("{:?}", snapshot.side.rows[16][15])
        );
        let mut app = App {
            current: Some(0),
            worlds: vec![world],
            workspace: {
                let mut state = crate::workspace_tabs::State::default();
                state.open(0);
                state
            },
            ..Default::default()
        };
        app.quad.nonce = 2;
        let _ = update(&mut app, Action::Loaded(1, (0, 0), Ok(Box::new(snapshot))));
        assert!(app.quad.snapshot.is_none());
    }
}

/// 四视图只重定向自身导航与源点选；笔画交回共同编辑事务。
pub(crate) fn scene_message(message: Message) -> Message {
    match message {
        Message::Brush(..)
        | Message::Navigate(..)
        | Message::ProjectedEntitySelected(..)
        | Message::SceneMarkerSelected(..) => message,
        other => Message::Quad(Action::Scene(Box::new(other))),
    }
}
