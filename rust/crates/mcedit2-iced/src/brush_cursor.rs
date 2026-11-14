//! 真实材质的工具光标；后台只生成显示几何，不暂存方块，也不发布拾取来源。
use crate::{
    brush_stroke::{Config, Mode, Sample, Source},
    gpu_mesh::{Quality, Vertex},
    projected_entities::Mapping,
    spectator::{SpectatorScene, StreamMesh},
    textures::TextureBank,
    view_navigation::Target,
    world_camera::WorldCamera,
    App, Message,
};
use iced::{mouse, widget::shader, Element, Length, Rectangle, Size};
use mcedit2_world::{
    editing::{parse_state, BiomeTarget, Bounds, Session},
    surface::SurfaceView,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

fn slot(target: Target) -> usize {
    match target {
        Target::Perspective => 0,
        Target::Isometric => 1,
        Target::TopDown => 2,
        Target::Horizontal => 3,
        Target::Vertical => 4,
        Target::Quad(p) => 5 + p.index(),
    }
}
type CachedSource = Option<((usize, usize), Arc<()>, Arc<Session>)>;
pub(crate) struct Shared {
    source: Mutex<CachedSource>,
    slots: [Arc<Cache>; 9],
}
impl Default for Shared {
    fn default() -> Self {
        Self {
            source: Mutex::new(None),
            slots: std::array::from_fn(|_| Arc::new(Cache::default())),
        }
    }
}
impl Shared {
    pub fn clear(&self) {
        *self.source.lock().expect("光标会话锁被毒化") = None;
        for cache in &self.slots {
            cache.clear();
        }
    }
}
#[derive(Clone)]
pub(crate) struct Context {
    shared: Arc<Shared>,
    session: Arc<Session>,
    bank: Option<Arc<TextureBank>>,
    show: bool,
    quality: Quality,
    language: crate::language::Language,
}
impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CursorContext")
            .field("show_textures", &self.show)
            .finish()
    }
}
fn same_bank(a: &Option<Arc<TextureBank>>, b: &Option<Arc<TextureBank>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}
pub(crate) fn context(app: &App) -> Option<Context> {
    let key = app.current_key()?;
    let session = app.sessions.get(&key)?;
    let revision = session.revision();
    let shared = &app.block_edit.cursor;
    let mut source = shared.source.lock().expect("光标源缓存锁被毒化");
    if source
        .as_ref()
        .is_none_or(|(old, version, _)| *old != key || !Arc::ptr_eq(version, &revision))
    {
        *source = Some((key, revision, Arc::new(session.clone())));
    }
    Some(Context {
        shared: Arc::clone(shared),
        session: Arc::clone(&source.as_ref().expect("光标会话已缓存").2),
        bank: app.scene_texture_bank().cloned(),
        show: app.scene_show_textures(),
        quality: app.gpu_quality,
        language: app.language,
    })
}
pub(crate) fn with_assets(
    mut config: Option<Config>,
    bank: Option<Arc<TextureBank>>,
    show: bool,
) -> Option<Config> {
    if let Some(context) = config.as_mut().and_then(|c| c.visual.as_mut()) {
        context.bank = bank;
        context.show = show;
    }
    config
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Projection {
    World(WorldCamera),
    Planar(Mapping),
}
impl From<WorldCamera> for Projection {
    fn from(camera: WorldCamera) -> Self {
        match camera {
            WorldCamera::Orthographic(p) => Self::Planar(Mapping::Isometric(p)),
            _ => Self::World(camera),
        }
    }
}
#[derive(Clone)]
struct Key {
    config: Config,
    source: Source,
    bounds: Bounds,
    numeric: Option<bool>,
    session: Arc<Session>,
    bank: Option<Arc<TextureBank>>,
    show: bool,
    quality: Quality,
    language: crate::language::Language,
}
impl Key {
    fn same(&self, other: &Self) -> bool {
        self.config == other.config
            && self.source.same(&other.source)
            && self.bounds == other.bounds
            && self.numeric == other.numeric
            && Arc::ptr_eq(&self.session, &other.session)
            && same_bank(&self.bank, &other.bank)
            && self.show == other.show
            && self.quality == other.quality
            && self.language == other.language
    }
}
struct Request {
    key: Key,
    numeric: Option<bool>,
    token: Arc<AtomicBool>,
}
#[derive(Default)]
struct State {
    key: Option<Key>,
    queued: Option<Request>,
    token: Arc<AtomicBool>,
    running: bool,
    ready: Option<Arc<StreamMesh>>,
    projection: Option<(Projection, Size)>,
    error: Option<String>,
}
#[derive(Default)]
struct Cache {
    state: Mutex<State>,
}
impl Cache {
    fn clear(&self) {
        let mut s = self.state.lock().expect("光标缓存锁被毒化");
        s.token.store(true, Ordering::Release);
        s.key = None;
        s.queued = None;
        s.ready = None;
        s.projection = None;
        s.error = None;
    }
    fn request(
        self: &Arc<Self>,
        key: Key,
        numeric: Option<bool>,
        projection: Projection,
        size: Size,
    ) {
        let spawn = {
            let mut s = self.state.lock().expect("光标请求锁被毒化");
            s.projection = Some((projection, size));
            if s.key.as_ref().is_some_and(|old| old.same(&key)) {
                return;
            }
            s.token.store(true, Ordering::Release);
            s.token = Arc::new(AtomicBool::new(false));
            s.queued = Some(Request {
                key: key.clone(),
                numeric,
                token: Arc::clone(&s.token),
            });
            s.key = Some(key);
            s.ready = None;
            s.error = None;
            let spawn = !s.running;
            s.running = true;
            spawn
        };
        if !spawn {
            return;
        }
        let cache = Arc::clone(self);
        if let Err(error) = std::thread::Builder::new()
            .name("brush-cursor".into())
            .spawn(move || loop {
                let request = {
                    let mut s = cache.state.lock().expect("光标工作队列锁被毒化");
                    let Some(request) = s.queued.take() else {
                        s.running = false;
                        break;
                    };
                    request
                };
                let result = build(&request.key, request.numeric, &|| {
                    request.token.load(Ordering::Acquire)
                });
                let mut s = cache.state.lock().expect("光标发布锁被毒化");
                if Arc::ptr_eq(&s.token, &request.token) && !request.token.load(Ordering::Acquire) {
                    match result {
                        Ok(mesh) => s.ready = Some(Arc::new(mesh)),
                        Err(error) => {
                            log::error!("[brush-cursor] {error}");
                            s.error = Some(error);
                        }
                    }
                    crate::render_wake::notify();
                }
            })
        {
            let mut s = self.state.lock().expect("光标线程错误锁被毒化");
            s.running = false;
            s.queued = None;
            s.error = Some(format!("无法启动光标准备线程：{error}"));
            log::error!("[brush-cursor] {}", s.error.as_ref().unwrap());
        }
    }
}
/// 每次画布绘制使用同一实际命中和投影；空命中、修饰键及停用会撤回光标。
pub(crate) fn request(
    config: Option<&Config>,
    target: Target,
    source: Option<(Source, Sample)>,
    projection: Projection,
    size: Size,
) -> Option<String> {
    let context = config.and_then(|c| c.visual.as_ref())?;
    let cache = &context.shared.slots[slot(target)];
    let Some((source, sample)) = source else {
        cache.clear();
        return None;
    };
    let config = config.expect("光标已有配置");
    let bounds = match crate::brush_stroke::cursor_sample(config, &sample) {
        Ok(b) => b,
        Err(error) => {
            cache.clear();
            log::error!("[brush-cursor] {error}");
            return Some(error);
        }
    };
    let mut semantic = config.clone();
    semantic.visual = None;
    cache.request(
        Key {
            config: semantic,
            source,
            bounds,
            numeric: sample
                .owner()
                .map(|owner| owner.state.name.starts_with("numeric:")),
            session: Arc::clone(&context.session),
            bank: context.bank.clone(),
            show: context.show,
            quality: context.quality,
            language: context.language,
        },
        sample
            .owner()
            .map(|owner| owner.state.name.starts_with("numeric:")),
        projection,
        size,
    );
    let error = cache
        .state
        .lock()
        .expect("光标错误读取锁被毒化")
        .error
        .clone();
    error
}
pub(crate) fn draw(
    frame: &mut iced::widget::canvas::Frame,
    config: Option<&Config>,
    target: Target,
    input: Option<crate::brush_stroke::InputFrame>,
    projection: Projection,
    size: Size,
) {
    let source = input.and_then(|input| match input.sample {
        Ok(sample) => sample.map(|s| (input.source, s)),
        Err(error) => {
            log::error!("[brush-cursor] {error}");
            frame.fill_text(iced::widget::canvas::Text {
                content: error,
                color: iced::Color::from_rgb(1., 0.35, 0.35),
                ..Default::default()
            });
            None
        }
    });
    if let Some(error) = request(config, target, source, projection, size) {
        frame.fill_text(iced::widget::canvas::Text {
            content: format!("光标材质：{error}"),
            color: iced::Color::from_rgb(1., 0.35, 0.35),
            ..Default::default()
        });
    }
}
fn build(
    key: &Key,
    numeric: Option<bool>,
    cancelled: &dyn Fn() -> bool,
) -> Result<StreamMesh, String> {
    let numeric = match numeric {
        Some(numeric) => numeric,
        None => {
            let mut reader = key.session.source_reader().map_err(|e| e.to_string())?;
            let [x, _, z] = key.bounds.minimum;
            let (chunk, _) = key
                .session
                .visible_chunk_base_shared_cached(&mut reader, x.div_euclid(16), z.div_euclid(16))
                .map_err(|e| e.to_string())?;
            let chunk = chunk.ok_or("工作平面光标目标没有实际源区块；请先创建区块")?;
            matches!(
                chunk.layout,
                mcedit2_world::sections::Layout::McRegion128
                    | mcedit2_world::sections::Layout::NumericAnvil112
            )
        }
    };
    let size = key.config.dimensions()?;
    let budget = key.config.work_budget()?;
    let count = size.into_iter().map(usize::from).product::<usize>();
    let target = match key.config.mode {
        Mode::Fill => parse_state(&key.config.target).map_err(|e| e.to_string())?,
        Mode::Replace => {
            let mcedit2_world::editing::BrushPaint::Replace(rules) = key.config.paint()? else {
                unreachable!()
            };
            rules[0].target.clone()
        }
        Mode::Biome => parse_state(if numeric {
            "numeric:2[data=0]"
        } else {
            "minecraft:grass_block[snowy=false]"
        })
        .map_err(|e| e.to_string())?,
    };
    if target.name.starts_with("numeric:") != numeric {
        return Err("光标目标与真实源方块的数字 / 命名布局不同".into());
    }
    let state_bytes = std::mem::size_of::<mcedit2_world::sections::State>()
        + target.name.len()
        + target
            .properties
            .iter()
            .map(|(a, b)| std::mem::size_of::<(String, String)>() + a.len() + b.len())
            .sum::<usize>();
    budget
        .check(
            count
                .checked_mul((state_bytes + 1) * 4)
                .ok_or("光标工作集大小溢出")?,
        )
        .map_err(|e| e.to_string())?;
    let cells = key
        .config
        .mask()
        .select(std::iter::once(Ok(key.bounds)), budget, cancelled, |_| {})
        .map_err(|e| e.to_string())?;
    let [x0, y0, z0] = key.bounds.minimum;
    let mask = (0..size[1])
        .flat_map(|y| (0..size[2]).flat_map(move |z| (0..size[0]).map(move |x| (x, y, z))))
        .map(|(x, y, z)| cells.contains((x0 + i32::from(x), y0 + i32::from(y), z0 + i32::from(z))))
        .collect::<Vec<_>>();
    // 群系光标使用独立候选，保留实际整列 / 四格单元、版本和混色邻域。
    let mut candidate;
    let session = if key.config.mode == Mode::Biome {
        candidate = (*key.session).clone();
        if let Some(paint) = candidate
            .plan_biome_selection_cancellable(
                cells.clone(),
                &BiomeTarget::parse(&key.config.target).map_err(|e| e.to_string())?,
                budget,
                cancelled,
                |_| {},
            )
            .map_err(|e| e.to_string())?
        {
            candidate
                .commit_structure_cancellable(paint.into_structure(), cancelled)
                .map_err(|e| e.to_string())?;
        }
        &candidate
    } else {
        &key.session
    };
    let colors = mcedit2_world::surface::inspect_biome_region(session, key.bounds, cancelled)
        .map_err(|e| e.to_string())?;
    let version = session.original_world().data_version;
    let surface = Arc::new(SurfaceView {
        biomes: colors,
        x: x0,
        z: z0,
        y_min: y0,
        y_max: key.bounds.maximum[1],
        width: size[0],
        depth: size[2],
        voxels: Vec::new(),
        source_tiles: Vec::new(),
        source_chunks: 0,
        missing_columns: 0,
    });
    let scene = SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface,
        texture_bank: key.bank.clone(),
        show_textures: key.show,
        block_layers: Default::default(),
        camera: Default::default(),
        quality: key.quality,
        cache: Default::default(),
        presented: Default::default(),
        presented_view: Default::default(),
        pending: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Default::default(),
        lighting: crate::lighting::Table::unlit(),
    };
    let blocks = vec![target; count];
    let volume = crate::pending_import::Volume {
        size,
        offset: [0; 3],
        blocks: &blocks,
        mask: Some(&mask),
        block_entities: &[],
        entities: &[],
        biomes: None,
        numeric_biomes: None,
        data_version: version,
        painting_variants: &std::collections::BTreeMap::new(),
        metadata: false,
        map_world: None,
        item_environment: None,
    };
    let frame = crate::pending_import::build_geometry_with_budget(
        &volume,
        key.bounds.minimum,
        &scene,
        false,
        key.language,
        budget,
        cancelled,
    )?;
    for diagnostic in &frame.diagnostics {
        log::error!("[brush-cursor] {diagnostic}");
    }
    Ok((*frame.mesh).clone())
}
pub(crate) fn overlay<'a>(
    content: Element<'a, Message>,
    config: Option<Config>,
    target: Target,
    height: Length,
) -> Element<'a, Message> {
    let Some(context) = config.as_ref().and_then(|c| c.visual.as_ref()) else {
        return content;
    };
    let cache = Arc::clone(&context.shared.slots[slot(target)]);
    macro_rules! layer {
        ($n:literal) => {
            iced::widget::shader(Layer::<$n> {
                cache,
                quality: context.quality,
            })
            .width(Length::Fill)
            .height(height)
        };
    }
    let layer: Element<'a, Message> = match slot(target) {
        0 => layer!(0).into(),
        1 => layer!(1).into(),
        2 => layer!(2).into(),
        3 => layer!(3).into(),
        4 => layer!(4).into(),
        5 => layer!(5).into(),
        6 => layer!(6).into(),
        7 => layer!(7).into(),
        8 => layer!(8).into(),
        _ => unreachable!("九个既定光标视口"),
    };
    iced::widget::stack![content, layer].into()
}
struct Layer<const N: usize> {
    cache: Arc<Cache>,
    quality: Quality,
}
#[derive(Default)]
pub(crate) struct LayerState {
    tick: Option<std::time::Instant>,
    projected: Mutex<Option<(Arc<StreamMesh>, Mapping, Size, Arc<Vec<Vertex>>)>>,
}
/// 每个视口使用独立 primitive 类型，避免 Iced 的共享流水线覆盖其他视口。
#[derive(Debug, Clone)]
pub(crate) struct CursorPrimitive<const N: usize>(pub(crate) crate::gpu_scene::Primitive);
pub(crate) struct CursorPipeline {
    pub(crate) inner: Option<crate::gpu_scene::Pipeline>,
    format: iced::wgpu::TextureFormat,
    used: bool,
}
impl shader::Pipeline for CursorPipeline {
    fn new(
        _: &iced::wgpu::Device,
        _: &iced::wgpu::Queue,
        format: iced::wgpu::TextureFormat,
    ) -> Self {
        Self {
            inner: None,
            format,
            used: false,
        }
    }
    fn trim(&mut self) {
        if !self.used {
            self.inner = None;
        }
        self.used = false;
    }
}
impl<const N: usize> shader::Primitive for CursorPrimitive<N> {
    type Pipeline = CursorPipeline;
    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        device: &iced::wgpu::Device,
        queue: &iced::wgpu::Queue,
        bounds: &Rectangle,
        viewport: &shader::Viewport,
    ) {
        pipeline.used = true;
        let inner = pipeline.inner.get_or_insert_with(|| {
            <crate::gpu_scene::Pipeline as shader::Pipeline>::new(device, queue, pipeline.format)
        });
        shader::Primitive::prepare(&self.0, inner, device, queue, bounds, viewport);
    }
    fn draw(&self, pipeline: &Self::Pipeline, pass: &mut iced::wgpu::RenderPass<'_>) -> bool {
        shader::Primitive::draw(
            &self.0,
            pipeline.inner.as_ref().expect("光标流水线已经准备"),
            pass,
        )
    }
}
impl<const N: usize> shader::Program<Message> for Layer<N> {
    type State = LayerState;
    type Primitive = CursorPrimitive<N>;
    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        _: Rectangle,
        _: mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        if let iced::Event::Window(iced::window::Event::RedrawRequested(at)) = event {
            state.tick = Some(*at);
            let s = self.cache.state.lock().expect("光标唤醒锁被毒化");
            if s.running {
                return Some(shader::Action::request_redraw_at(
                    *at + std::time::Duration::from_millis(16),
                ));
            }
            return s
                .ready
                .as_ref()
                .and_then(|mesh| mesh.atlas.as_ref())
                .and_then(|atlas| atlas.animation_deadline())
                .map(shader::Action::request_redraw_at);
        }
        None
    }
    fn draw(&self, state: &Self::State, _: mouse::Cursor, bounds: Rectangle) -> Self::Primitive {
        CursorPrimitive(self.primitive(state, bounds))
    }
}
impl<const N: usize> Layer<N> {
    fn primitive(&self, state: &LayerState, bounds: Rectangle) -> crate::gpu_scene::Primitive {
        let s = self.cache.state.lock().expect("光标显示锁被毒化");
        let frame = s
            .ready
            .as_ref()
            .zip(s.projection)
            .filter(|(_, (_, size))| *size == bounds.size());
        let Some((mesh, (projection, _))) = frame else {
            return crate::gpu_scene::cursor_primitive(
                None,
                None,
                Arc::new(Vec::new()),
                self.quality,
                state.tick,
            );
        };
        match projection {
            Projection::World(camera) => crate::gpu_scene::cursor_primitive(
                Some(Arc::clone(mesh)),
                Some(camera.uniform_at_origin(mesh.origin, bounds.size())),
                Arc::new(Vec::new()),
                self.quality,
                state.tick,
            ),
            Projection::Planar(mapping) => {
                let mut cached = state.projected.lock().expect("光标投影锁被毒化");
                if cached.as_ref().is_none_or(|(old, map, size, _)| {
                    !Arc::ptr_eq(old, mesh) || *map != mapping || *size != bounds.size()
                }) {
                    let mut vertices = Vec::new();
                    for chunk in mesh.chunks.iter() {
                        for quad in chunk.quads().iter() {
                            let quad = quad.viewed(
                                mapping.basis(),
                                std::array::from_fn(|i| {
                                    mapping.view_origin()[i] - f64::from(chunk.origin[i])
                                }),
                            );
                            for i in [0, 1, 2, 0, 2, 3] {
                                let world = std::array::from_fn(|axis| {
                                    f64::from(chunk.origin[axis]) + f64::from(quad.xyz[i][axis])
                                });
                                let (p, depth) = mapping.project_material(world, quad.mode);
                                vertices.push(Vertex {
                                    xy: [p.x, p.y],
                                    depth,
                                    rgba: quad.rgba[i],
                                    uv: quad.uv[i],
                                    mode: quad.mode,
                                    xyz: [0.; 3],
                                    world_mode: 0,
                                    atlas_page: quad.atlas_page,
                                    light: crate::lighting::FULLBRIGHT,
                                });
                            }
                        }
                    }
                    crate::gpu_scene::clip_cursor_vertices(&mut vertices, bounds.size());
                    *cached = Some((Arc::clone(mesh), mapping, bounds.size(), Arc::new(vertices)));
                }
                crate::gpu_scene::cursor_primitive(
                    Some(Arc::clone(mesh)),
                    None,
                    Arc::clone(&cached.as_ref().unwrap().3),
                    self.quality,
                    state.tick,
                )
            }
        }
    }
}

#[cfg(test)]
#[path = "brush_cursor_tests.rs"]
pub(crate) mod tests;
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    match category {
        crate::test_categories::Category::Rendering => tests::material_cursor(),
        crate::test_categories::Category::Background => tests::request_handoff(),
        _ => {}
    }
}
