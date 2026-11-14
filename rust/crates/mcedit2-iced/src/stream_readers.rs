//! 一个世界流复用固定读取线程和资源分类缓存；每批仍重新冻结磁盘来源。
use crate::{source_visibility, textures::TextureBank};
use iced::futures::channel::mpsc;
use mcedit2_world::{
    block_lookup::SharedRegionBatch,
    editing::Session,
    far_view::{self, FarChunkPatch, FarChunkRequest},
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{sync_channel, SyncSender},
    Arc, Mutex,
};

pub(crate) type Patches = Vec<(FarChunkRequest, Patch)>;
pub(crate) type Patch = Result<(FarChunkPatch, Arc<crate::spectator_stream::PatchCost>), String>;
#[derive(Debug, Clone)]
pub(crate) struct Delivery {
    pub patches: Patches,
    pub finished: bool,
    pub error: Option<String>,
}

/// 共享批次来源，按工作线程宽度成组交接；先完成的来源不等整批最慢区块。
pub(crate) fn deliveries(
    receiver: Result<mpsc::UnboundedReceiver<Event>, String>,
    requests: Vec<FarChunkRequest>,
    group: usize,
    cancelled: Arc<AtomicBool>,
) -> impl iced::futures::Stream<Item = Delivery> {
    use iced::futures::StreamExt;
    assert!(group > 0, "区块交接宽度必须为正数");
    let remaining = requests
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let (receiver, error) = match receiver {
        Ok(receiver) => (Some(receiver), None),
        Err(error) => (None, Some(error)),
    };
    iced::futures::stream::unfold(
        (receiver, remaining, error, false),
        move |(mut receiver, mut remaining, error, done)| {
            let cancelled = Arc::clone(&cancelled);
            async move {
                if done {
                    return None;
                }
                let mut patches = Vec::new();
                let error = if let Some(error) = error {
                    error
                } else {
                    loop {
                        match receiver.as_mut().expect("有效读取持有接收端").next().await {
                            Some(Event::Chunk(request, patch)) => {
                                if !remaining.remove(&request) {
                                    break "读取线程返回了重复或未请求的区块".into();
                                }
                                patches.push((request, patch));
                                if patches.len() >= group && !remaining.is_empty() {
                                    return Some((
                                        Delivery {
                                            patches,
                                            finished: false,
                                            error: None,
                                        },
                                        (receiver, remaining, None, false),
                                    ));
                                }
                            }
                            Some(Event::Finished)
                                if remaining.is_empty() || cancelled.load(Ordering::Acquire) =>
                            {
                                return Some((
                                    Delivery {
                                        patches,
                                        finished: true,
                                        error: None,
                                    },
                                    (receiver, remaining, None, true),
                                ));
                            }
                            Some(Event::Failed(error)) => break error,
                            Some(Event::Finished) => break "读取线程未返回全部区块便结束".into(),
                            None => break "读取线程未返回完成通知".into(),
                        }
                    }
                };
                Some((
                    Delivery {
                        patches,
                        finished: true,
                        error: Some(error),
                    },
                    (receiver, remaining, None, true),
                ))
            }
        },
    )
}
pub(crate) enum Event {
    Chunk(FarChunkRequest, Patch),
    Finished,
    Failed(String),
}
fn cancelled_batch() -> mpsc::UnboundedReceiver<Event> {
    let (sender, receiver) = mpsc::unbounded();
    sender
        .unbounded_send(Event::Finished)
        .expect("新建读取通道必须可写");
    receiver
}

