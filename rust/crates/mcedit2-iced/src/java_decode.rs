//! 有界图片解码流水线：归档读取和结果发布保持源顺序，工作线程只接触独立图片。
use super::{png, LoadControl, Source, TextureBank};
use crate::resource_budget::ResourceBudget;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

struct Job {
    sequence: usize,
    id: String,
    source: Source,
    metadata_source: Option<Source>,
    data: Vec<u8>,
    plan: png::Plan,
    reserved: usize,
    legacy_ratio: Option<(u32, u32)>,
}
struct Decoded {
    sequence: usize,
    id: String,
    source: Source,
    metadata_source: Option<Source>,
    reserved: usize,
    output: png::Output,
}
struct Pool {
    sender: Option<SyncSender<Job>>,
    receiver: Receiver<Result<Decoded, String>>,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}
impl Pool {
    fn new(count: usize, control: &LoadControl) -> Result<Self, String> {
        let (sender, requests) = mpsc::sync_channel::<Job>(count);
        let requests = Arc::new(Mutex::new(requests));
        let (results, receiver) = mpsc::channel();
        let mut pool = Self {
            sender: Some(sender),
            receiver,
            stop: Arc::new(AtomicBool::new(false)),
            workers: Vec::new(),
        };
        for number in 0..count {
            let requests = Arc::clone(&requests);
            let results = results.clone();
            let stop = Arc::clone(&pool.stop);
            let cancellation = Arc::clone(&control.cancelled);
            let worker = std::thread::Builder::new().name(format!("mcedit2-png-{number}"))
                .spawn(move || {
                    let cancelled = || stop.load(Ordering::Acquire) || cancellation.load(Ordering::Acquire);
                    loop {
                        if cancelled() { break; }
                        let job = match requests.lock() {
                            Ok(queue) => queue.recv(),
                            Err(error) => {
                                let reason = format!("图片解码队列锁异常：{error}");
                                log::info!("[assets] {reason}");
                                let _ = results.send(Err(reason));
                                stop.store(true, Ordering::Release);
                                break;
                            }
                        };
                        let Ok(job) = job else { break; };
                        if cancelled() { break; }
                        let mut output = job.plan.decode(&job.data, &cancelled);
                        if let Some((width,height)) = job.legacy_ratio {
                            let dimensions = output.animation.as_ref().map(|sprite| &sprite.source)
                                .or_else(|| output.simple.as_ref().and_then(|value| value.as_ref().ok()))
                                .or_else(|| output.material.as_ref().and_then(|value| value.as_ref().ok()).map(|value| &value.0))
                                .map(|tile| [tile.width,tile.height]);
                            if let Some([actual_width,actual_height]) = dimensions {
                                if u64::from(actual_width)*u64::from(height) != u64::from(actual_height)*u64::from(width) {
                                    let error = format!("旧箱子实体整图布局要求 {width}:{height}，实际为 {actual_width}×{actual_height}，拒绝错贴");
                                    output.material = Some(Err(error.clone()));
                                    output.simple = Some(Err(error));
                                    output.animation = None;
                                }
                            }
                        }
                        let decoded=Decoded {sequence:job.sequence,id:job.id,source:job.source,metadata_source:job.metadata_source,reserved:job.reserved,output};
                        if results.send(Ok(decoded)).is_err() {break;}

                    }
                }).map_err(|e|format!("无法启动图片解码线程：{e}"))?;
            pool.workers.push(worker);
        }
        Ok(pool)
    }
    fn submit(&self, job: Job) -> Result<Option<Job>, String> {
        if self.stop.load(Ordering::Acquire) {
            return Err("图片解码线程已停止，请查看资源错误日志".into());
        }
        match self
            .sender
            .as_ref()
            .ok_or("图片解码队列已关闭")?
            .try_send(job)
        {
            Ok(()) => Ok(None),
            Err(TrySendError::Full(job)) => Ok(Some(job)),
            Err(TrySendError::Disconnected(_)) => Err("图片解码线程已退出".into()),
        }
    }
    fn receive(&self, control: &LoadControl) -> Result<Decoded, String> {
        loop {
            control.check()?;
            match self.receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self.workers.iter().any(JoinHandle::is_finished) {
                        return Err("图片解码工作线程提前退出；请查看线程错误日志".into());
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("图片解码队列在结果发布前关闭".into())
                }
            }
        }
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.sender.take();
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                log::info!("[assets] 回收图片解码线程时发现未处理的 panic");
            }
        }
    }
}

