//! 独立结构的资源场景与局部拾取，复用既有网格、GPU、材质及相机实现。
use crate::{
    gpu_mesh::Quality,
    pending_import,
    spectator::{self, Camera, SpectatorScene},
    structure_workspace::Action,
    textures::TextureBank,
    Message,
};
use iced::{
    keyboard, mouse,
    widget::canvas::{self, Frame, Path, Stroke},
    Color, Point, Rectangle, Renderer, Theme,
};
use mcedit2_world::{
    clipboard::BlockClipboard,
    editing::{Bounds, EditWorkBudget},
    surface::SurfaceView,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone)]
pub(crate) struct Assets {
    pub bank: Arc<TextureBank>,
    pub paths: Vec<PathBuf>,
    pub data_paths: Vec<PathBuf>,
    pub definitions: Arc<mcedit2_world::world_data::Snapshot>,
    pub plugin_identity: Vec<(PathBuf, String)>,
}
impl std::fmt::Debug for Assets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("结构资源")
            .field("paths", &self.paths)
            .field("data_paths", &self.data_paths)
            .finish()
    }
}
pub(crate) fn load(
    source: &BlockClipboard,
    path: &std::path::Path,
    input: &str,
    data_input: &str,
    budget: crate::resource_budget::ResourceBudget,
    cancelled: Arc<AtomicBool>,
    block_definitions: Arc<Vec<crate::block_definitions::Binding>>,
    plugin_sources: Vec<mcedit2_plugins::catalog::PluginSource>,
    configured: Vec<PathBuf>,
) -> Result<Assets, String> {
    let paths = if input.trim().is_empty() {
        let release = source
            .source_release
            .as_deref()
            .or(match source.data_version {
                Some(1343) => Some("1.12.2"),
                Some(3465) => Some("1.20.1"),
                Some(3955) => Some("1.21.1"),
                _ => None,
            })
            .ok_or("结构没有可用于查找客户端的版本声明，请选择实际客户端和资源包")?;
        vec![crate::auto_assets::discover(path, release, &configured)?
            .ok_or_else(|| format!("没有找到 {release} 客户端，请选择实际客户端和资源包"))?]
    } else {
        input
            .split(';')
            .map(|path| PathBuf::from(path.trim()))
            .collect()
    };
    let data_paths = if data_input.trim().is_empty() {
        Vec::new()
    } else {
        data_input
            .split(';')
            .map(|path| PathBuf::from(path.trim()))
            .collect::<Vec<_>>()
    };
    let world_data =
        mcedit2_world::world_data::Snapshot::read_packs(&data_paths, source.data_version)
            .map_err(|error| error.to_string())?;
    let control = crate::textures::LoadControl {
        block_definitions,
        budget: Some(budget),
        world_data: Arc::clone(&world_data),
        cancelled,
        progress: Arc::new(|message| log::info!("[structure-assets] {message}")),
        ..Default::default()
    };
    let mut effective_paths = paths.clone();
    let mut plugins = Vec::new();
    for plugin in &plugin_sources {
        if control.cancelled.load(Ordering::Acquire) {
            return Err("结构插件资源加载已取消".into());
        }
        plugin.verify_unchanged()?;
        let resources = plugin.resources()?.ok_or("已启用插件的资源声明已经消失")?;
        effective_paths.push(resources.root.clone());
        plugins.push((plugin, resources));
    }
    let mut bank = TextureBank::open_stack_java_controlled(&effective_paths, &control)?;
    if let Some(version) = source.data_version {
        if bank.item_data_version() != Some(version) {
            return Err(format!(
                "客户端资源版本 {:?} 与结构 DataVersion {version} 不同；请选择对应客户端作为第一层",
                bank.item_data_version()
            ));
        }
    }
    bank.apply_registry_data(Arc::clone(&world_data), source.data_version, &control)?;
    let mut plugin_identity = Vec::new();
    for (plugin, expected) in plugins {
        if control.cancelled.load(Ordering::Acquire) {
            return Err("结构插件资源加载已取消".into());
        }
        plugin.verify_unchanged()?;
        let current = plugin.resources()?.ok_or("插件资源声明在加载期间消失")?;
        if current.root != expected.root || current.sha256 != expected.sha256 {
            return Err("插件资源在结构加载期间发生变化，请重新加载".into());
        }
        plugin_identity.push((expected.root, expected.sha256));
    }
    Ok(Assets {
        bank: Arc::new(bank),
        paths,
        data_paths,
        definitions: world_data,
        plugin_identity,
    })
}
#[derive(Clone)]
pub(crate) struct Preview {
    pub source: Arc<BlockClipboard>,
    pub scene: SpectatorScene,
    pub frame: Arc<pending_import::Frame>,
    views: [spectator::PresentedViewCache; 5],
    presented: [Arc<Mutex<Option<Arc<spectator::StreamMesh>>>>; 5],
}
impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("结构场景")
            .field("size", &self.source.size)
            .field("diagnostics", &self.frame.diagnostics)
            .finish()
    }
}
impl Preview {
    pub fn is_current(&self, source: &Arc<BlockClipboard>) -> bool {
        Arc::ptr_eq(&self.source, source)
    }
    pub fn shown(&self, pane: usize, size: iced::Size) -> Option<spectator::PresentedView> {
        let frame = self.views[pane].lock().expect("结构呈现锁").clone()?;
        (Arc::ptr_eq(&frame.mesh, &self.frame.mesh) && frame.viewport.logical == size)
            .then_some(frame)
    }
    pub fn drop_hit(
        &self,
        pane: usize,
        camera: Camera,
        point: Point,
        size: iced::Size,
    ) -> Option<crate::library::structure::Hit> {
        let shown = self.shown(pane, size)?;
        if shown.camera != camera.into() {
            return None;
        }
        let hit = shown.pick(point)?;
        Some(crate::library::structure::Hit {
            position: hit.owner?.position,
            face: hit.face,
        })
    }
    pub fn view(&self, camera: Camera, pane: usize) -> SpectatorScene {
        let mut scene = self.scene.clone();
        scene.camera = camera;
        scene.presented_view = Arc::clone(&self.views[pane]);
        scene.presented = Arc::clone(&self.presented[pane]);
        scene
    }
}
pub(crate) struct State {
    pub mode: crate::workspace_ui::View,
    pub cameras: [Camera; 5],
    pub paths: String,
    pub data_paths: String,
    pub report: String,
    pub assets: Option<Assets>,
    pub assets_epoch: u64,
    pub assets_busy: bool,
    pub assets_cancel: Arc<AtomicBool>,
    pub picking: bool,
    pub preview: Option<Arc<Preview>>,
    pub epoch: u64,
    pub busy: bool,
    pub cancel: Arc<AtomicBool>,
    pub last_frame: Option<std::time::Instant>,
    pub navigation: Arc<Mutex<Navigation>>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mode: crate::workspace_ui::View::Perspective,
            cameras: [Camera::default(); 5],
            paths: String::new(),
            data_paths: String::new(),
            report: String::new(),
            assets: None,
            assets_epoch: 0,
            assets_busy: false,
            assets_cancel: Arc::new(AtomicBool::new(false)),
            picking: false,
            preview: None,
            epoch: 0,
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            last_frame: None,
            navigation: Default::default(),
        }
    }
}
#[derive(Clone)]
struct Pointer {
    source: Arc<BlockClipboard>,
    epoch: u64,
    view: View,
    target: crate::view_navigation::Target,
    anchor: Point,
    size: iced::Size,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    Scene(usize),
    Slice,
}
struct Held {
    pointer: Pointer,
    physical: keyboard::key::Physical,
    key: keyboard::Key,
    command: crate::navigation_bindings::Command,
    pressed: std::time::Instant,
}
#[derive(Default)]
pub(crate) struct Navigation {
    pointer: Option<Pointer>,
    held: Option<Held>,
}
impl Navigation {
    pub fn point(
        &mut self,
        view: View,
        source: &Arc<BlockClipboard>,
        epoch: u64,
        target: crate::view_navigation::Target,
        anchor: Option<Point>,
        size: iced::Size,
    ) {
        if let Some(anchor) = anchor {
            self.pointer = Some(Pointer {
                source: Arc::clone(source),
                epoch,
                view,
                target,
                anchor,
                size,
            });
        } else if self
            .pointer
            .as_ref()
            .is_some_and(|pointer| pointer.view == view)
        {
            self.pointer = None;
        }
    }
    pub fn sync(&mut self, active: bool, source: Option<&Arc<BlockClipboard>>, epoch: u64) {
        let current = |pointer: &Pointer| {
            active
                && pointer.epoch == epoch
                && source.is_some_and(|source| Arc::ptr_eq(source, &pointer.source))
        };
        if self
            .pointer
            .as_ref()
            .is_some_and(|pointer| !current(pointer))
        {
            self.pointer = None;
        }
        if self
            .held
            .as_ref()
            .is_some_and(|held| !current(&held.pointer))
        {
            self.held = None;
        }
    }
    pub fn keyboard(
        &mut self,
        config: &crate::navigation_bindings::Config,
        event: &keyboard::Event,
        epoch: u64,
        sticky: bool,
    ) -> Option<Option<Action>> {
        match event {
            keyboard::Event::KeyReleased {
                key, physical_key, ..
            } => {
                if self.held.as_ref().is_some_and(|held| {
                    held.physical == *physical_key
                        && (matches!(physical_key, keyboard::key::Physical::Code(_))
                            || held.key == *key)
                }) {
                    let held = self.held.take().expect("按键释放已找到所属导航手势");
                    if held.command == crate::navigation_bindings::Command::Look {
                        return Some(Some(Action::ReleaseLook(
                            held.pointer.epoch,
                            held.pressed.elapsed(),
                        )));
                    }
                }
                None
            }
            keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                repeat,
                ..
            } => {
                let pointer = self
                    .pointer
                    .as_ref()
                    .filter(|pointer| pointer.epoch == epoch)?
                    .clone();
                let binding = crate::navigation_bindings::Binding::Key(
                    crate::keyboard_bindings::Chord::event(key, *modifiers)?,
                );
                let command = config.find(pointer.target, &binding)?;
                if let Some(steps) = command.zoom() {
                    return Some(Some(match pointer.view {
                        View::Scene(pane) => Action::SceneNavigate(
                            pointer.source,
                            pane,
                            epoch,
                            command,
                            iced::Vector::ZERO,
                            steps,
                        ),
                        View::Slice => Action::Camera(
                            pointer.source,
                            epoch,
                            crate::structure_slice::Navigation::Zoom {
                                steps,
                                anchor: pointer.anchor,
                                size: pointer.size,
                            },
                        ),
                    }));
                }
                if !repeat && sticky && command != crate::navigation_bindings::Command::Look {
                    return Some(Some(Action::ExitLook(epoch)));
                }
                if !repeat {
                    self.held = Some(Held {
                        pointer,
                        physical: *physical_key,
                        key: key.clone(),
                        command,
                        pressed: std::time::Instant::now(),
                    });
                }
                Some(None)
            }
            _ => None,
        }
    }
    pub(crate) fn last_view(
        &self,
        source: &Arc<BlockClipboard>,
        epoch: u64,
    ) -> Option<(View, crate::view_navigation::Target)> {
        self.pointer
            .as_ref()
            .filter(|pointer| pointer.epoch == epoch && Arc::ptr_eq(&pointer.source, source))
            .map(|pointer| (pointer.view, pointer.target))
    }
    pub fn command(
        &self,
        view: View,
        source: &Arc<BlockClipboard>,
        epoch: u64,
    ) -> Option<crate::navigation_bindings::Command> {
        self.held
            .as_ref()
            .filter(|held| {
                held.pointer.view == view
                    && held.pointer.epoch == epoch
                    && Arc::ptr_eq(&held.pointer.source, source)
            })
            .map(|held| held.command)
    }
}
impl State {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.assets_cancel.store(true, Ordering::Release);
    }
    pub fn reset(&mut self, size: [u16; 3]) {
        self.cameras[0] = fit_mode(size, self.mode);
        for (index, (yaw, pitch)) in [(225., 30.), (180., 89.5), (180., 0.), (90., 0.)]
            .into_iter()
            .enumerate()
        {
            self.cameras[index + 1] = fit_at(size, false, yaw, pitch);
        }
    }
}
pub(crate) fn fit(size: [u16; 3], perspective: bool) -> Camera {
    fit_at(size, perspective, 225., 30.)
}
pub(crate) fn fit_mode(size: [u16; 3], mode: crate::workspace_ui::View) -> Camera {
    if mode == crate::workspace_ui::View::Overhead {
        fit_at(size, false, 180., 89.5)
    } else {
        fit(size, mode == crate::workspace_ui::View::Perspective)
    }
}
fn fit_at(size: [u16; 3], perspective: bool, yaw: f32, pitch: f32) -> Camera {
    let mut camera = Camera {
        yaw,
        pitch,
        perspective,
        ..Default::default()
    };
    let (_, _, forward) = camera.basis();
    let radius = size.into_iter().map(f64::from).fold(1., f64::max);
    if !perspective {
        camera.zoom = (24. / radius as f32).clamp(crate::view_navigation::MIN_ZOOM, 1.);
    }
    let distance = if perspective {
        (radius * 2.).min(4096.)
    } else {
        0.
    };
    camera.position = std::array::from_fn(|axis| {
        (f64::from(size[axis]) - 1.) * 0.5 - f64::from(forward[axis]) * distance
    });
    camera
}
pub(crate) fn build(
    source: Arc<BlockClipboard>,
    assets: Assets,
    quality: Quality,
    language: crate::language::Language,
    budget: EditWorkBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<Preview, String> {
    if source.data_version.is_some() && source.data_version != assets.bank.item_data_version() {
        return Err("结构版本已变化，请重新加载对应客户端资源".into());
    }
    let surface = Arc::new(SurfaceView {
        x: 0,
        z: 0,
        y_min: 0,
        y_max: i32::from(source.size[1]) - 1,
        width: source.size[0],
        depth: source.size[2],
        biomes: Default::default(),
        voxels: Vec::new(),
        source_tiles: Vec::new(),
        source_chunks: 0,
        missing_columns: 0,
    });
    let mut scene = SpectatorScene {
        entity_frame: None,
        entity_enabled: true,
        entity_cache: Default::default(),
        surface,
        texture_bank: Some(assets.bank),
        show_textures: true,
        block_layers: Default::default(),
        camera: fit(source.size, true),
        quality,
        cache: Default::default(),
        presented: Default::default(),
        presented_view: Default::default(),
        pending: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Default::default(),
        lighting: crate::lighting::Table::unlit(),
    };
    let volume = pending_import::Volume {
        size: source.size,
        offset: [0; 3],
        blocks: &source.blocks,
        mask: source.mask.as_deref(),
        block_entities: &source.block_entities,
        entities: &source.entities,
        biomes: source.biomes.as_ref(),
        numeric_biomes: source.numeric_biomes.as_ref(),
        data_version: source.data_version,
        painting_variants: &source.painting_variants,
        metadata: source.metadata.is_some(),
        map_world: None,
        item_environment: None,
    };
    let frame = Arc::new(pending_import::build_document_geometry_with_budget(
        &volume, [0; 3], &scene, false, language, budget, cancelled,
    )?);
    for diagnostic in &frame.diagnostics {
        log::warn!("[structure-scene] {diagnostic}");
    }
    scene.cache = Arc::new(Mutex::new(Some(Arc::clone(&frame.mesh))));
    Ok(Preview {
        source,
        scene,
        frame,
        views: std::array::from_fn(|_| Default::default()),
        presented: std::array::from_fn(|_| Default::default()),
    })
}
pub(crate) struct Controls<'a> {
    pub id: u64,
    pub language: crate::language::Language,
    pub epoch: u64,
    pub pane: usize,
    pub preview: &'a Preview,
    pub camera: Camera,
    pub selected: Option<[i32; 3]>,
    pub flood: Option<crate::flood_fill::StructureConfig>,
    pub inspect: Option<crate::selection_handle::Config>,
    pub plugin: Option<crate::structure_workspace::plugin::Config>,
    pub brush: Option<crate::structure_workspace::brush::Config>,
    pub commands: Result<Option<crate::command_visuals::Document>, String>,
    pub handle: Option<crate::selection_handle::Layer>,
    pub selection: Option<Bounds>,
    pub interactive: bool,
    pub target: crate::view_navigation::Target,
    pub navigation: Arc<crate::navigation_bindings::Config>,
    pub flight: Arc<
        std::collections::BTreeMap<
            crate::navigation_bindings::Binding,
            crate::keyboard_bindings::Command,
        >,
    >,
    pub shared: Arc<Mutex<Navigation>>,
}
struct Drag {
    button: mouse::Button,
    command: crate::navigation_bindings::Command,
    last: Point,
}
#[derive(Default)]
pub(crate) struct ControlsState {
    identity: Option<(u64, u64)>,
    mesh: std::sync::Weak<spectator::StreamMesh>,
    viewport: Option<Rectangle>,
    drag: Option<Drag>,
    modifiers: keyboard::Modifiers,
    pointer: Option<Point>,
    brush: crate::structure_workspace::brush::Pointer,
    hover: crate::structure_workspace::hover::Scene,
}
pub(crate) fn navigate(
    camera: &mut Camera,
    command: crate::navigation_bindings::Command,
    delta: iced::Vector,
    steps: f32,
) -> Result<(), String> {
    use crate::navigation_bindings::Command;
    match command {
        Command::Look | Command::Orbit => camera.look(delta.x, delta.y),
        Command::MoveCamera => {
            camera.pan(delta)?;
        }
        Command::Pan => {
            let (right, up, _) = camera.basis();
            let scale = f64::from(std::f32::consts::SQRT_2 / (camera.zoom * 16.));
            for axis in 0..3 {
                camera.position[axis] += (-f64::from(right[axis]) * f64::from(delta.x)
                    + f64::from(up[axis]) * f64::from(delta.y))
                    * scale;
            }
        }
        Command::ZoomIn | Command::ZoomOut => {
            camera.zoom_by(steps)?;
        }
    }
    Ok(())
}
impl Controls<'_> {
    fn brush_sample(
        &self,
        point: Point,
        size: iced::Size,
    ) -> Result<Option<crate::brush_stroke::Sample>, String> {
        let Some(shown) = self
            .shown(size)
            .filter(|shown| shown.camera == self.camera.into())
        else {
            return Ok(None);
        };
        shown
            .pick_terrain(point)
            .as_ref()
            .map(crate::brush_stroke::Sample::from_hit)
            .transpose()
    }
    fn navigation(
        &self,
        command: crate::navigation_bindings::Command,
        delta: iced::Vector,
        steps: f32,
    ) -> Action {
        Action::SceneNavigate(
            Arc::clone(&self.preview.source),
            self.pane,
            self.epoch,
            command,
            delta,
            steps,
        )
    }
    fn shown(&self, size: iced::Size) -> Option<spectator::PresentedView> {
        self.preview.shown(self.pane, size)
    }
    fn pick(&self, point: Point, size: iced::Size, extend: bool) -> Option<Action> {
        let shown = self.shown(size)?;
        if shown.camera != self.camera.into() {
            return None;
        }
        let hit = if self.inspect.is_some() || self.plugin.is_some() {
            shown.pick_terrain(point)
        } else {
            shown.pick(point)
        }?;
        if let Some(config) = &self.plugin {
            let owner = hit.owner?;
            return Some(Action::Plugin(
                crate::structure_workspace::plugin::Action::Click(
                    config.clone(),
                    Arc::clone(&self.preview.source),
                    mcedit2_plugins::ToolEvent {
                        hit: owner.position,
                        state: (*owner.state).clone(),
                        view: crate::structure_workspace::plugin::view_name(self.target),
                        camera: Some(self.camera.position),
                        yaw: self.camera.yaw,
                        pitch: self.camera.pitch,
                        zoom: self.camera.zoom,
                        pan: [0.; 2],
                    },
                ),
            ));
        }
        if let Some(config) = self.inspect {
            return Some(Action::InspectCell(
                config,
                Arc::clone(&self.preview.source),
                hit.owner?.position,
            ));
        }
        if let Some(config) = self.flood {
            if !config.ready {
                return None;
            }
            let owner = hit.owner?;
            return Some(Action::FloodCell(
                config,
                Arc::clone(&self.preview.source),
                owner.position,
                hit.face,
            ));
        }
        if !extend {
            let mut object = None;
            let mut depth = f64::INFINITY;
            for part in &self.preview.frame.objects {
                if let Some(hit) = part.picking.cast(
                    shown.camera,
                    point,
                    shown.viewport,
                    shown.atlas.as_deref(),
                    shown.quality,
                    depth,
                ) {
                    if let Some(target) = part.groups[hit.group_index] {
                        depth = hit.depth;
                        object = Some(target);
                    }
                }
            }
            if depth <= hit.depth {
                if let Some(object) = object {
                    return Some(Action::SelectObject(
                        Arc::clone(&self.preview.source),
                        object,
                    ));
                }
            }
        }
        let owner = hit.owner?;
        Some(Action::Cell(
            Arc::clone(&self.preview.source),
            owner.position,
            extend,
        ))
    }
}
impl canvas::Program<Message> for Controls<'_> {
    type State = ControlsState;
    fn draw(
        &self,
        state: &ControlsState,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        if let Some(shown) = self.shown(bounds.size()) {
            state.brush.draw(
                &mut frame,
                self.brush
                    .as_ref()
                    .filter(|_| !state.modifiers.alt() && !state.modifiers.control()),
                crate::structure_workspace::brush::View::Scene(self.target),
                &self.preview.source,
                crate::selection_handle::View::World(shown.camera, bounds.size()),
                cursor.position_in(bounds),
                |point| self.brush_sample(point, bounds.size()),
            );
            crate::command_visuals::document(
                &self.commands,
                &self.preview.source,
                crate::selection_handle::View::World(shown.camera, bounds.size()),
                &mut frame,
                false,
            );
            if let Some(layer) = &self.handle {
                layer.draw(
                    crate::selection_handle::View::World(shown.camera, bounds.size()),
                    &mut frame,
                    cursor.position_in(bounds),
                );
            }
            if let crate::world_camera::WorldCamera::Free(camera) = shown.camera {
                let entire = Bounds {
                    minimum: [0; 3],
                    maximum: self.preview.source.size.map(|size| i32::from(size) - 1),
                };
                crate::spectator::controls::draw_box(
                    &mut frame,
                    camera,
                    bounds.size(),
                    entire,
                    Color::from_rgba(0.6, 0.6, 0.7, 0.6),
                );
                if let Some(selection) = self.selection {
                    crate::spectator::controls::draw_box(
                        &mut frame,
                        camera,
                        bounds.size(),
                        selection,
                        Color::from_rgb(0.1, 0.8, 1.),
                    );
                }
                if let Some(position) = self.selected {
                    crate::spectator::controls::draw_box(
                        &mut frame,
                        camera,
                        bounds.size(),
                        Bounds {
                            minimum: position,
                            maximum: position,
                        },
                        Color::WHITE,
                    );
                }
                if let Some(config) = self.flood.filter(|config| config.ready) {
                    let point = cursor.position_in(bounds).or(state
                        .pointer
                        .filter(|point| Rectangle::with_size(bounds.size()).contains(*point)));
                    if let Some(hit) = point.and_then(|point| shown.pick(point)) {
                        if let Some(owner) = hit.owner {
                            match crate::flood_fill::offset_position(
                                owner.position,
                                hit.face,
                                config.hover,
                            ) {
                                Ok(position) => crate::spectator::controls::draw_box(
                                    &mut frame,
                                    camera,
                                    bounds.size(),
                                    Bounds {
                                        minimum: position,
                                        maximum: position,
                                    },
                                    Color::from_rgb(1., 0.7, 0.1),
                                ),
                                Err(error) => log::error!("[structure-flood] {error}"),
                            }
                        }
                    }
                }
                let origin = [-0.5; 3];
                for (axis, color) in [
                    Color::from_rgb(1., 0.2, 0.2),
                    Color::from_rgb(0.2, 1., 0.2),
                    Color::from_rgb(0.2, 0.4, 1.),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut end = origin;
                    end[axis] += 3.;
                    if let Some((a, b)) = shown.camera.segment(origin, end, bounds.size()) {
                        frame.stroke(
                            &Path::line(a, b),
                            Stroke::default().with_width(2.).with_color(color),
                        );
                    }
                }
            }
        }
        if let Some(shown) = self.shown(bounds.size()) {
            let point = state.pointer.and_then(|_| cursor.position_in(bounds));
            let position = state.hover.position(&shown, point);
            crate::structure_workspace::hover::draw(
                &self.preview.source,
                position,
                self.preview.scene.texture_bank.as_deref(),
                self.language,
                &mut frame,
            );
        }
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        state: &mut ControlsState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let same_mesh = state
            .mesh
            .upgrade()
            .is_some_and(|mesh| Arc::ptr_eq(&mesh, &self.preview.frame.mesh));
        if state.identity != Some((self.id, self.epoch))
            || !same_mesh
            || state.viewport != Some(bounds)
        {
            if let Some(layer) = &self.handle {
                layer.cancel();
            }
            state.viewport = Some(bounds);
            state.identity = Some((self.id, self.epoch));
            state.mesh = Arc::downgrade(&self.preview.frame.mesh);
            state.drag = None;
            state.pointer = None;
            state.brush.cancel();
        }
        let publish = |action| {
            canvas::Action::publish(Message::StructureWorkspace(self.id, action)).and_capture()
        };
        if let canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            state.modifiers = *modifiers;
        }
        if matches!(event, canvas::Event::Window(iced::window::Event::Unfocused)) {
            state.drag = None;
            state.pointer = None;
            state.modifiers = keyboard::Modifiers::default();
        }
        let previous = state.pointer;
        if let canvas::Event::Mouse(mouse::Event::CursorMoved { position }) = event {
            state.pointer = Some(Point::new(position.x - bounds.x, position.y - bounds.y));
        }
        let point = state
            .pointer
            .or_else(|| cursor.position_in(bounds))
            .filter(|point| Rectangle::with_size(bounds.size()).contains(*point));
        if matches!(event, canvas::Event::Mouse(_)) {
            let mut shared = self.shared.lock().expect("结构导航输入锁");
            shared.point(
                View::Scene(self.pane),
                &self.preview.source,
                self.epoch,
                self.target,
                point,
                bounds.size(),
            );
        }
        let navigating = state.drag.is_some()
            || self
                .shared
                .lock()
                .expect("结构导航输入锁")
                .command(View::Scene(self.pane), &self.preview.source, self.epoch)
                .is_some();
        if let Some(action) = state.brush.event(
            self.id,
            self.brush.as_ref(),
            crate::structure_workspace::brush::View::Scene(self.target),
            &self.preview.source,
            event,
            point,
            state.modifiers,
            navigating,
            |point| self.brush_sample(point, bounds.size()),
        ) {
            return Some(action);
        }
        if let Some(layer) = &self.handle {
            let navigating = state.drag.is_some()
                || self
                    .shared
                    .lock()
                    .expect("结构导航输入锁")
                    .command(View::Scene(self.pane), &self.preview.source, self.epoch)
                    .is_some();
            if navigating
                || matches!(event, canvas::Event::Mouse(mouse::Event::ButtonPressed(button)) if *button != mouse::Button::Left)
                || matches!(
                    event,
                    canvas::Event::Mouse(mouse::Event::WheelScrolled { .. })
                )
            {
                layer.cancel();
            } else if let Some(action) = layer.event(
                self.target,
                self.shown(bounds.size())
                    .map(|shown| crate::selection_handle::View::World(shown.camera, bounds.size())),
                event,
                bounds,
                cursor,
                |point| {
                    Some(crate::structure_workspace::selection::input(
                        self.id,
                        Arc::clone(&self.preview.source),
                        self.brush_sample(point, bounds.size()),
                    ))
                },
            ) {
                return Some(action);
            }
        }
        match event {
            canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
                None
            }
            canvas::Event::Window(iced::window::Event::Unfocused) => {
                state.drag = None;
                state.pointer = None;
                state.modifiers = keyboard::Modifiers::default();
                Some(canvas::Action::request_redraw())
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(button))
                if *button != mouse::Button::Left && point.is_some() =>
            {
                use crate::navigation_bindings::{Binding, Button};
                let binding = Binding::Mouse(
                    Button::read(*button).expect("导航只接收非左键"),
                    crate::keyboard_bindings::modifiers(state.modifiers),
                );
                if self.target == crate::view_navigation::Target::Perspective {
                    if let Some(&command) = self.flight.get(&binding) {
                        return Some(
                            canvas::Action::publish(Message::KeyboardBindings(
                                crate::keyboard_bindings::Action::MouseDown(
                                    *button,
                                    command,
                                    self.epoch,
                                    Some(crate::keyboard_bindings::Owner::Structure(self.id)),
                                ),
                            ))
                            .and_capture(),
                        );
                    }
                }
                if let Some(command) = self.navigation.find(self.target, &binding) {
                    if let Some(steps) = command.zoom() {
                        return Some(publish(self.navigation(command, iced::Vector::ZERO, steps)));
                    }
                    state.drag = Some(Drag {
                        button: *button,
                        command,
                        last: point.expect("导航起点位于结构画布内"),
                    });
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(button))
                if *button != mouse::Button::Left =>
            {
                if state
                    .drag
                    .as_ref()
                    .is_some_and(|drag| drag.button == *button)
                {
                    state.drag = None;
                }
                Some(canvas::Action::publish(Message::KeyboardBindings(
                    crate::keyboard_bindings::Action::MouseUp(*button),
                )))
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let next = Point::new(position.x - bounds.x, position.y - bounds.y);
                if let Some(drag) = &mut state.drag {
                    let delta = next - drag.last;
                    drag.last = next;
                    return Some(publish(self.navigation(drag.command, delta, 0.)));
                }
                let command = self.shared.lock().expect("结构导航输入锁").command(
                    View::Scene(self.pane),
                    &self.preview.source,
                    self.epoch,
                );
                if let (Some(previous), Some(command)) = (previous, command) {
                    return Some(publish(self.navigation(command, next - previous, 0.)));
                }
                if point.is_some()
                    || previous
                        .is_some_and(|point| Rectangle::with_size(bounds.size()).contains(point))
                    || self.flood.is_some()
                    || self.handle.is_some()
                    || self.brush.is_some()
                {
                    return Some(canvas::Action::request_redraw());
                }
                None
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) if point.is_some() => {
                let steps = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.,
                };
                if steps == 0. {
                    return None;
                }
                let binding = crate::navigation_bindings::Binding::Wheel(
                    steps > 0.,
                    crate::keyboard_bindings::modifiers(state.modifiers),
                );
                self.navigation.find(self.target, &binding).map(|command| {
                    publish(self.navigation(
                        command,
                        iced::Vector::ZERO,
                        command.zoom().expect("滚轮只绑定缩放") * steps.abs(),
                    ))
                })
            }
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.interactive =>
            {
                self.pick(point?, bounds.size(), state.modifiers.shift())
                    .map(publish)
            }
            _ => None,
        }
    }
    fn mouse_interaction(
        &self,
        state: &ControlsState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(interaction) = self.handle.as_ref().and_then(|layer| {
            layer.interaction(
                self.shown(bounds.size())
                    .map(|shown| crate::selection_handle::View::World(shown.camera, bounds.size())),
                cursor.position_in(bounds),
            )
        }) {
            return interaction;
        }
        if state.drag.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}