pub(crate) struct Readers {
    source: Arc<Session>,
    lanes: Vec<SyncSender<Option<Arc<Batch>>>>,
    stopped: Arc<AtomicBool>,
    previous: Mutex<Option<Arc<SharedRegionBatch>>>,
}
struct Batch {
    requests: Vec<FarChunkRequest>,
    shared: Arc<SharedRegionBatch>,
    cancelled: Arc<AtomicBool>,
    next: AtomicUsize,
    remaining: AtomicUsize,
    sender: mpsc::UnboundedSender<Event>,
}
impl Drop for Readers {
    fn drop(&mut self) {
        self.stop();
    }
}
impl Readers {
    pub(crate) fn start(
        source: Arc<Session>,
        assets: Option<Arc<TextureBank>>,
        show_textures: bool,
        block_layers: crate::block_visibility::Layers,
        workers: usize,
    ) -> Result<Self, String> {
        #[cfg(test)]
        let prepared = std::env::var_os("MCEDIT2_READ_REFERENCE").is_none();
        #[cfg(not(test))]
        let prepared = true;
        Self::start_inner(
            source,
            assets,
            show_textures,
            block_layers,
            workers,
            prepared,
        )
    }
    fn start_inner(
        source: Arc<Session>,
        assets: Option<Arc<TextureBank>>,
        show_textures: bool,
        block_layers: crate::block_visibility::Layers,
        workers: usize,
        _prepared: bool,
    ) -> Result<Self, String> {
        if workers == 0 {
            return Err("读取线程预算不能为零".into());
        }
        let mut pool = Self {
            source,
            lanes: Vec::with_capacity(workers),
            stopped: Arc::new(AtomicBool::new(false)),
            previous: Mutex::new(None),
        };
        for ordinal in 0..workers {
            let (sender, incoming) = sync_channel::<Option<Arc<Batch>>>(1);
            let source = Arc::clone(&pool.source);
            let stopped = Arc::clone(&pool.stopped);
            let assets = assets.clone();
            std::thread::Builder::new()
                .name(format!("mcedit2-source-reader-{ordinal}"))
                .spawn(move || {
                    let mut visibility = source_visibility::Reader::new(assets, show_textures)
                        .with_layers(block_layers);
                    #[cfg(test)]
                    if !_prepared {
                        visibility.disable_fixed_queries();
                    }

                    while let Ok(request) = incoming.recv() {
                        let Some(batch) = request else { break };
                        if stopped.load(Ordering::Acquire) {
                            batch.cancelled.store(true, Ordering::Release);
                        }
                        batch.run(&source, &mut visibility);
                        batch.finish_lane();
                        if stopped.load(Ordering::Acquire) {
                            break;
                        }
                    }
                })
                .map_err(|e| format!("无法启动世界读取线程 {ordinal}：{e}"))?;
            pool.lanes.push(sender);
        }
        Ok(pool)
    }
    pub(crate) fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        for lane in &self.lanes {
            // 空闲线程收到 None 便退出；队列非空时由 stopped 终止，断开代表已经退出。
            match lane.try_send(None) {
                Ok(()) | Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {}
                Err(std::sync::mpsc::TrySendError::Full(_)) => {}
            }
        }
    }
    pub(crate) fn read(
        &self,
        requests: Vec<FarChunkRequest>,
        cancelled: Arc<AtomicBool>,
        memory: usize,
    ) -> Result<mpsc::UnboundedReceiver<Event>, String> {
        if cancelled.load(Ordering::Acquire) {
            return Ok(cancelled_batch());
        }
        if self.stopped.load(Ordering::Acquire) || requests.is_empty() {
            return Err("读取线程已停止或批次没有来源请求".into());
        }
        let mut shared = self
            .source
            .shared_source_reader(memory)
            .map_err(|e| e.to_string())?;
        let mut previous = self
            .previous
            .lock()
            .map_err(|e| format!("读取复用缓存锁失败：{e}"))?;
        #[cfg(test)]
        if std::env::var_os("MCEDIT2_READ_REFERENCE").is_some() {
            *previous = None;
        }
        if let Some(before) = previous.as_ref() {
            shared
                .reuse_decoded_from(before)
                .map_err(|e| e.to_string())?;
        }
        let shared = Arc::new(shared);
        *previous = Some(Arc::clone(&shared));
        drop(previous);
        let (sender, receiver) = mpsc::unbounded();
        let batch = Arc::new(Batch {
            requests,
            shared,
            cancelled,
            next: AtomicUsize::new(0),
            remaining: AtomicUsize::new(self.lanes.len()),
            sender,
        });
        for lane in &self.lanes {
            if let Err(error) = lane.try_send(Some(Arc::clone(&batch))) {
                if batch.cancelled.load(Ordering::Acquire) || self.stopped.load(Ordering::Acquire) {
                    return Ok(cancelled_batch());
                }
                self.stop();
                return Err(format!("读取批次提交失败：{error}"));
            }
        }
        Ok(receiver)
    }
}
impl Batch {
    fn run(&self, source: &Session, visibility: &mut source_visibility::Reader) {
        // 每批重新冻结 Region；只有逐字节相同的镜像才复用上一批的解码结果。
        let mut reader = match source.source_reader() {
            Ok(reader) => reader,
            Err(error) => {
                if !self.cancelled.swap(true, Ordering::AcqRel) {
                    if self
                        .sender
                        .unbounded_send(Event::Failed(error.to_string()))
                        .is_err()
                    {
                        log::debug!("[terrain] 来源初始化失败时接收端已释放");
                    }
                }
                return;
            }
        };
        // 完整扫描已存 Section；远景只取其中的表面，不能按世界版本截掉自定义高度。
        let (ymin, ymax) = (-2048, 2047);
        while !self.cancelled.load(Ordering::Acquire) {
            let index = self.next.fetch_add(1, Ordering::Relaxed);
            let Some(&request) = self.requests.get(index) else {
                break;
            };
            let result = {
                if request.span == 1 {
                    mcedit2_world::chunk_surface::inspect_full_chunk_parallel_with_visibility(
                        source,
                        &self.shared,
                        &mut reader,
                        request.cx,
                        request.cz,
                        visibility,
                        &|| self.cancelled.load(Ordering::Acquire),
                    )
                } else {
                    far_view::inspect_far_chunk_parallel(
                        source,
                        &mut reader,
                        &self.shared,
                        request,
                        ymin,
                        ymax,
                    )
                }
            }
            .map(|patch| {
                let bytes = Arc::new(crate::spectator_stream::PatchCost::new(&patch));
                (patch, bytes)
            })
            .map_err(|e| e.to_string());
            if self.cancelled.load(Ordering::Acquire) {
                break;
            }
            if self
                .sender
                .unbounded_send(Event::Chunk(request, result))
                .is_err()
            {
                log::debug!("[terrain] 区块完成时视图接收端已释放，停止后续读取");
                self.cancelled.store(true, Ordering::Release);
                break;
            }
        }
    }
    fn finish_lane(&self) {
        if self.remaining.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        if self.sender.unbounded_send(Event::Finished).is_err()
            && !self.cancelled.load(Ordering::Acquire)
        {
            log::debug!("[terrain] 读取结束时视图接收端已释放");
        }
        self.sender.close_channel();
    }
}

