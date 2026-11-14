//! 有界闲置缓存的稳定整数索引排序；不持有源几何或 GPU 资源。
#[derive(Default)]
struct SortWorkspace {
    keys: Vec<u32>,
    scratch: Vec<usize>,
    order: Vec<usize>,
}
fn sort_pool() -> &'static std::sync::Mutex<Option<SortWorkspace>> {
    static POOL: std::sync::OnceLock<std::sync::Mutex<Option<SortWorkspace>>> =
        std::sync::OnceLock::new();
    POOL.get_or_init(|| std::sync::Mutex::new(None))
}
fn lock_sort_pool() -> std::sync::MutexGuard<'static, Option<SortWorkspace>> {
    sort_pool().lock().expect("排序工作区缓存锁被毒化")
}
/// 空间索引与绘制顺序共享的后台排序工作区；取消后仍通过 Drop 回收安全的数组容量。
pub(crate) struct Sorting {
    workspace: Option<SortWorkspace>,
}
impl Sorting {
    /// 无相机的源序无需生成键或工作直方图。
    pub fn source_order(&mut self, count: usize, cancelled: &dyn Fn() -> bool) -> bool {
        let work = self.workspace.as_mut().expect("排序工作区尚未归还");
        work.order.clear();
        work.order.extend(0..count);
        !cancelled()
    }
    pub fn acquire() -> Self {
        Self {
            workspace: Some(lock_sort_pool().take().unwrap_or_default()),
        }
    }
    pub fn indices(&self) -> &[usize] {
        &self.workspace.as_ref().expect("排序工作区尚未归还").order
    }
    /// 相同的稳定基数索引器也供空间元数据使用，不复制源面或改变绘制顺序。
    pub fn sort_indices(
        &mut self,
        count: usize,
        key: impl Fn(usize) -> u32,
        cancelled: &dyn Fn() -> bool,
    ) -> bool {
        if cancelled() {
            return false;
        }
        let work = self.workspace.as_mut().expect("排序工作区尚未归还");
        // 每次从源顺序开始，不能让上一个镜头的等深度顺序覆盖原始绘制顺序。
        work.order.clear();
        work.order.extend(0..count);
        work.keys.clear();
        work.keys.reserve(count);
        let mut histograms = [[0usize; 256]; 4];
        for index in 0..count {
            if index.is_multiple_of(1024) && cancelled() {
                return false;
            }
            let key = key(index);
            work.keys.push(key);
            // 一次顺序遍历同时建立四位直方图，省掉每一轮沿排序索引随机读取键来计数。
            for (pass, shift) in [0, 8, 16, 24].into_iter().enumerate() {
                histograms[pass][((key >> shift) & 255) as usize] += 1;
            }
        }
        work.scratch.resize(count, 0);
        for (pass, shift) in [0, 8, 16, 24].into_iter().enumerate() {
            if cancelled() {
                return false;
            }
            let mut offsets = histograms[pass];
            // 相同字节不改变稳定顺序；跳过该轮也不改变 total_cmp 降序的语义。
            if offsets.iter().any(|n| *n == count) {
                continue;
            }
            let mut total = 0;
            for count in &mut offsets {
                let n = *count;
                *count = total;
                total += n;
            }
            for (position, &index) in work.order.iter().enumerate() {
                if position.is_multiple_of(4096) && cancelled() {
                    return false;
                }
                let slot = &mut offsets[((work.keys[index] >> shift) & 255) as usize];
                work.scratch[*slot] = index;
                *slot += 1;
            }
            std::mem::swap(&mut work.order, &mut work.scratch);
        }
        !cancelled()
    }
}
impl Drop for Sorting {
    fn drop(&mut self) {
        let Some(workspace) = self.workspace.take() else {
            return;
        };
        let bytes = workspace.keys.capacity().checked_mul(4).and_then(|n| {
            workspace
                .scratch
                .capacity()
                .checked_add(workspace.order.capacity())
                .and_then(|count| count.checked_mul(std::mem::size_of::<usize>()))
                .and_then(|other| n.checked_add(other))
        });
        // 只约束闲置缓存；大排序照常执行，超预算容量在任务完成后释放。
        if bytes.is_none_or(|bytes| bytes > 64 * 1024 * 1024) {
            return;
        }
        // 保留 scratch 的已初始化长度，下次同规模排序无需再次清零整个数组。
        // order 和 keys 会在任务开始时重建；缓存不持有任何源网格或 GPU 分配引用。
        let mut slot = lock_sort_pool();
        if slot
            .as_ref()
            .is_none_or(|old| old.order.capacity() < workspace.order.capacity())
        {
            *slot = Some(workspace);
        }
    }
}
