//! 原生呈现速率及可选详细记录。不等待 GPU，也不创建离屏替身。
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<State>> = OnceLock::new();

/// 两秒滑动速率，按 100ms 合并完成事件，最多保留 21 个桶。
/// 采样不足半秒时没有速率；窗口起点所在桶可能带来不足 100ms 的统计误差。
/// 计时包含等待与空闲，不表示某段 CPU / GPU 代码的执行速度。
pub struct RollingRate {
    started: Instant,
    buckets: VecDeque<(Instant, u64)>,
}
impl Default for RollingRate {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            buckets: VecDeque::new(),
        }
    }
}
impl RollingRate {
    pub fn record(&mut self, completed: u64, now: Instant) {
        while self
            .buckets
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) >= Duration::from_secs(2))
        {
            let _ = self.buckets.pop_front();
        }
        if completed == 0 {
            return;
        }
        if let Some((at, count)) = self.buckets.back_mut() {
            if now.duration_since(*at) < Duration::from_millis(100) {
                *count += completed;
                return;
            }
        }
        self.buckets.push_back((now, completed));
    }
    pub fn per_second(&self, now: Instant) -> Option<f64> {
        let elapsed = now.duration_since(self.started);
        if elapsed < Duration::from_millis(500) {
            return None;
        }
        let count: u64 = self
            .buckets
            .iter()
            .filter(|(at, _)| now.duration_since(*at) < Duration::from_secs(2))
            .map(|(_, count)| count)
            .sum();
        Some(count as f64 / elapsed.min(Duration::from_secs(2)).as_secs_f64())
    }
}

/// 当前进程窗口的呈现提交速率；不包含截图或失败的 surface 获取。
pub fn presentation_rate(now: Instant) -> Option<f64> {
    state()
        .lock()
        .expect("读取呈现速率时锁失败")
        .live
        .per_second(now)
}
/// 仅由实际窗口绘制事件调用；限制状态栏消息频率，不安排下一帧或定时唤醒。
pub fn refresh_due(now: Instant) -> bool {
    let mut state = state().lock().expect("刷新呈现速率时锁失败");
    if !state.has_presented
        || state
            .last_report
            .is_some_and(|at| now.duration_since(at) < Duration::from_millis(500))
    {
        return false;
    }
    state.last_report = Some(now);
    true
}
pub(super) fn presented(now: Instant) {
    let mut state = state().lock().expect("记录呈现速率时锁失败");
    state.live.record(1, now);
    state.has_presented = true;
}

#[derive(Clone, Debug)]
pub struct SurfaceInfo {
    pub adapter: String,
    pub format: String,
    pub present_mode: String,
    pub maximum_frame_latency: u32,
    pub size: [u32; 2],
}
#[derive(Debug)]
pub struct Section {
    pub name: &'static str,
    pub elapsed: Duration,
}
#[derive(Debug)]
pub struct Frame {
    pub ended: Instant,
    pub interval: Option<Duration>,
    pub acquire: Duration,
    pub render_submit: Duration,
    pub present: Duration,
    pub size: [u32; 2],
    pub error: Option<String>,
    pub sections: Vec<Section>,
    pub counters: BTreeMap<&'static str, u64>,
}
#[derive(Debug)]
pub struct Recording {
    pub started: Instant,
    pub surface: Option<SurfaceInfo>,
    pub frames: Vec<Frame>,
}
#[derive(Default)]
struct State {
    surface: Option<SurfaceInfo>,
    capture: Option<Capture>,
    live: RollingRate,
    has_presented: bool,
    last_report: Option<Instant>,
}
struct Capture {
    started: Instant,
    previous_present: Option<Instant>,
    sections: Vec<Section>,
    counters: BTreeMap<&'static str, u64>,
    frames: Vec<Frame>,
}
fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}
pub fn is_recording() -> bool {
    ENABLED.load(Ordering::Acquire)
}
pub struct Session {
    active: bool,
}
pub fn start() -> Session {
    let mut state = state().lock().expect("原生呈现记录锁失败");
    assert!(state.capture.is_none(), "同一进程只能有一份原生呈现记录");
    state.capture = Some(Capture {
        started: Instant::now(),
        previous_present: None,
        sections: Vec::new(),
        counters: BTreeMap::new(),
        frames: Vec::new(),
    });
    ENABLED.store(true, Ordering::Release);
    Session { active: true }
}
fn stop() -> Recording {
    ENABLED.store(false, Ordering::Release);
    let mut state = state().lock().expect("结束原生呈现记录时锁失败");
    let capture = state.capture.take().expect("原生呈现记录尚未开始");
    Recording {
        started: capture.started,
        surface: state.surface.clone(),
        frames: capture.frames,
    }
}
impl Session {
    pub fn finish(mut self) -> Recording {
        self.active = false;
        stop()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if self.active {
            drop(stop());
        }
    }
}
/// 区间在记录关闭后结束时不再计入；关闭操作不等待其他调用者。
pub struct Span {
    name: &'static str,
    started: Option<Instant>,
}
pub fn span(name: &'static str) -> Span {
    Span {
        name,
        started: is_recording().then(Instant::now),
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            if is_recording() {
                if let Some(capture) = &mut state().lock().expect("记录原生区间时锁失败").capture
                {
                    capture.sections.push(Section {
                        name: self.name,
                        elapsed: started.elapsed(),
                    });
                }
            }
        }
    }
}
pub fn counter(name: &'static str, value: u64) {
    if !is_recording() {
        return;
    }
    if let Some(capture) = &mut state().lock().expect("记录原生计数时锁失败").capture {
        *capture.counters.entry(name).or_default() += value;
    }
}
pub(super) fn surface(info: SurfaceInfo) {
    state().lock().expect("记录窗口配置时锁失败").surface = Some(info);
}
pub(super) fn frame(
    started: Instant,
    acquired: Instant,
    submitted: Instant,
    ended: Instant,
    size: [u32; 2],
    error: Option<String>,
) {
    let mut state = state().lock().expect("记录窗口提交时锁失败");
    let Some(capture) = &mut state.capture else {
        return;
    };
    let interval = capture
        .previous_present
        .map(|previous| ended.duration_since(previous));
    if error.is_none() {
        capture.previous_present = Some(ended);
    }
    capture.frames.push(Frame {
        ended,
        interval,
        acquire: acquired.duration_since(started),
        render_submit: submitted.duration_since(acquired),
        present: ended.duration_since(submitted),
        size,
        error,
        sections: std::mem::take(&mut capture.sections),
        counters: std::mem::take(&mut capture.counters),
    });
}