/// 仅在完整应用诊断中比较实际读取链路；动态与固定判定输出逐格相等才记录速度。
#[cfg(test)]
pub(crate) fn verify_delivery_coalescing() {
    use iced::futures::{executor::block_on, FutureExt, StreamExt};
    let requests = (0..8)
        .map(|cx| FarChunkRequest { cx, cz: 0, span: 1 })
        .collect::<Vec<_>>();
    let (sender, receiver) = mpsc::unbounded();
    let mut output = Box::pin(deliveries(
        Ok(receiver),
        requests.clone(),
        4,
        Arc::new(AtomicBool::new(false)),
    ));
    for request in &requests[..3] {
        sender
            .unbounded_send(Event::Chunk(*request, Err("受控来源错误".into())))
            .unwrap();
    }
    assert!(
        output.next().now_or_never().is_none(),
        "不足交接宽度时不得逐块刷新"
    );
    sender
        .unbounded_send(Event::Chunk(requests[3], Err("受控来源错误".into())))
        .unwrap();
    let first = block_on(output.next()).unwrap();
    assert_eq!(first.patches.len(), 4);
    assert!(!first.finished);
    for request in &requests[4..7] {
        sender
            .unbounded_send(Event::Chunk(*request, Err("受控来源错误".into())))
            .unwrap();
    }
    sender.unbounded_send(Event::Finished).unwrap();
    let last = block_on(output.next()).unwrap();
    assert!(last.finished);
    assert_eq!(
        last.patches.len(),
        3,
        "只发布真实完成的结果，批次错误单独报告"
    );
    assert!(last.error.as_ref().unwrap().contains("未返回全部"));
    assert!(block_on(output.next()).is_none());
    let mut failed = Box::pin(deliveries(
        Err("无法冻结实际区块来源".into()),
        requests,
        4,
        Arc::new(AtomicBool::new(false)),
    ));
    let failure = block_on(failed.next()).unwrap();
    assert!(failure.finished);
    assert!(failure.patches.is_empty());
    assert_eq!(failure.error.as_deref(), Some("无法冻结实际区块来源"));
    assert!(block_on(failed.next()).is_none());
}

