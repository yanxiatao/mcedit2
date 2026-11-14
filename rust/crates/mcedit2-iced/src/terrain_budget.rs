//! 将地形读取和建模分配到同一 CPU 预算，避免两个流水阶段各自占满处理器。
use std::sync::OnceLock;

pub(crate) struct Limits {
    pub readers: usize,
    pub meshers: usize,
    pub gpu_preparers: usize,
}

pub(crate) fn limits() -> &'static Limits {
    static LIMITS: OnceLock<Limits> = OnceLock::new();
    LIMITS.get_or_init(|| {
        let cores = match std::thread::available_parallelism() {
            Ok(count) => count.get(),
            Err(error) => {
                log::warn!("[terrain] 无法查询 CPU 并行度，读取和建模各使用一个线程：{error}");
                1
            }
        };
        // 三个流水阶段至少各一个；为界面、输入和透明排序保留核心。
        // 上传准备与读取、建模共用预算，不能每一阶段各自占满处理器。
        let slots = cores.saturating_sub(4).clamp(3, if crate::spectator_stream::serial_reference() { 10 } else { 24 });
        let gpu_preparers = if slots >= 12 { 4 } else if slots >= 6 { 2 } else { 1 };
        let readers = ((slots - gpu_preparers) / 2).min(8);
        let meshers = slots - gpu_preparers - readers;
        log::debug!("[terrain] {cores} 个逻辑核心，读取上限 {readers}，建模上限 {meshers}，GPU 准备上限 {gpu_preparers}");
        Limits { readers, meshers, gpu_preparers }
    })
}