struct Publication {
    pending: BTreeMap<usize, Decoded>,
    next: usize,
    active: usize,
    in_flight: usize,
    peak: usize,
    memory: usize,
    rgba_bytes: usize,
    auxiliary_bytes: usize,
    models: usize,
    source_bytes: usize,
    budget: ResourceBudget,
    total: usize,
    progress: Instant,
}
impl Publication {
    fn check(&self, extra: usize) -> Result<(), String> {
        let resident = self
            .memory
            .checked_add(self.auxiliary_bytes)
            .and_then(|n| n.checked_add(self.source_bytes))
            .and_then(|n| n.checked_add(self.in_flight))
            .and_then(|n| n.checked_add(extra))
            .ok_or("资源解码工作集大小溢出")?;
        self.budget.check(self.models, resident, 0)
    }
    fn admit(
        &mut self,
        extra: usize,
        pool: &Pool,
        bank: &mut TextureBank,
        control: &LoadControl,
    ) -> Result<(), String> {
        loop {
            control.check()?;
            match self.check(extra) {
                Ok(()) => return Ok(()),
                Err(error) if self.active == 0 => return Err(error),
                Err(_) => self.receive(pool, bank, control)?,
            }
        }
    }
    fn receive(
        &mut self,
        pool: &Pool,
        bank: &mut TextureBank,
        control: &LoadControl,
    ) -> Result<(), String> {
        let decoded = pool.receive(control)?;
        let sequence = decoded.sequence;
        if sequence < self.next || self.pending.insert(sequence, decoded).is_some() {
            return Err("图片解码结果重复发布".into());
        }
        while let Some(decoded) = self.pending.remove(&self.next) {
            control.check()?;
            self.in_flight = self
                .in_flight
                .checked_sub(decoded.reserved)
                .ok_or("解码预留计量失配")?;
            self.active = self.active.checked_sub(1).ok_or("解码任务计量失配")?;
            let font_bytes = decoded
                .output
                .font
                .as_ref()
                .and_then(|v| v.as_ref().ok())
                .map_or(0, |v| v.rgba.len());
            let material_bytes = decoded
                .output
                .material
                .as_ref()
                .and_then(|v| v.as_ref().ok())
                .map_or(0, |(v, _)| v.rgba.len());
            let animation_bytes = decoded
                .output
                .animation
                .as_ref()
                .map_or(0, |sprite| sprite.bytes());
            self.check(
                font_bytes
                    .checked_add(material_bytes)
                    .and_then(|n| n.checked_add(animation_bytes))
                    .ok_or("图片输出计量溢出")?,
            )?;
            self.publish(decoded, bank)?;
            self.next += 1;
            if self.progress.elapsed() >= Duration::from_millis(150) || self.next == self.total {
                control.report(format!("解码图片 {}/{}", self.next, self.total))?;
                self.progress = Instant::now();
            }
        }
        Ok(())
    }
    fn retain(&mut self, bytes: usize) -> Result<(), String> {
        self.check(bytes)?;
        self.memory = self.memory.checked_add(bytes).ok_or("图片常驻计量溢出")?;
        Ok(())
    }
    fn retain_rgba(&mut self, bytes: usize) -> Result<(), String> {
        self.retain(bytes)?;
        self.rgba_bytes = self
            .rgba_bytes
            .checked_add(bytes)
            .ok_or("图片 RGBA 计量溢出")?;
        Ok(())
    }
    fn publish(&mut self, decoded: Decoded, bank: &mut TextureBank) -> Result<(), String> {
        let png::Output {
            layout,
            font,
            simple,
            sampler,
            material,
            diagnostics,
            animation,
        } = decoded.output;
        let origin = decoded.metadata_source.as_ref().map_or_else(
            || decoded.source.description(),
            |meta| {
                format!(
                    "{}；元数据 {}",
                    decoded.source.description(),
                    meta.description()
                )
            },
        );
        for diagnostic in diagnostics {
            let message = format!("纹理 {}：{diagnostic}；来源 {origin}", decoded.source.asset);
            let bytes = message.len() + std::mem::size_of::<String>();
            self.check(bytes)?;
            self.auxiliary_bytes = self
                .auxiliary_bytes
                .checked_add(bytes)
                .ok_or("图片诊断常驻计量溢出")?;
            log::info!("[assets] {message}");
            Arc::make_mut(&mut bank.warnings).push(message);
        }
        let font_image = font.is_some();
        if let Some(layout) = layout {
            self.retain(decoded.id.len() + std::mem::size_of::<(String, [u8; 2])>() + 64)?;
            Arc::make_mut(&mut bank.png_layouts).insert(decoded.id.clone(), layout);
        }
        if let Some(sampler) = sampler {
            let bytes = decoded.id.len()
                + std::mem::size_of::<(String, Result<super::texture_sampler::Sampler, String>)>()
                + 64
                + sampler.as_ref().err().map_or(0, String::len);
            self.retain(bytes)?;
            if let Err(error) = &sampler {
                let message = format!(
                    "整图采样元数据 {} 无效：{error}；来源 {origin}",
                    decoded.source.asset
                );
                self.retain(message.len() + std::mem::size_of::<String>())?;
                log::info!("[assets] {message}");
                Arc::make_mut(&mut bank.warnings).push(message);
            }
            Arc::make_mut(&mut bank.simple_samplers).insert(decoded.id.clone(), sampler);
        }
        let atlas_image = material.as_ref().is_some_and(Result::is_ok);
        if let Some(simple) = simple {
            match simple {
                Ok(tile) => {
                    self.retain_rgba(tile.rgba.len())?;
                    if !font_image && !atlas_image {
                        Arc::make_mut(&mut bank.statistics).decoded_png += 1;
                    }
                    Arc::make_mut(&mut bank.simple_tiles).insert(decoded.id.clone(), tile);
                }
                Err(error) => {
                    let error = format!(
                        "整图纹理 {} 无效：{error}；来源 {origin}",
                        decoded.source.asset
                    );
                    log::info!("[assets] {error}");
                    Arc::make_mut(&mut bank.warnings).push(error);
                }
            }
        }
        if let Some(result) = font {
            match result {
                Ok(tile) => {
                    self.retain_rgba(tile.rgba.len())?;
                    Arc::make_mut(&mut bank.fonts).insert_image(decoded.id.clone(), tile);
                    Arc::make_mut(&mut bank.statistics).decoded_png += 1;
                }
                Err(error) => {
                    log::info!(
                        "[assets] 字体图片 {} 无效：{error}；来源 {origin}",
                        decoded.source.asset
                    );
                    Arc::make_mut(&mut bank.statistics).failed_png += 1;
                    Arc::make_mut(&mut bank.warnings).push(format!(
                        "字体图片 {} 无效：{error}；来源 {origin}",
                        decoded.source.asset
                    ));
                }
            }
        }
        if let Some(result) = material {
            match result {
                Ok((tile, _animated)) => {
                    if decoded.source.colormap.is_some()
                        && (tile.width != 256 || tile.height != 256)
                    {
                        Arc::make_mut(&mut bank.statistics).failed_png += 1;
                        Arc::make_mut(&mut bank.warnings).push(format!(
                            "颜色表 {} 必须为 256×256，实际 {}×{}",
                            decoded.source.asset, tile.width, tile.height
                        ));
                        return Ok(());
                    }
                    self.retain_rgba(tile.rgba.len())?;
                    match decoded.source.colormap {
                        Some("grass") => Arc::make_mut(&mut bank.tints).grass_colormap = Some(tile),
                        Some("foliage") => {
                            Arc::make_mut(&mut bank.tints).foliage_colormap = Some(tile)
                        }
                        Some(_) => return Err("未知颜色表".into()),
                        None => {
                            if !font_image {
                                Arc::make_mut(&mut bank.statistics).decoded_png += 1;
                            }
                            if let Some(sprite) = animation {
                                self.retain_rgba(sprite.source.rgba.len())?;
                                let metadata = sprite.bytes() - sprite.source.rgba.len();
                                self.check(metadata)?;
                                self.auxiliary_bytes = self
                                    .auxiliary_bytes
                                    .checked_add(metadata)
                                    .ok_or("动画时序常驻计量溢出")?;
                                Arc::make_mut(&mut bank.live_animations)
                                    .insert(decoded.id.clone(), Arc::new(sprite));
                            }
                            Arc::make_mut(&mut bank.tiles).insert(decoded.id, tile);
                        }
                    }
                }
                Err(error) => {
                    log::info!(
                        "[assets] 纹理 {} 无效：{error}；来源 {origin}",
                        decoded.source.asset
                    );
                    Arc::make_mut(&mut bank.statistics).failed_png += 1;
                    Arc::make_mut(&mut bank.warnings).push(format!(
                        "纹理 {} 无效：{error}；来源 {origin}",
                        decoded.source.asset
                    ));
                }
            }
        }
        Ok(())
    }
}

