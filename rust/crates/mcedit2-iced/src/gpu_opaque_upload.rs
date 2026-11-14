//! 源分类、材质批次、无损编码及高度片索引全部在后台准备。
//! 有界队列限制在途 CPU 数据；取消会释放接收端并解除工作线程的发送等待。
use super::super::material_hints::Hints;
use super::world;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError},
    Arc, Mutex,
};

/// 小面批次配合较深的队列，保持与原队列相同的面载荷预算，消除每帧只交接少数 Section 的气泡。
pub(super) struct QueueLimits {
    pub events: usize,
    pub requests: usize,
    pub segment_bytes: usize,
    pub geometry_events: usize,
    pub geometry_faces: usize,
}
pub(super) fn queue_limits() -> QueueLimits {
    #[cfg(test)]
    if std::env::var_os("MCEDIT2_READ_REFERENCE").is_some() {
        return QueueLimits {
            events: 2,
            requests: 2,
            segment_bytes: 8 << 20,
            geometry_events: 4,
            geometry_faces: 8192,
        };
    }
    QueueLimits {
        events: 64,
        requests: 32,
        segment_bytes: 256 << 10,
        geometry_events: 32,
        geometry_faces: 1024,
    }
}
pub(super) enum Event {
    Batch {
        chunk: usize,
        segment: world::PreparedSegment,
    },
    Complete {
        chunk: usize,
        blended: Arc<Vec<usize>>,
    },
    Regional(RegionalEvent),
}
pub(super) enum RegionalEvent {
    Batch {
        region: usize,
        segment: world::PreparedSegment,
    },
    Complete {
        region: usize,
        animated: Arc<super::atlas::AnimatedMaterials>,
    },
}
pub(super) struct Worker {
    cancelled: Arc<AtomicBool>,
    lanes: Vec<Lane>,
    /// 源请求的发布顺序。后续线程即使先完成也不能越过前面的区块。
    pending: VecDeque<usize>,
    request_limit: usize,
    segment_bytes: usize,
    ready_lane: usize,
}
struct Lane {
    receiver: Mutex<Receiver<Result<Event, String>>>,
    requests: SyncSender<Request>,
    in_flight: usize,
}
enum Request {
    Source {
        chunk: usize,
        source: Arc<crate::mesh_geometry::Geometry>,
        count: usize,
        groups: Arc<[usize]>,
        hints: Arc<Hints>,
    },
    Region {
        index: usize,
        source: Arc<super::regional::Definition>,
        count: usize,
        y32: bool,
        groups: Arc<[usize]>,
        hints: Arc<Hints>,
        atlas: Option<Arc<crate::textures::TextureAtlas>>,
    },
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl Worker {
    pub fn start() -> Result<Self, String> {
        let limits = queue_limits();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut worker = Self {
            cancelled,
            lanes: Vec::new(),
            pending: VecDeque::new(),
            request_limit: limits.requests,
            segment_bytes: limits.segment_bytes,
            ready_lane: 0,
        };
        for ordinal in 0..crate::terrain_budget::limits().gpu_preparers {
            let signal = Arc::clone(&worker.cancelled);
            // 每线程面载荷最多 16 MiB；提前准备多个小 Section，界面仍按既有帧预算上传。
            let (sender, receiver) = crate::render_wake::bounded(limits.events);
            let (requests, incoming) = sync_channel::<Request>(limits.requests);
            std::thread::Builder::new()
                .name(format!("mcedit2-opaque-prepare-{ordinal}"))
                .spawn(move || {
                    let cancelled = || signal.load(Ordering::Acquire);
                    while let Ok(request) = incoming.recv() {
                        if cancelled() {
                            break;
                        }
                        let result = match request {
                            Request::Source {
                                chunk,
                                source,
                                count,
                                groups,
                                hints,
                            } => {
                                prepare(chunk, &source, count, &groups, &hints, &sender, &cancelled)
                            }
                            Request::Region {
                                index,
                                source,
                                count,
                                y32,
                                groups,
                                hints,
                                atlas,
                            } => super::regional::prepare(
                                index,
                                &source,
                                count,
                                y32,
                                &groups,
                                &hints,
                                atlas.as_deref(),
                                &sender,
                                &cancelled,
                            ),
                        };
                        if let Err(error) = result {
                            if !cancelled() {
                                log::error!("[gpu-upload] 后台不透明网格准备失败：{error}");
                                let _ = sender.send(Err(error));
                            }
                            break;
                        }
                    }
                })
                .map_err(|e| format!("无法启动不透明网格准备线程：{e}"))?;
            worker.lanes.push(Lane {
                receiver: Mutex::new(receiver),
                requests,
                in_flight: 0,
            });
        }
        Ok(worker)
    }
    /// 空闲线程并行准备不同区块；所有线程忙碌时立即让出。
    pub fn request(
        &mut self,
        chunk: usize,
        source: Arc<crate::mesh_geometry::Geometry>,
        bytes: usize,
        groups: Arc<[usize]>,
        hints: Arc<Hints>,
    ) -> Result<bool, String> {
        let count = world::segment_faces(bytes.min(self.segment_bytes) as u64);
        if count == 0 {
            return Err("GPU 上传预算不足以容纳一个完整源面".into());
        }
        self.submit(Request::Source {
            chunk,
            source,
            count,
            groups,
            hints,
        })
    }
    pub fn region(
        &mut self,
        index: usize,
        source: Arc<super::regional::Definition>,
        bytes: usize,
        y32: bool,
        groups: Arc<[usize]>,
        hints: Arc<Hints>,
        atlas: Option<Arc<crate::textures::TextureAtlas>>,
    ) -> Result<bool, String> {
        let bytes = bytes.min(self.segment_bytes);
        let count = (bytes / std::mem::size_of::<super::super::encoding::RegionalFull>())
            .min(u32::MAX as usize);
        if count == 0 {
            return Err("GPU 上传预算不足以容纳一个区域源面".into());
        }
        self.submit(Request::Region {
            index,
            source,
            count: bytes,
            y32,
            groups,
            hints,
            atlas,
        })
    }
    fn submit(&mut self, request: Request) -> Result<bool, String> {
        let Some(index) = self
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane.in_flight < self.request_limit)
            .min_by_key(|(_, lane)| lane.in_flight)
            .map(|(index, _)| index)
        else {
            return Ok(false);
        };
        let lane = &mut self.lanes[index];
        match lane.requests.try_send(request) {
            Ok(()) => {
                lane.in_flight += 1;
                self.pending.push_back(index);
                Ok(true)
            }
            Err(TrySendError::Full(_)) => Err("空闲不透明准备线程仍持有旧请求".into()),
            Err(TrySendError::Disconnected(_)) => Err("不透明网格准备线程已退出".into()),
        }
    }
    pub fn poll(&mut self) -> Result<Option<Event>, String> {
        let Some(&index) = self.pending.front() else {
            return Err("不透明准备尚未提交请求便要求发布结果".into());
        };
        let lane = &mut self.lanes[index];
        match lane
            .receiver
            .get_mut()
            .map_err(|_| "不透明网格准备队列锁异常")?
            .try_recv()
        {
            Ok(Ok(event)) => {
                if matches!(
                    event,
                    Event::Complete { .. } | Event::Regional(RegionalEvent::Complete { .. })
                ) {
                    lane.in_flight -= 1;
                    self.pending.pop_front();
                }
                Ok(Some(event))
            }
            Ok(Err(error)) => Err(error),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("不透明网格准备线程未发布完整结果便退出".into()),
        }
    }
    /// 普通区域互不依赖；任一线程的就绪结果都可上传，源内部顺序仍由该线程保证。
    pub fn poll_ready(&mut self) -> Result<Option<Event>, String> {
        let lane_count = self.lanes.len();
        for offset in 0..lane_count {
            let index = (self.ready_lane + offset) % lane_count;
            let lane = &mut self.lanes[index];
            if lane.in_flight == 0 {
                continue;
            }
            match lane
                .receiver
                .get_mut()
                .map_err(|_| "不透明网格准备队列锁异常")?
                .try_recv()
            {
                Ok(Ok(event)) => {
                    self.ready_lane = (index + 1) % lane_count;
                    if matches!(
                        event,
                        Event::Complete { .. } | Event::Regional(RegionalEvent::Complete { .. })
                    ) {
                        lane.in_flight -= 1;
                        let position = self
                            .pending
                            .iter()
                            .position(|&p| p == index)
                            .ok_or("区域完成结果没有对应的在途请求")?;
                        self.pending.remove(position);
                    }
                    return Ok(Some(event));
                }
                Ok(Err(error)) => return Err(error),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    return Err("不透明网格准备线程未发布完整结果便退出".into())
                }
            }
        }
        Ok(None)
    }
}
#[cfg(test)]
pub(super) fn verify_ready_delivery() {
    let (first, receiver0) = crate::render_wake::bounded(2);
    let (second, receiver1) = crate::render_wake::bounded(2);
    let (requests0, _input0) = sync_channel(1);
    let (requests1, _input1) = sync_channel(1);
    let mut worker = Worker {
        cancelled: Arc::new(AtomicBool::new(false)),
        lanes: vec![
            Lane {
                receiver: Mutex::new(receiver0),
                requests: requests0,
                in_flight: 1,
            },
            Lane {
                receiver: Mutex::new(receiver1),
                requests: requests1,
                in_flight: 1,
            },
        ],
        pending: VecDeque::from([0, 1]),
        request_limit: 1,
        segment_bytes: 256 << 10,
        ready_lane: 0,
    };
    // 第一条通道尚未完成，第二条的真实通道结果必须立即返回。
    assert!(second
        .send(Ok(Event::Regional(RegionalEvent::Complete {
            region: 9,
            animated: Arc::new(Default::default())
        })))
        .is_ok());
    assert!(matches!(
        worker.poll_ready().unwrap(),
        Some(Event::Regional(RegionalEvent::Complete { region: 9, .. }))
    ));
    assert_eq!(worker.pending, VecDeque::from([0]));
    assert!(first
        .send(Ok(Event::Regional(RegionalEvent::Complete {
            region: 8,
            animated: Arc::new(Default::default())
        })))
        .is_ok());
    assert!(matches!(
        worker.poll_ready().unwrap(),
        Some(Event::Regional(RegionalEvent::Complete { region: 8, .. }))
    ));
    assert!(worker.pending.is_empty());
    assert!(worker.poll_ready().unwrap().is_none());
}

fn prepare(
    chunk: usize,
    source: &crate::mesh_geometry::Geometry,
    count: usize,
    groups: &[usize],
    hints: &Hints,
    sender: &crate::render_wake::Sender<Result<Event, String>>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let blended = Arc::clone(source.blended());
    let ordered = source.ordered();
    let quads = source.quads();
    // 有序不透明层逐字保留原顺序及完整编码，不为批次优化重排共面内容。
    for batch in ordered.chunks(count) {
        if cancelled() {
            return Ok(());
        }
        let mut encoded = Vec::with_capacity(batch.len());
        for (position, &index) in batch.iter().enumerate() {
            if position.is_multiple_of(1024) && cancelled() {
                return Ok(());
            }
            encoded.push(hints.encode(quads[index]));
        }
        let Some(segment) = world::PreparedSegment::prepare(
            quads,
            batch,
            super::super::encoding::Encoded::Complete(encoded),
            None,
            groups,
            cancelled,
        )?
        else {
            return Ok(());
        };
        if sender.send(Ok(Event::Batch { chunk, segment })).is_err() {
            return Ok(());
        }
    }
    if !cancelled() {
        let _ = sender.send(Ok(Event::Complete { chunk, blended }));
    }
    Ok(())
}
