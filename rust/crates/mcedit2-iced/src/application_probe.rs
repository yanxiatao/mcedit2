//! 使用实际 Iced 控件树、消息、后台任务和 wgpu 呈现的离屏应用诊断。
//! 不创建窗口、不访问剪贴板、不装载用户首选项，也不执行编辑或写盘动作。
use crate::{spectator::Camera, textures::TextureBank, App, Bounds, Message, NearMode};
use iced::futures::{
    stream::{BoxStream, SelectAll},
    FutureExt, StreamExt,
};
use iced::wgpu;
use iced::{keyboard, mouse, Color, Event, Point, Size};
use iced_runtime::{user_interface, Action, UserInterface};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

// 正常回放对齐原生窗口；旧诊断的额外校验可显式启用，保留同配置性能对照。
fn profile_instance_flags() -> wgpu::InstanceFlags {
    if std::env::var_os("MCEDIT2_PROFILE_GPU_VALIDATION").is_some() {
        wgpu::InstanceFlags::debugging()
    } else {
        iced_wgpu::instance_flags()
    }
}

#[derive(Default)]
struct Counters {
    near: usize,
    plans: usize,
    models: usize,
    look: usize,
}
struct Reveal {
    started: Instant,
    columns: HashSet<(i32, i32)>,
    submitted_ms: Option<f64>,
    visible_probe_ms: Option<f64>,
    sampled_mesh: Option<Arc<crate::spectator::StreamMesh>>,
}
#[derive(Default, serde::Serialize)]
struct ColumnTimes {
    requested_ms: f64,
    read_ms: Option<f64>,
    meshed_ms: Option<f64>,
    submitted_ms: Option<f64>,
    nonempty: bool,
    prefetched: bool,
    prepared_mesh: bool,
    prepared_gpu: bool,
}
struct FlowProbe {
    started: Instant,
    initial: HashSet<(i32, i32)>,
    columns: BTreeMap<(i32, i32), ColumnTimes>,
    cpu: Option<Arc<crate::spectator::StreamMesh>>,
    gpu: Option<Arc<crate::spectator::StreamMesh>>,
    known_cpu: HashSet<(i32, i32)>,
    known_gpu: HashSet<(i32, i32)>,
}
impl FlowProbe {
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.
    }
    fn requested(&mut self, columns: impl Iterator<Item = ((i32, i32), Option<bool>)>) {
        let now = self.now();
        for (column, loaded) in columns.filter(|(c, _)| !self.initial.contains(c)) {
            let prepared_mesh = self.known_cpu.contains(&column);
            let prepared_gpu = self.known_gpu.contains(&column);
            self.columns.entry(column).or_insert_with(|| ColumnTimes {
                requested_ms: now,
                read_ms: loaded.map(|_| now),
                nonempty: loaded.unwrap_or(false),
                prefetched: loaded.is_some(),
                prepared_mesh,
                prepared_gpu,
                meshed_ms: prepared_mesh.then_some(now),
                ..Default::default()
            });
        }
    }
    fn read(&mut self, delivery: &crate::stream_readers::Delivery) {
        let now = self.now();
        for (request, result) in &delivery.patches {
            if let Some(column) = self.columns.get_mut(&(request.cx, request.cz)) {
                if let Ok((patch, _)) = result {
                    column.read_ms.get_or_insert(now);
                    column.nonempty |= !patch.voxels.is_empty();
                }
            }
        }
    }
    fn mesh(&mut self, mesh: Option<Arc<crate::spectator::StreamMesh>>, gpu: bool) {
        let slot = if gpu { &mut self.gpu } else { &mut self.cpu };
        let Some(mesh) = mesh else {
            return;
        };
        if slot.as_ref().is_some_and(|old| Arc::ptr_eq(old, &mesh)) {
            return;
        }
        *slot = Some(Arc::clone(&mesh));
        if gpu {
            self.known_gpu = mesh
                .terrain
                .as_ref()
                .unwrap_or(&mesh)
                .chunks
                .iter()
                .filter(|p| p.key.2 == 1)
                .map(|p| (p.key.0, p.key.1))
                .collect();
        } else {
            self.known_cpu = mesh
                .chunks
                .iter()
                .filter(|p| p.key.2 == 1)
                .map(|p| (p.key.0, p.key.1))
                .collect();
        }
        let now = self.now();
        for part in mesh.chunks.iter().filter(|p| p.key.2 == 1) {
            if let Some(column) = self.columns.get_mut(&(part.key.0, part.key.1)) {
                let time = if gpu {
                    &mut column.submitted_ms
                } else {
                    &mut column.meshed_ms
                };
                time.get_or_insert(now);
            }
        }
    }
    fn report(&self, wanted: HashSet<(i32, i32)>) -> serde_json::Value {
        let stages = [
            "plan_to_read",
            "read_to_mesh",
            "mesh_to_submit",
            "plan_to_submit",
        ];
        let mut result = serde_json::Map::new();
        for (stage, name) in stages.into_iter().enumerate() {
            let mut samples = self
                .columns
                .values()
                .filter_map(|c| {
                    if !c.nonempty {
                        return None;
                    }
                    let delta = match stage {
                        0 => Some(c.read_ms? - c.requested_ms),
                        1 => Some(c.meshed_ms? - c.read_ms?),
                        2 => Some(c.submitted_ms? - c.meshed_ms?),
                        _ => Some(c.submitted_ms? - c.requested_ms),
                    }?;
                    (delta >= 0.).then_some(delta)
                })
                .collect::<Vec<_>>();
            samples.sort_by(f64::total_cmp);
            let quantile = |n: usize| {
                samples
                    .get(samples.len().saturating_sub(1) * n / 100)
                    .copied()
            };
            result.insert(name.into(), serde_json::json!({"count":samples.len(), "p50_ms":quantile(50), "p95_ms":quantile(95), "max_ms":samples.last()}));
        }
        result.insert(
            "prefetched_columns".into(),
            serde_json::json!(self
                .columns
                .values()
                .filter(|c| c.prefetched && c.nonempty)
                .count()),
        );
        result.insert(
            "unsubmitted_nonempty_in_final_range".into(),
            serde_json::json!(self
                .columns
                .iter()
                .filter(|(id, c)| wanted.contains(id) && c.nonempty && c.submitted_ms.is_none())
                .count()),
        );
        result.insert(
            "columns".into(),
            serde_json::json!(self
                .columns
                .iter()
                .map(|(column, times)| serde_json::json!({"column":column,"times":times}))
                .collect::<Vec<_>>()),
        );
        result.insert(
            "prepared_mesh_columns".into(),
            serde_json::json!(self
                .columns
                .values()
                .filter(|c| c.prepared_mesh && c.nonempty)
                .count()),
        );
        result.insert(
            "prepared_gpu_columns".into(),
            serde_json::json!(self
                .columns
                .values()
                .filter(|c| c.prepared_gpu && c.nonempty)
                .count()),
        );
        serde_json::Value::Object(result)
    }
}
struct Driver {
    app: App,
    reveal: Option<Reveal>,
    flow: Option<FlowProbe>,
    tasks: SelectAll<BoxStream<'static, Action<Message>>>,
    wakes: BoxStream<'static, ()>,
    renderer: iced::Renderer,
    cache: user_interface::Cache,
    device: wgpu::Device,
    target: wgpu::Texture,
    viewport: iced_wgpu::graphics::Viewport,
    size: Size,
    counts: Counters,
    app_ms: f64,
    adapter: String,
    in_flight: VecDeque<wgpu::SubmissionIndex>,
}
impl Driver {
    fn new(world: mcedit2_world::WorldSummary, bank: Arc<TextureBank>, radius: u16) -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            flags: profile_instance_flags(),
            ..Default::default()
        });
        let adapter = super::gpu_scene::hardware_tests::block_on_for_application(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        )
        .unwrap();
        let features = if adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION)
        {
            adapter.features()
                & (wgpu::Features::INDIRECT_FIRST_INSTANCE
                    | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = super::gpu_scene::hardware_tests::block_on_for_application(
            adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("离屏完整应用设备"),
                required_features: features,
                required_limits: wgpu::Limits {
                    max_bind_groups: 2,
                    ..Default::default()
                },
                ..Default::default()
            }),
        )
        .unwrap();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let engine = iced_wgpu::Engine::new(
            &adapter,
            device.clone(),
            queue,
            format,
            None,
            iced_wgpu::graphics::Shell::headless(),
        );
        let renderer = iced_wgpu::Renderer::new(engine, iced::Font::default(), iced::Pixels(16.));
        let physical = Size::new(1920, 1080);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("完整应用离屏画面"),
            size: wgpu::Extent3d {
                width: physical.width,
                height: physical.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let viewport = iced_wgpu::graphics::Viewport::with_physical_size(physical, 1.);
        let open_flight = std::env::var_os("MCEDIT2_PROFILE_OPEN_FLIGHT").is_some();
        let mut app = App {
            worlds: vec![world],
            workspace: {
                let mut state = crate::workspace_tabs::State::default();
                state.open(0);
                state
            },
            current: Some(0),
            readonly: true,
            texture_bank: Some(bank),
            manual_textures: true,
            show_textures: true,
            use_gpu: true,
            spectator: true,
            surface_exposed: true,
            surface_pitch: 35,
            spectator_radius: radius,
            spectator_camera: Camera {
                position: [2.5, if open_flight { 400. } else { 215.5 }, -17.5],
                yaw: 0.,
                pitch: if open_flight { 45. } else { 23. },
                zoom: 0.25,
                perspective: true,
            },
            ..Default::default()
        };
        app.entity_models.enabled = true;
        app.entity_models.online_skins = false;
        let bounds = Bounds {
            minimum: [-16, 192, -48],
            maximum: [47, 239, 15],
        };
        app.surface_area = crate::scene_selection::coordinates(bounds);
        app.enqueue_near((0, 0), NearMode::Volume, bounds, None)
            .unwrap();
        let mut result = Self {
            app,
            reveal: None,
            flow: None,
            tasks: SelectAll::new(),
            wakes: crate::render_wake::events().boxed(),
            renderer,
            cache: Default::default(),
            device,
            target,
            viewport,
            size: Size::new(physical.width as f32, physical.height as f32),
            counts: Default::default(),
            app_ms: 0.,
            adapter: format!("{:?}", adapter.get_info()),
            in_flight: VecDeque::new(),
        };
        result.message(Message::WorldMeshPresented);
        result
    }
    fn message(&mut self, message: Message) {
        if let (Some(flow), Message::SpectatorStreamBatch(_, _, delivery)) =
            (&mut self.flow, &message)
        {
            flow.read(delivery);
        }
        match &message {
            Message::NearLoaded(..) => self.counts.near += 1,
            Message::SpectatorStreamPlan(..) => self.counts.plans += 1,
            Message::EntityModelsLoaded(..) => self.counts.models += 1,
            Message::SpectatorLook(..) => self.counts.look += 1,
            _ => {}
        }
        let start = Instant::now();
        let task = crate::update(&mut self.app, message);
        if let (Some(flow), Some(stream)) = (&mut self.flow, &self.app.spectator_stream) {
            flow.requested(
                stream
                    .source_coordinates()
                    .map(|c| (c, stream.loaded_column_nonempty(c))),
            );
        }
        self.app_ms += start.elapsed().as_secs_f64() * 1000.;
        if let Some(stream) = iced_runtime::task::into_stream(task) {
            self.tasks.push(stream);
        }
    }
    fn drain(&mut self) {
        if let Some(Some(())) = self.wakes.next().now_or_never() {
            self.message(Message::WorldMeshPresented);
        }
        for _ in 0..256 {
            match self.tasks.next().now_or_never() {
                Some(Some(Action::Output(message))) => self.message(message),
                Some(Some(Action::Reload)) => {}
                Some(Some(action)) => panic!("只读诊断不应触发外部运行时动作：{action:?}"),
                _ => break,
            }
        }
    }
    fn frame(&mut self, events: &[Event], cursor: Point) -> serde_json::Value {
        self.drain();
        let start = Instant::now();
        let mut ui = UserInterface::build(
            crate::view(&self.app),
            self.size,
            std::mem::take(&mut self.cache),
            &mut self.renderer,
        );
        let mut layout_ms = start.elapsed().as_secs_f64() * 1000.;
        let mut messages = Vec::new();
        let start = Instant::now();
        let (state, _) = ui.update(
            events,
            mouse::Cursor::Available(cursor),
            &mut self.renderer,
            &mut iced::advanced::clipboard::Null,
            &mut messages,
        );
        let input_ms = start.elapsed().as_secs_f64() * 1000.;
        let message_count = messages.len();
        let rebuild = state.has_layout_changed() || !messages.is_empty();
        let draw_ms;
        if rebuild {
            self.cache = ui.into_cache();
            for message in messages {
                self.message(message);
            }
            let start = Instant::now();
            let mut ui = UserInterface::build(
                crate::view(&self.app),
                self.size,
                std::mem::take(&mut self.cache),
                &mut self.renderer,
            );
            layout_ms += start.elapsed().as_secs_f64() * 1000.;
            let start = Instant::now();
            ui.draw(
                &mut self.renderer,
                &iced::Theme::Dark,
                &iced::advanced::renderer::Style {
                    text_color: Color::WHITE,
                },
                mouse::Cursor::Available(cursor),
            );
            draw_ms = start.elapsed().as_secs_f64() * 1000.;
            self.cache = ui.into_cache();
        } else {
            let start = Instant::now();
            ui.draw(
                &mut self.renderer,
                &iced::Theme::Dark,
                &iced::advanced::renderer::Style {
                    text_color: Color::WHITE,
                },
                mouse::Cursor::Available(cursor),
            );
            draw_ms = start.elapsed().as_secs_f64() * 1000.;
            self.cache = ui.into_cache();
        }
        let target = self.target.create_view(&Default::default());
        let start = Instant::now();
        let renderer = &mut self.renderer;
        let submission = renderer.present(
            Some(Color::from_rgb(0.16, 0.17, 0.18)),
            wgpu::TextureFormat::Rgba8Unorm,
            &target,
            &self.viewport,
        );
        let present_ms = start.elapsed().as_secs_f64() * 1000.;
        // 在真实 queue.submit 后记录交接；可见性取样和 GPU 等待不能计入上屏准备耗时。
        if let Some(flow) = &mut self.flow {
            flow.mesh(
                self.app
                    .spectator_stream_snapshot
                    .as_ref()
                    .and_then(|s| s.ready.lock().unwrap().clone()),
                false,
            );
            flow.mesh(self.app.spectator_presented.lock().unwrap().clone(), true);
        }
        let start = Instant::now();
        self.in_flight.push_back(submission);
        // 原生 Iced 交换链最多两帧在途；不逐帧等待最新 GPU 提交，保留 CPU/GPU 重叠。
        // 显式串行模式用于和冻结的旧诊断程序比较，截图自身仍等待实际回读完成。
        let poll = if self.in_flight.len() >= 2
            || std::env::var_os("MCEDIT2_PROFILE_SERIAL_GPU").is_some()
        {
            wgpu::PollType::Wait {
                submission_index: self.in_flight.pop_front(),
                timeout: Some(Duration::from_secs(60)),
            }
        } else {
            wgpu::PollType::Poll
        };
        self.device.poll(poll).unwrap();
        let wait_ms = start.elapsed().as_secs_f64() * 1000.;
        let app_ms = std::mem::take(&mut self.app_ms);
        self.observe_reveal();
        serde_json::json!({"application_ms":app_ms,"layout_ms":layout_ms,"input_ms":input_ms,
            "draw_ms":draw_ms,"present_ms":present_ms,"wait_ms":wait_ms,"messages":message_count})
    }
    fn observe_reveal(&mut self) {
        let Some(reveal) = self.reveal.as_mut() else {
            return;
        };
        if reveal.visible_probe_ms.is_some() {
            return;
        }
        let Some(view) = self.app.spectator_presented_view.lock().unwrap().clone() else {
            return;
        };
        if reveal
            .sampled_mesh
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &view.mesh))
        {
            return;
        }
        reveal.sampled_mesh = Some(Arc::clone(&view.mesh));
        let new_column = |x: i32, z: i32| !reveal.columns.contains(&(x, z));
        if !view
            .mesh
            .chunks
            .iter()
            .any(|part| part.key.2 == 1 && new_column(part.key.0, part.key.1))
        {
            return;
        }
        reveal
            .submitted_ms
            .get_or_insert_with(|| reveal.started.elapsed().as_secs_f64() * 1000.);
        // 只确认当前已显示网格中、实际相机可命中的新区地形，不以后台读取完成冒充出现。
        for y in 1..8 {
            for x in 1..14 {
                let point = Point::new(
                    view.viewport.logical.width * x as f32 / 14.,
                    view.viewport.logical.height * y as f32 / 8.,
                );
                if view
                    .mesh
                    .pick_with_atlas(
                        view.camera,
                        point,
                        view.viewport,
                        view.quality,
                        view.atlas.as_deref(),
                    )
                    .and_then(|hit| hit.owner)
                    .is_some_and(|owner| {
                        new_column(
                            owner.position[0].div_euclid(16),
                            owner.position[2].div_euclid(16),
                        )
                    })
                {
                    reveal.visible_probe_ms = Some(reveal.started.elapsed().as_secs_f64() * 1000.);
                    return;
                }
            }
        }
    }
    fn ready(&self) -> bool {
        if self.app.near_busy {
            return false;
        }
        if self
            .app
            .spectator_stream
            .as_ref()
            .is_none_or(|s| s.running || s.dirty || s.total == 0 || s.complete != s.total)
        {
            return false;
        }
        let layers = self.app.scene_layers.snapshot.as_ref();
        if self
            .app
            .entity_models
            .frame
            .as_ref()
            .is_none_or(|f| layers.is_none_or(|s| !Arc::ptr_eq(s, &f.snapshot)))
        {
            return false;
        }
        let current = self
            .app
            .spectator_stream_snapshot
            .as_ref()
            .and_then(|s| s.ready.lock().unwrap().clone());
        let shown = self.app.spectator_presented.lock().unwrap().clone();
        current.as_ref().is_some_and(|c| {
            c.complete
                && shown
                    .as_ref()
                    .is_some_and(|s| s.complete && crate::spectator::StreamMesh::same_terrain(s, c))
        })
    }
    fn pending_status(&self) -> String {
        let current = self
            .app
            .spectator_stream_snapshot
            .as_ref()
            .and_then(|s| s.ready.lock().unwrap().clone());
        let shown = self.app.spectator_presented.lock().unwrap().clone();
        format!(
            "任务={}，近景={}，建模={}，当前完整={:?}，显示完整={:?}，地形同代={}，实体同代={}；{}",
            !self.tasks.is_empty(),
            self.app.near_busy,
            *self.app.spectator_pending.lock().unwrap(),
            current.as_ref().map(|m| m.complete),
            shown.as_ref().map(|m| m.complete),
            current
                .as_ref()
                .zip(shown.as_ref())
                .is_some_and(|(a, b)| crate::spectator::StreamMesh::same_terrain(a, b)),
            self.app.entity_models.frame.as_ref().is_some_and(|f| self
                .app
                .scene_layers
                .snapshot
                .as_ref()
                .is_some_and(|s| Arc::ptr_eq(s, &f.snapshot))),
            self.app
                .spectator_stream
                .as_ref()
                .map_or_else(String::new, |s| s.status())
        )
    }
    fn settle(&mut self, cursor: Point) -> f64 {
        let start = Instant::now();
        let timeout = std::env::var("MCEDIT2_PROFILE_TIMEOUT_SECONDS")
            .map(|value| value.parse::<u64>().expect("诊断超时秒数必须是整数"))
            .unwrap_or(180);
        assert!((1..=3600).contains(&timeout), "诊断超时范围为 1..=3600 秒");
        let mut stable = 0;
        let mut reported = 0;
        loop {
            self.frame(
                &[Event::Window(iced::window::Event::RedrawRequested(
                    Instant::now(),
                ))],
                cursor,
            );
            stable = if self.ready() { stable + 1 } else { 0 };
            if stable >= 3 {
                return start.elapsed().as_secs_f64() * 1000.;
            }
            let elapsed = start.elapsed().as_secs();
            if elapsed / 10 > reported {
                reported = elapsed / 10;
                eprintln!(
                    "[应用回放 {elapsed}s] ready={} stable={stable} {}",
                    self.ready(),
                    self.pending_status()
                );
            }
            assert!(
                start.elapsed() < Duration::from_secs(timeout),
                "应用尚未完成：{} / {} / {} / {}",
                self.pending_status(),
                self.app.surface_report,
                self.app
                    .spectator_stream
                    .as_ref()
                    .map_or_else(String::new, |s| s.status()),
                self.app.entity_models.status
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
pub(crate) fn run(world: mcedit2_world::WorldSummary, bank: Arc<TextureBank>, radius: u16) {
    mcedit2_world::diagnostics::install(&mcedit2_world::launch::Arguments {
        debug: true,
        verbosity: 1,
        ..Default::default()
    })
    .unwrap();
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let source = Arc::new(
        mcedit2_world::editing::Session::new(world.clone(), world.dimensions[0].name.clone())
            .unwrap(),
    );
    let reading =
        crate::stream_readers::compare_throughput(source, Arc::clone(&bank), radius.min(3));
    std::fs::write(
        directory.join("experience-reading.json"),
        serde_json::to_vec_pretty(&reading).unwrap(),
    )
    .unwrap();
    eprintln!("[读取对照] {reading}");
    let mut driver = Driver::new(world, bank, radius);
    let mut cursor = Point::new(1000., 450.);
    let startup = driver.settle(cursor);
    let initial_view = driver
        .app
        .spectator_presented_view
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    for _ in 0..12 {
        driver.frame(
            &[Event::Window(iced::window::Event::RedrawRequested(
                Instant::now(),
            ))],
            cursor,
        );
    }
    let mut samples = Vec::new();
    let before = (
        driver.counts.near,
        driver.counts.plans,
        driver.counts.models,
    );
    driver.frame(
        &[Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Right,
        ))],
        cursor,
    );
    for _ in 0..60 {
        let paced = Instant::now();
        let mut events = Vec::new();
        for _ in 0..16 {
            cursor.x += 0.25;
            events.push(Event::Mouse(mouse::Event::CursorMoved { position: cursor }));
        }
        events.push(Event::Window(iced::window::Event::RedrawRequested(
            Instant::now(),
        )));
        let mut sample = driver.frame(&events, cursor);
        sample["phase"] = "look".into();
        samples.push(sample);
        std::thread::sleep(Duration::from_millis(16).saturating_sub(paced.elapsed()));
    }
    driver.frame(
        &[Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Right,
        ))],
        cursor,
    );
    let after_look = (
        driver.counts.near,
        driver.counts.plans,
        driver.counts.models,
    );
    let shown = driver
        .app
        .spectator_presented
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    driver.reveal = Some(Reveal {
        started: Instant::now(),
        columns: shown
            .chunks
            .iter()
            .filter(|part| part.key.2 == 1)
            .map(|part| (part.key.0, part.key.1))
            .collect(),
        submitted_ms: None,
        visible_probe_ms: None,
        sampled_mesh: None,
    });
    driver.flow = Some(FlowProbe {
        started: Instant::now(),
        initial: shown
            .chunks
            .iter()
            .filter(|p| p.key.2 == 1)
            .map(|p| (p.key.0, p.key.1))
            .collect(),
        columns: BTreeMap::new(),
        cpu: None,
        gpu: None,
        known_cpu: shown
            .terrain
            .as_ref()
            .unwrap_or(&shown)
            .chunks
            .iter()
            .filter(|p| p.key.2 == 1)
            .map(|p| (p.key.0, p.key.1))
            .collect(),
        known_gpu: shown
            .terrain
            .as_ref()
            .unwrap_or(&shown)
            .chunks
            .iter()
            .filter(|p| p.key.2 == 1)
            .map(|p| (p.key.0, p.key.1))
            .collect(),
    });
    driver.message(Message::SpectatorKey(keyboard::Event::KeyPressed {
        key: keyboard::Key::Character("w".into()),
        modified_key: keyboard::Key::Character("w".into()),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::KeyW),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        text: None,
        repeat: false,
    }));
    let fast_flight = std::env::var_os("MCEDIT2_PROFILE_FAST").is_some();
    if fast_flight {
        driver.message(Message::SpectatorKey(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Control),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Control),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ControlLeft),
            location: keyboard::Location::Left,
            modifiers: keyboard::Modifiers::CTRL,
            text: None,
            repeat: false,
        }));
        assert!(
            driver.app.spectator_keys.contains(&'!'),
            "加速回放必须经过实际加速快捷键"
        );
    }
    let clock = driver.app.spectator_last_frame.unwrap();
    let continuous_seconds = std::env::var("MCEDIT2_PROFILE_FLIGHT_SECONDS")
        .ok()
        .map(|v| {
            let seconds = v.parse::<u64>().expect("持续回放秒数必须是整数");
            assert!((1..=120).contains(&seconds));
            seconds
        });
    let flight_started = Instant::now();
    for i in 1.. {
        if continuous_seconds.map_or(i > 180, |seconds| {
            flight_started.elapsed() >= Duration::from_secs(seconds)
        }) {
            break;
        }
        let paced = Instant::now();
        driver.message(Message::SpectatorTick(if continuous_seconds.is_some() {
            Instant::now()
        } else {
            clock + Duration::from_millis(i * 16)
        }));
        let mut sample = driver.frame(
            &[Event::Window(iced::window::Event::RedrawRequested(
                Instant::now(),
            ))],
            cursor,
        );
        sample["phase"] = "flight".into();
        samples.push(sample);
        std::thread::sleep(Duration::from_millis(16).saturating_sub(paced.elapsed()));
    }
    let flight_wall_ms = flight_started.elapsed().as_secs_f64() * 1000.;
    driver.message(Message::SpectatorFocusLost);
    let tail = driver.settle(cursor);
    let camera = driver.app.spectator_camera;
    assert!(
        camera.yaw > 40. && camera.yaw < 50.,
        "真实控件应当改变相机：{camera:?}"
    );
    assert_eq!(
        before, after_look,
        "完整来源中的转头不能重新读盘或重建位置模型"
    );
    assert!(driver
        .app
        .sessions
        .values()
        .all(|s| !s.has_pending_changes()));
    let renderer = &mut driver.renderer;
    let rgba = renderer.screenshot(&driver.viewport, Color::from_rgb(0.16, 0.17, 0.18));
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    image::save_buffer(
        directory.join("app-profile-final.png"),
        &rgba,
        1920,
        1080,
        image::ColorType::Rgba8,
    )
    .unwrap();
    driver
        .app
        .spectator_stream
        .as_ref()
        .unwrap()
        .verify_source_memory();
    let mut report = serde_json::json!({"adapter":driver.adapter,"startup_ms":startup,"tail_ms":tail,
        "radius":radius,"source_cache_memory":driver.app.spectator_stream.as_ref().map(|stream|stream.source_memory()),"stream":driver.app.spectator_stream.as_ref().map(|stream|stream.status()),
        "debug_assertions":cfg!(debug_assertions),"instance_flags":format!("{:?}",profile_instance_flags()),
        "maximum_frames_in_flight":if std::env::var_os("MCEDIT2_PROFILE_SERIAL_GPU").is_some() {1} else {2},
        "prepare_ahead":crate::spectator_stream::prepare_ahead(),
        "flight_blocks_per_second":if fast_flight {30} else {12},
        "read_reference":std::env::var_os("MCEDIT2_READ_REFERENCE").is_some(),
        "open_flight":std::env::var_os("MCEDIT2_PROFILE_OPEN_FLIGHT").is_some(),
        "first_new_region_submitted_ms":driver.reveal.as_ref().and_then(|r|r.submitted_ms),
        "first_new_region_visible_probe_ms":driver.reveal.as_ref().and_then(|r|r.visible_probe_ms),"window_created":false,"width":1920,"height":1080,"samples":samples,
        "look_messages":driver.counts.look,"near_publications":driver.counts.near,
        "source_plans":driver.counts.plans,"entity_publications":driver.counts.models,"appearance_cache":driver.app.entity_models.appearance_statistics(),"camera":camera});
    report["reading_comparison"] = reading;
    report["stream_serial_reference"] = crate::spectator_stream::serial_reference().into();
    report["continuous_flight_seconds"] = serde_json::json!(continuous_seconds);
    report["flight_wall_ms"] = flight_wall_ms.into();
    report["pipeline"] = driver.flow.as_ref().unwrap().report(
        driver
            .app
            .spectator_stream
            .as_ref()
            .unwrap()
            .source_coordinates()
            .collect(),
    );
    // 用生产控件树离屏查看窄侧栏布局，不创建或占用桌面窗口。
    driver.app.workspace_pane = crate::workspace_ui::Pane::Display;
    driver.app.language = crate::Language::Chinese;
    driver.app.workspace.layout = crate::workspace_ui::layout::State::restore(
        &crate::workspace_ui::layout::Saved::default()
            .with_legacy(crate::workspace_ui::layout::Side::Left, Some(0.16), false)
            .unwrap(),
    );
    for (group, name) in [
        (crate::display_options::Group::Distance, "distance"),
        (crate::display_options::Group::Camera, "camera"),
        (crate::display_options::Group::Layers, "layers"),
    ] {
        driver.app.workspace.presentation.display_group = group;
        driver.frame(
            &[Event::Window(iced::window::Event::RedrawRequested(
                Instant::now(),
            ))],
            cursor,
        );
        let renderer = &mut driver.renderer;
        let pixels = renderer.screenshot(&driver.viewport, Color::from_rgb(0.16, 0.17, 0.18));
        image::save_buffer(
            directory.join(format!("experience-layout-{name}.png")),
            &pixels,
            1920,
            1080,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    let final_view = driver
        .app
        .spectator_presented_view
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    driver.app.retire_spectator_sources();
    driver.app.scene_layers.invalidate();
    let _ = driver
        .app
        .entity_models
        .maintain_with_root(None, None, None, false, false, None);
    // 先释放完整界面的 GPU 资源，再以相同实际网格、相机、尺寸和采样计时，
    // 不把设备等待、纹理上传或另一套较小分辨率混成世界通道的 GPU 时间。
    drop(driver);
    if std::env::var_os("MCEDIT2_GPU_PROFILE").is_some() {
        let mut calibration = Vec::new();
        for (name, view) in [("look_start", initial_view), ("flight_end", final_view)] {
            let crate::world_camera::WorldCamera::Free(camera) = view.camera else {
                panic!("应用诊断应使用实际透视相机");
            };
            let mut hashes = Vec::new();
            let mut images = Vec::new();
            for section_culling in [false, true] {
                let path = directory.join(format!("section-{name}-{section_culling}.png"));
                let mut timing = crate::gpu_scene::hardware_tests::profile_world(
                    Arc::clone(&view.mesh),
                    camera,
                    crate::gpu_scene::hardware_tests::WorldProfile {
                        size: Size::new(view.viewport.raster.width, view.viewport.raster.height),
                        quality: view.quality,
                        section_culling,
                        reference_image: Some(path.clone()),
                        ..Default::default()
                    },
                );
                timing["view"] = name.into();
                timing["section_culling"] = section_culling.into();
                hashes.push(timing["pixel_hash"].clone());
                images.push(path);
                calibration.push(timing);
            }
            let reference = image::open(&images[0]).unwrap().to_rgba8();
            let culled = image::open(&images[1]).unwrap().to_rgba8();
            assert_eq!(reference.dimensions(), culled.dimensions());
            let changed = reference
                .pixels()
                .zip(culled.pixels())
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(changed, 0, "{name}：空域剔除改变了 {changed} 个像素");
            assert_eq!(hashes[0], hashes[1], "{name}：稳定像素摘要不一致");
        }
        report["gpu_calibration"] = calibration.into();
    }
    std::fs::write(
        directory.join("app-profile-world.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    eprintln!(
        "完整应用离屏诊断完成：启动 {startup:.1}ms，收尾 {tail:.1}ms；结果 app-profile-world.json"
    );
}
