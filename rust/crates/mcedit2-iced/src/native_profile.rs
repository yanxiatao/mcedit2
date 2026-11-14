//! 可选的真实窗口回放；等待实际源与显示世代一致后才开始计时。
use crate::{App, Message};
use iced::{Subscription, Task};
use iced_wgpu::window::frame_statistics::{self, Session};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct Config {
    output: PathBuf,
    pub window_size: iced::Size,
    pub scale: f32,
    pub radius: Option<u16>,
    review: Duration,
    pub outcome: Arc<AtomicU8>,
}
impl Config {
    pub fn load(args: &mcedit2_world::launch::Arguments) -> Result<Option<Self>, String> {
        let Some(output) = std::env::var_os("MCEDIT2_NATIVE_PROFILE") else {
            return Ok(None);
        };
        let output = PathBuf::from(output);
        if !output.is_absolute() || !args.view || args.worlds.len() != 1 {
            return Err("原生回放要求绝对输出路径、-view 和一个存档".into());
        }
        let window_size = match setting("MCEDIT2_NATIVE_PROFILE_WINDOW")? {
            None => iced::window::Settings::default().size,
            Some(value) => {
                let (w, h) = value.split_once('x').ok_or("回放窗口格式应为宽x高")?;
                let width: f32 = w.parse().map_err(|e| format!("回放窗口宽度无效：{e}"))?;
                let height: f32 = h.parse().map_err(|e| format!("回放窗口高度无效：{e}"))?;
                if !(320. ..=8192.).contains(&width) || !(240. ..=8192.).contains(&height) {
                    return Err("回放窗口尺寸超出允许范围".into());
                }
                iced::Size::new(width, height)
            }
        };
        let scale: f32 = setting("MCEDIT2_NATIVE_PROFILE_SCALE")?
            .map(|v| v.parse::<f32>().map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or(1.);
        if !(0.5..=4.).contains(&scale) {
            return Err("回放缩放应在 0.5..=4 之间".into());
        }
        let radius = setting("MCEDIT2_NATIVE_PROFILE_RADIUS")?
            .map(|value| {
                value
                    .parse::<u16>()
                    .map_err(|error| format!("回放视距无效：{error}"))
            })
            .transpose()?;
        if let Some(radius) = radius {
            crate::camera_distance::Saved {
                radius,
                ..Default::default()
            }
            .validate()?;
        }
        let review: u64 = setting("MCEDIT2_NATIVE_PROFILE_REVIEW_SECONDS")?
            .map(|v| v.parse::<u64>().map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or(0);
        if review > 30 {
            return Err("回放结束的画面查看时间不能超过 30 秒".into());
        }
        Ok(Some(Self {
            output,
            window_size,
            scale,
            radius,
            review: Duration::from_secs(review),
            outcome: Arc::new(AtomicU8::new(0)),
        }))
    }
}
fn setting(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(format!("{name} 无效：{error}")),
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Event {
    Poll,
    Frame(Instant),
    Focus(bool),
    Interrupt,
}
pub(crate) struct State {
    config: Config,
    launched: Instant,
    focused: bool,
    announced: bool,
    warmup: Option<Instant>,
    previous: Option<Instant>,
    capture: Option<Session>,
    flight_started: Option<Instant>,
    initial: Option<crate::spectator::Camera>,
    review_until: Option<Instant>,
}
impl State {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            launched: Instant::now(),
            focused: false,
            announced: false,
            warmup: None,
            previous: None,
            capture: None,
            flight_started: None,
            initial: None,
            review_until: None,
        }
    }
    pub fn subscription(&self) -> Subscription<Message> {
        let mut sources = vec![iced::event::listen_with(|event, _, _| {
            let event = match event {
                iced::Event::Window(iced::window::Event::Focused) => Event::Focus(true),
                iced::Event::Window(iced::window::Event::Unfocused) => Event::Focus(false),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { .. })
                | iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
                | iced::Event::Mouse(iced::mouse::Event::WheelScrolled { .. }) => Event::Interrupt,
                _ => return None,
            };
            Some(Message::NativeProfile(event))
        })];
        if self.capture.is_some() {
            sources.push(iced::window::frames().map(|at| Message::NativeProfile(Event::Frame(at))));
        } else {
            sources.push(
                iced::time::every(Duration::from_millis(250))
                    .map(|_| Message::NativeProfile(Event::Poll)),
            );
        }
        Subscription::batch(sources)
    }
    fn advance(&mut self, app: &mut App, event: Event) -> Result<Task<Message>, String> {
        if let Some(until) = self.review_until {
            return Ok(
                if matches!(event, Event::Interrupt | Event::Focus(false))
                    || Instant::now() >= until
                {
                    iced::exit()
                } else {
                    Task::none()
                },
            );
        }
        match event {
            Event::Focus(focused) => {
                self.focused = focused;
                if !focused {
                    if self.capture.is_some() {
                        return self.finish(app, Some("测量时窗口失去焦点"));
                    }
                    self.warmup = None;
                }
            }
            Event::Interrupt => {
                if self.capture.is_some() {
                    return self.finish(app, Some("用户输入中止回放"));
                }
                self.warmup = None;
            }
            Event::Frame(at) if self.capture.is_some() => {
                let elapsed = at.duration_since(self.previous.replace(at).unwrap());
                let recording_elapsed = at.duration_since(self.warmup.unwrap());
                if self.flight_started.is_none() && recording_elapsed >= Duration::from_secs(3) {
                    self.flight_started = Some(at);
                }
                if let Some(flight) = self.flight_started {
                    if at.duration_since(flight) >= Duration::from_secs(3) {
                        return self.finish(app, None);
                    }
                    let keys = std::collections::BTreeSet::from(['w', 'd']);
                    app.spectator_camera.fly(&keys, elapsed.as_secs_f32());
                    app.recenter_spectator();
                } else {
                    let initial = self.initial.expect("旋转回放缺少初始相机");
                    let phase = recording_elapsed.as_secs_f32() / 3.;
                    let target = initial.yaw + (phase * std::f32::consts::TAU).sin() * 12.;
                    let delta = (target - app.spectator_camera.yaw + 180.).rem_euclid(360.) - 180.;
                    app.spectator_camera.look(delta / 0.19, 0.);
                }
            }
            Event::Poll => {
                if self.launched.elapsed() > Duration::from_secs(600) {
                    return self.finish(app, Some("等待完整源或窗口焦点超时"));
                }
                if ready(app)? {
                    if !self.announced {
                        log::error!("[native-profile] 源与窗口显示已就绪；聚焦后预热 1 秒，旋转 3 秒、飞行 3 秒，随后自动关闭");
                        self.announced = true;
                    }
                    if self.focused {
                        let now = Instant::now();
                        let warmup = *self.warmup.get_or_insert(now);
                        if now.duration_since(warmup) >= Duration::from_secs(1) {
                            self.initial = Some(app.spectator_camera);
                            self.capture = Some(frame_statistics::start());
                            self.warmup = Some(now);
                            self.previous = Some(now);
                        }
                    }
                } else {
                    self.warmup = None;
                }
            }
            _ => {}
        }
        Ok(Task::none())
    }
    fn finish(&mut self, app: &App, reason: Option<&str>) -> Result<Task<Message>, String> {
        let recording = self.capture.take().map(Session::finish);
        let mut rotation = Vec::new();
        let mut flight = Vec::new();
        let mut errors = Vec::new();
        let surface = recording.as_ref().and_then(|r| r.surface.as_ref()).map(|s| serde_json::json!({
            "adapter":s.adapter,"format":s.format,"present_mode":s.present_mode,"maximum_frame_latency":s.maximum_frame_latency,"size":s.size
        }));
        if let Some(recording) = recording {
            for frame in recording.frames {
                if let Some(error) = &frame.error {
                    errors.push(error.clone());
                }
                let sections: std::collections::BTreeMap<_, f64> = frame.sections.into_iter().fold(
                    std::collections::BTreeMap::new(),
                    |mut all, s| {
                        *all.entry(s.name).or_default() += s.elapsed.as_secs_f64() * 1000.;
                        all
                    },
                );
                let sample = serde_json::json!({
                    "at_ms":frame.ended.duration_since(recording.started).as_secs_f64()*1000.,
                    "interval_ms":frame.interval.map(|d|d.as_secs_f64()*1000.),
                    "acquire_ms":frame.acquire.as_secs_f64()*1000.,"render_submit_ms":frame.render_submit.as_secs_f64()*1000.,
                    "present_ms":frame.present.as_secs_f64()*1000.,"size":frame.size,"error":frame.error,"sections_ms":sections,"counters":frame.counters
                });
                if self
                    .flight_started
                    .is_some_and(|start| frame.ended >= start)
                {
                    flight.push(sample);
                } else {
                    rotation.push(sample);
                }
            }
        }
        let sufficient = [&rotation, &flight].iter().all(|samples| {
            samples
                .iter()
                .filter(|s| s["counters"]["world_passes"].as_u64().unwrap_or(0) > 0)
                .count()
                >= 2
        });
        let status = if reason.is_some() {
            "cancelled"
        } else if !errors.is_empty()
            || !sufficient
            || app.spectator_stream.as_ref().is_some_and(|s| s.errors != 0)
        {
            "failed"
        } else {
            "complete"
        };
        let camera = |c: crate::spectator::Camera| serde_json::json!({"position":c.position,"yaw":c.yaw,"pitch":c.pitch,"zoom":c.zoom});
        let report = serde_json::json!({
            "status":status,"reason":reason,"surface":surface,"errors":errors,
            "measurement":"真实窗口提交间隔与 CPU 阶段耗时，不代表屏幕显示完成或 GPU 执行耗时",
            "path":"绕初始朝向左右各 12 度往返旋转 3 秒，随后沿 w+d 飞行 3 秒",
            "fullbright":true,"radius":app.spectator_radius,"textured":app.show_textures,
            "quality":format!("{:?}",app.gpu_quality),"application_scale":self.config.scale,"appearance_cache":app.entity_models.appearance_statistics(),
            "viewport":app.spectator_presented_view.lock().expect("记录最终显示尺寸失败").as_ref().map(|f|serde_json::json!({"logical":[f.viewport.logical.width,f.viewport.logical.height],"raster":[f.viewport.raster.width,f.viewport.raster.height]})),
            "source_cache_memory":app.spectator_stream.as_ref().map(|stream|stream.source_memory()),
            "source":app.spectator_stream.as_ref().map(|s|serde_json::json!({"complete":s.complete,"total":s.total,"errors":s.errors})),
            "initial_camera":self.initial.map(camera),"final_camera":camera(app.spectator_camera),
            "rotation":rotation,"flight":flight
        });
        let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(&self.config.output, bytes)
            .map_err(|e| format!("写入原生回放记录失败：{e}"))?;
        self.config
            .outcome
            .store(if status == "complete" { 1 } else { 2 }, Ordering::Release);
        log::error!(
            "[native-profile] {status}: {}",
            self.config.output.display()
        );
        if status == "complete" && !self.config.review.is_zero() {
            self.review_until = Some(Instant::now() + self.config.review);
            log::error!(
                "[native-profile] 记录已完成，保留画面 {:?}；输入或失焦即关闭",
                self.config.review
            );
            Ok(Task::none())
        } else {
            Ok(iced::exit())
        }
    }
}
fn ready(app: &App) -> Result<bool, String> {
    if app.current_key().is_none() && !app.near_busy {
        return Err(format!("存档没有成功打开：{:?}", app.messages));
    }
    if !app.spectator || !app.use_gpu || app.show_top_down || app.show_advanced {
        return Err("原生回放需要透视 GPU 视图".into());
    }
    if app.texture_loading
        || app.near_busy
        || app.near_job.is_some()
        || !app.entity_models.is_idle()
        || !app.scene_layers.is_idle()
    {
        return Ok(false);
    }
    let Some(stream) = &app.spectator_stream else {
        return Ok(false);
    };
    if stream.errors != 0 {
        return Err(format!("源读取失败 {} 个区块", stream.errors));
    }
    if !stream.is_idle()
        || stream.dirty
        || !stream.matches_resources(
            app.texture_bank.as_ref(),
            app.show_textures,
            app.entity_models.visibility.block_layers(),
        )
    {
        return Ok(false);
    }
    let Some(snapshot) = &app.spectator_stream_snapshot else {
        return Ok(false);
    };
    let ready = snapshot.ready.lock().expect("读取回放源世代失败");
    let shown = app
        .spectator_presented
        .lock()
        .expect("读取回放显示世代失败");
    let (Some(ready), Some(shown)) = (ready.as_ref(), shown.as_ref()) else {
        return Ok(false);
    };
    if !ready.complete
        || !shown.complete
        || !crate::spectator::StreamMesh::same_terrain(ready, shown)
    {
        return Ok(false);
    }
    Ok(match (&app.entity_models.frame, &shown.entities) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    })
}
pub(crate) fn update(app: &mut App, event: Event) -> Task<Message> {
    let mut state = app.native_profile.take().expect("回放事件没有运行状态");
    let result = state.advance(app, event);
    app.native_profile = Some(state);
    match result {
        Ok(task) => task,
        Err(error) => {
            log::error!("[native-profile] {error}");
            app.native_profile
                .as_ref()
                .unwrap()
                .config
                .outcome
                .store(2, Ordering::Release);
            iced::exit()
        }
    }
}