/// 同一真实源的逐格参照与吞吐检查。
#[cfg(test)]
pub(crate) fn compare_throughput(
    source: Arc<Session>,
    assets: Arc<TextureBank>,
    radius: u16,
) -> serde_json::Value {
    use iced::futures::{executor::block_on, StreamExt};
    use std::{collections::BTreeMap, time::Instant};
    let mut requests = crate::spectator_stream::plan([2.5, 215.5, -17.5], radius).unwrap();
    for request in &mut requests {
        request.span = 1;
    }
    let mut reference = BTreeMap::new();
    let mut measurements = Vec::new();
    let workers = crate::terrain_budget::limits().readers;
    for (ordinal, reuse) in [false, false, true, true, false].into_iter().enumerate() {
        let pool = Readers::start_inner(
            Arc::clone(&source),
            Some(Arc::clone(&assets)),
            true,
            Default::default(),
            workers,
            reuse,
        )
        .unwrap();
        let start = Instant::now();
        let mut stored = 0;
        let batch = 12;
        for requests in requests.chunks(batch) {
            if !reuse {
                *pool.previous.lock().unwrap() = None;
            }
            let mut reads = pool
                .read(
                    requests.to_vec(),
                    Arc::new(AtomicBool::new(false)),
                    2 * 1024 * 1024 * 1024,
                )
                .unwrap();
            let mut finished = false;
            while let Some(event) = block_on(reads.next()) {
                match event {
                    Event::Chunk(request, result) => {
                        let (patch, _) = result.unwrap();
                        stored += usize::from(patch.tile.source_present);
                        let key = (request.cx, request.cz, request.span);
                        if let Some(expected) = reference.get(&key) {
                            let expected: &FarChunkPatch = expected;
                            assert_eq!(
                                patch.voxels, expected.voxels,
                                "不同读取调度必须得到相同的方块、面、光照和对象来源"
                            );
                            assert_eq!(patch.tile.source_present, expected.tile.source_present);
                            assert_eq!(patch.tile.unknown_columns, expected.tile.unknown_columns);
                            match (&patch.sections, &expected.sections) {
                                (Some(actual), Some(expected)) => {
                                    assert_eq!(actual.len(), expected.len());
                                    for (actual, expected) in actual.iter().zip(expected.iter()) {
                                        assert_eq!(actual.y, expected.y);
                                        assert_eq!(actual.connections, expected.connections);
                                        for y in 0..16 {
                                            for z in 0..16 {
                                                for x in 0..16 {
                                                    assert_eq!(
                                                        actual.opaque(x, y, z),
                                                        expected.opaque(x, y, z),
                                                        "固定模型分类不得改变空域剔除的整格占用"
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                (None, None) => {}
                                _ => panic!("固定分类不得改变 Section 拓扑是否存在"),
                            }
                        } else {
                            reference.insert(key, patch);
                        }
                    }
                    Event::Failed(error) => panic!("真实来源读取失败：{error}"),
                    Event::Finished => {
                        finished = true;
                    }
                }
            }
            assert!(finished);
        }
        let seconds = start.elapsed().as_secs_f64();
        if ordinal == 0 {
            continue;
        } // 独立预热并建立逐格参照，不计入速度样本。
        measurements.push(serde_json::json!({"verified_decode_reuse":reuse,"fixed_palette_visibility":reuse,"batch_size":batch,
            "stored_chunks":stored,"requests":requests.len(),"seconds":seconds,"chunks_per_second":stored as f64/seconds}));
    }
    serde_json::json!({"radius":radius,"workers":workers,"scope":"同一真实存档、完整高度与材质；动态可见性对照 / 调色板固定判定与受镜像核对的解码复用；无窗口、无 GPU 建模上传", "samples":measurements})
}
