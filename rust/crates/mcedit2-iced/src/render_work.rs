//! 相机排序、场景排序和空域掩码共用固定工作线程；取消的任务不再另起线程。
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
};

struct Job {
    source: bool,
    cancelled: Arc<AtomicBool>,
    work: Box<dyn FnOnce() + Send>,
}
pub(crate) struct Task {
    name: &'static str,
    pending: Mutex<Option<Job>>,
}
impl Task {
    /// 队列满时保留任务所有权，让视图继续显示旧帧，下一次唤醒再提交。
    pub(crate) fn submit(&mut self) -> Result<bool, String> {
        let pending = self
            .pending
            .get_mut()
            .map_err(|e| format!("{} 待提交任务锁失败：{e}", self.name))?;
        if pending.is_none() {
            return Ok(true);
        }
        let queue = queue()?;
        let mut jobs = queue
            .jobs
            .lock()
            .map_err(|e| format!("视图任务队列锁失败：{e}"))?;
        let source = pending.as_ref().expect("任务尚待提交").source;
        let matching = jobs.iter().filter(|job| job.source == source).count();
        if jobs.len() >= CAPACITY
            || matching
                >= if source {
                    SOURCE_CAPACITY
                } else {
                    VIEW_CAPACITY
                }
        {
            return Ok(false);
        }
        jobs.push_back(pending.take().expect("任务尚待提交"));
        drop(jobs);
        queue.ready.notify_one();
        Ok(true)
    }
}
struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    ready: Condvar,
    closed: AtomicBool,
}
pub(crate) const CAPACITY: usize = 32;
const SOURCE_CAPACITY: usize = 24;
const VIEW_CAPACITY: usize = 16;

fn take_next(jobs: &mut VecDeque<Job>, source_streak: &mut usize) -> Option<Job> {
    let preferred_source = *source_streak < 3;
    let position = jobs
        .iter()
        .position(|job| job.source == preferred_source)
        .unwrap_or(0);
    let job = jobs.remove(position)?;
    if job.source {
        *source_streak += 1;
    } else {
        *source_streak = 0;
    }
    Some(job)
}

fn queue() -> Result<&'static Arc<Queue>, String> {
    static QUEUE: OnceLock<Result<Arc<Queue>, String>> = OnceLock::new();
    QUEUE
        .get_or_init(|| {
            let queue = Arc::new(Queue {
                jobs: Mutex::new(VecDeque::new()),
                ready: Condvar::new(),
                closed: AtomicBool::new(false),
            });
            let workers = std::thread::available_parallelism()
                .map_err(|e| format!("无法查询视图工作线程预算：{e}"))?
                .get()
                .saturating_sub(2)
                .clamp(1, 2);
            for ordinal in 0..workers {
                let shared = Arc::clone(&queue);
                let started = std::thread::Builder::new()
                    .name(format!("mcedit2-view-work-{ordinal}"))
                    .spawn(move || {
                        let mut source_streak = 0;
                        loop {
                            let job = {
                                let mut jobs = shared.jobs.lock().expect("视图任务队列锁失败");
                                while jobs.is_empty() && !shared.closed.load(Ordering::Acquire) {
                                    jobs =
                                        shared.ready.wait(jobs).expect("等待视图任务时队列锁失败");
                                }
                                if shared.closed.load(Ordering::Acquire) {
                                    return;
                                }
                                take_next(&mut jobs, &mut source_streak).expect("工作队列非空")
                            };
                            if job.cancelled.load(Ordering::Acquire) {
                                crate::render_wake::notify();
                                continue;
                            }
                            (job.work)();
                            crate::render_wake::notify();
                        }
                    });
                if let Err(error) = started {
                    // 初始化失败时也停止已经启动的线程，不留下没有调用方的等待者。
                    let jobs = queue
                        .jobs
                        .lock()
                        .map_err(|e| format!("关闭失败的视图线程池时锁异常：{e}"))?;
                    queue.closed.store(true, Ordering::Release);
                    drop(jobs);
                    queue.ready.notify_all();
                    return Err(format!("无法启动视图工作线程 {ordinal}：{error}"));
                }
            }
            Ok(queue)
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(crate) fn spawn(
    name: &'static str,
    cancelled: Arc<AtomicBool>,
    work: impl FnOnce() + Send + 'static,
) -> Result<Task, String> {
    spawn_with_priority(name, cancelled, work, false)
}

pub(crate) fn spawn_source(
    name: &'static str,
    cancelled: Arc<AtomicBool>,
    work: impl FnOnce() + Send + 'static,
) -> Result<Task, String> {
    spawn_with_priority(name, cancelled, work, true)
}

fn spawn_with_priority(
    name: &'static str,
    cancelled: Arc<AtomicBool>,
    work: impl FnOnce() + Send + 'static,
    source: bool,
) -> Result<Task, String> {
    let mut task = Task {
        name,
        pending: Mutex::new(Some(Job {
            source,
            cancelled,
            work: Box::new(work),
        })),
    };
    task.submit()?;
    Ok(task)
}

#[cfg(test)]
pub(crate) fn verify_scheduling() {
    let result = Arc::new(Mutex::new(Vec::new()));
    let mut jobs = VecDeque::new();
    for (id, source) in [
        (0, false),
        (1, true),
        (2, true),
        (3, true),
        (4, true),
        (5, false),
    ] {
        let result = Arc::clone(&result);
        jobs.push_back(Job {
            source,
            cancelled: Arc::new(AtomicBool::new(false)),
            work: Box::new(move || result.lock().unwrap().push(id)),
        });
    }
    let mut streak = 0;
    while let Some(job) = take_next(&mut jobs, &mut streak) {
        (job.work)();
    }
    assert_eq!(*result.lock().unwrap(), [1, 2, 3, 0, 4, 5]);
}