fn read_texture(
    mut reader: impl Read,
    expected: u64,
    control: &LoadControl,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(expected as usize)
        .map_err(|e| format!("无法分配 PNG 输入：{e}"))?;
    let mut chunk = [0u8; 65536];
    loop {
        control.check()?;
        let count = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if bytes
            .len()
            .checked_add(count)
            .is_none_or(|n| n as u64 > expected)
        {
            return Err("PNG 在建立资源索引后改变了长度，请重新加载资源".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    if bytes.len() as u64 != expected {
        return Err("PNG 在建立资源索引后改变了长度，请重新加载资源".into());
    }
    Ok(bytes)
}

pub(super) fn decode(
    mut bank: TextureBank,
    sources: BTreeMap<String, Source>,
    metadata_sources: BTreeMap<String, Source>,
    font_images: &BTreeSet<String>,
    atlas_ids: &BTreeSet<String>,
    source_bytes: usize,
    budget: ResourceBudget,
    control: &LoadControl,
) -> Result<(TextureBank, usize), String> {
    let started = Instant::now();
    let total = sources.len();
    if total == 0 {
        return Ok((bank, 0));
    }
    let workers = crate::terrain_budget::limits().readers.min(total).max(1);
    let pool = Pool::new(workers, control)?;
    let mut publication = Publication {
        pending: BTreeMap::new(),
        next: 0,
        active: 0,
        in_flight: 0,
        peak: 0,
        memory: 0,
        rgba_bytes: 0,
        auxiliary_bytes: 0,
        models: bank.models.source_bytes(),
        source_bytes,
        budget,
        total,
        progress: Instant::now(),
    };
    let mut groups: BTreeMap<PathBuf, Vec<(String, Source)>> = BTreeMap::new();
    for (id, source) in sources {
        groups
            .entry(source.file.clone())
            .or_default()
            .push((id, source));
    }
    let mut sequence = 0;
    for (path, sources) in groups {
        control.check()?;
        let mut opened = if sources.first().is_some_and(|(_, s)| s.member.is_some()) {
            Some(crate::pack_sources::open(
                &path,
                |extra| publication.admit(extra, &pool, &mut bank, control),
                || control.check(),
            )?)
        } else {
            None
        };
        let archive_bytes = opened.as_ref().map_or(0, |source| source.resident);
        publication.auxiliary_bytes = publication
            .auxiliary_bytes
            .checked_add(archive_bytes)
            .ok_or("PNG 嵌套归档工作集溢出")?;
        for (id, source) in sources {
            control.check()?;
            while publication.active >= workers * 2 {
                publication.receive(&pool, &mut bank, control)?;
            }
            let input = usize::try_from(source.length).map_err(|_| "PNG 输入超过地址范围")?;
            let metadata = bank
                .metadata
                .get(&id)
                .map_or(0, Vec::len)
                .checked_mul(8)
                .ok_or("PNG 元数据计量溢出")?;
            publication.admit(
                input.checked_add(metadata).ok_or("PNG 读取工作集溢出")?,
                &pool,
                &mut bank,
                control,
            )?;
            let data = if let Some(member) = &source.member {
                let member = opened
                    .as_mut()
                    .ok_or("PNG 归档未打开")?
                    .archive
                    .by_name(member)
                    .map_err(|e| e.to_string())?;
                read_texture(member, source.length, control)?
            } else {
                read_texture(
                    File::open(&source.file).map_err(|e| e.to_string())?,
                    source.length,
                    control,
                )?
            };
            let font = font_images.contains(&id);
            let material = !font || atlas_ids.contains(&id) || source.colormap.is_some();
            let metadata_source = if bank.metadata.contains_key(&id) {
                Some(
                    metadata_sources
                        .get(&id)
                        .ok_or("动画元数据缺少来源证明")?
                        .clone(),
                )
            } else {
                None
            };
            let plan = png::Plan::new(
                &data,
                bank.metadata.get(&id).map(Vec::as_slice),
                font,
                material,
            );
            let reserved = plan.working_bytes(data.len())?;
            publication.admit(reserved, &pool, &mut bank, control)?;
            let legacy_ratio = bank.models.legacy_chest_sheet_ratio(&id);
            let mut job = Job {
                sequence,
                id,
                source,
                metadata_source,
                data,
                plan,
                reserved,
                legacy_ratio,
            };
            loop {
                control.check()?;
                match pool.submit(job)? {
                    None => break,
                    Some(pending) => {
                        job = pending;
                        publication.receive(&pool, &mut bank, control)?;
                    }
                }
            }
            publication.active += 1;
            publication.in_flight = publication
                .in_flight
                .checked_add(reserved)
                .ok_or("PNG 在途计量溢出")?;
            publication.peak = publication.peak.max(publication.in_flight);
            sequence += 1;
        }
        drop(opened);
        publication.auxiliary_bytes = publication
            .auxiliary_bytes
            .checked_sub(archive_bytes)
            .ok_or("PNG 嵌套归档工作集计量失配")?;
    }
    while publication.active != 0 {
        publication.receive(&pool, &mut bank, control)?;
    }
    control.check()?;
    log::info!(
        "[assets] PNG 解码：{total} 项，{workers} 工作线程，在途预留峰值 {} MiB，{:?}",
        publication.peak.div_ceil(1024 * 1024),
        started.elapsed()
    );
    Arc::make_mut(&mut bank.statistics).decoded_bytes = publication.rgba_bytes;
    Ok((bank, publication.memory))
}
