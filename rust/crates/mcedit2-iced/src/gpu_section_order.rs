//! 独立常驻的透明源和索引。Section 之间排序只重排绘制命令。
use super::*;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
};

pub(in crate::gpu_scene) type Key = ([i32; 3], bool);
pub(in crate::gpu_scene) fn key(part: ChunkKey) -> Key {
    let (id, _, origin) = part;
    if id.2 == 1 {
        (origin, true)
    } else {
        ([id.0 * 16, origin[1], id.1 * 16], false)
    }
}
type Metadata = Arc<[world::LocalFace]>;
type SortOutput = (Arc<Vec<world::TransparentFace>>, transparent::PreparedOrder);

pub(in crate::gpu_scene) struct Resident {
    keys: Vec<ChunkKey>,
    origin: [i32; 3],
    center: [f32; 3],
    sources: transparent::Sources,
    faces: Arc<Vec<world::TransparentFace>>,
    state: Mutex<State>,
}
impl Resident {
    pub fn matches(&self, keys: &[ChunkKey], origin: [i32; 3]) -> bool {
        self.origin == origin && self.keys.as_slice() == keys
    }
    pub fn uses_only(&self, wanted: &std::collections::HashSet<ChunkKey>) -> bool {
        self.keys.iter().all(|key| wanted.contains(key))
    }
}

struct State {
    key: world::SortKey,
    order: transparent::Order,
    camera_job: Option<CameraSort>,
}

#[derive(Default)]
pub(super) struct Orders {
    pub ready: BTreeMap<Key, Arc<Resident>>,
    queue: Option<VecDeque<(Key, Vec<usize>)>>,
    running: Vec<(Key, Build)>,
    animation: Option<AnimationScan>,
    pub materials: Option<BTreeMap<Key, atlas::AnimatedMaterials>>,
    pub animation_sources: BTreeMap<Key, Arc<AnimationSource>>,
}

pub(in crate::gpu_scene) struct AnimationSource {
    keys: Vec<ChunkKey>,
    atlas: Option<Arc<crate::textures::TextureAtlas>>,
    used: atlas::AnimatedMaterials,
}

struct Worker<T> {
    dispatch: crate::render_work::Task,
    result: Mutex<mpsc::Receiver<Result<T, String>>>,
    cancel: Arc<AtomicBool>,
}
impl<T: Send + 'static> Worker<T> {
    fn new(
        name: &'static str,
        run: impl FnOnce(&AtomicBool) -> Result<T, String> + Send + 'static,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        let dispatch = crate::render_work::spawn_source(name, Arc::clone(&cancel), move || {
            let _ = sender.send(run(&flag));
            crate::render_wake::notify();
        })?;
        Ok(Self {
            dispatch,
            result: Mutex::new(receiver),
            cancel,
        })
    }
    fn poll(&mut self) -> Result<Option<T>, String> {
        if !self.dispatch.submit()? {
            return Ok(None);
        }
        match self
            .result
            .get_mut()
            .expect("Section 工作结果锁被毒化")
            .try_recv()
        {
            Ok(result) => result.map(Some),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err("Section 工作线程意外退出".into()),
        }
    }
}
impl<T> Drop for Worker<T> {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
type AnimationScan = Worker<BTreeMap<Key, Arc<AnimationSource>>>;

struct Build {
    keys: Vec<ChunkKey>,
    allocations: Vec<Arc<residency::Allocation>>,
    placements: Vec<[f32; 4]>,
    metadata: Vec<Metadata>,
    sources: transparent::Sources,
    job: Option<Worker<SortOutput>>,
    faces: Option<Arc<Vec<world::TransparentFace>>>,
    prepared: Option<transparent::PreparedOrder>,
    order: transparent::Order,
    camera: Option<crate::spectator::CameraUniform>,
}
impl Build {
    fn new(
        upload: &Pending,
        indices: &[usize],
        camera: Option<&crate::spectator::CameraUniform>,
    ) -> Self {
        Self {
            keys: indices.iter().map(|&i| upload.geometry_keys[i]).collect(),
            allocations: indices
                .iter()
                .map(|&i| Arc::clone(&upload.allocations[i]))
                .collect(),
            placements: indices.iter().map(|&i| upload.placements[i]).collect(),
            metadata: indices
                .iter()
                .map(|&i| {
                    Arc::clone(
                        upload.chunks[&upload.geometry_keys[i]]
                            .transparent_metadata
                            .as_ref()
                            .expect("已准备 Section 的透明源元数据"),
                    )
                })
                .collect(),
            sources: Default::default(),
            job: None,
            faces: None,
            prepared: None,
            order: Default::default(),
            camera: camera.copied().map(world::SortKey::reference),
        }
    }

    fn advance(
        &mut self,
        pipeline: &Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        groups: &Arc<[usize]>,
        origin: [i32; 3],
        id: Key,
        budget: &mut Budget,
    ) -> Result<Option<Arc<Resident>>, String> {
        if !self.sources.advance(
            &self.allocations,
            &self.placements,
            &pipeline.zero_offset.buffer,
            &pipeline.transparent_residency,
            device,
            queue,
            &pipeline.transparent_layout,
            budget,
        )? {
            return Ok(None);
        }
        if self.job.is_none() && self.faces.is_none() {
            let sources = self.sources.clone();
            let metadata = std::mem::take(&mut self.metadata);
            let allocations = self.allocations.clone();
            let offsets = self.placements.clone();
            let layout = transparent::OrderLayout::for_device(device, Arc::clone(groups));
            let camera = self.camera;
            self.job = Some(Worker::new("transparent-section", move |flag| {
                let cancelled = || flag.load(Ordering::Acquire);
                let mut faces = Vec::new();
                for (placement, ((allocation, offset), local)) in
                    allocations.iter().zip(offsets).zip(metadata).enumerate()
                {
                    let delta = [offset[0], offset[1], offset[2]];
                    let index =
                        u32::try_from(placement).map_err(|_| "Section 透明位置数量超出 u32")?;
                    if local.len() != allocation.len() {
                        return Err("Section 透明元数据与常驻源不一致".into());
                    }
                    for (ordinal, face) in local.iter().enumerate() {
                        if ordinal.is_multiple_of(1024) && cancelled() {
                            return Err("Section 透明准备已取消".into());
                        }
                        faces.push(face.placed(
                            delta,
                            sources.address(allocation.address(ordinal)?)?,
                            index,
                        ));
                    }
                }
                let mut sort = world::Sorting::acquire();
                if !sort.sort(&faces, camera.as_ref(), &cancelled) {
                    return Err("Section 透明排序已取消".into());
                }
                let prepared = transparent::PreparedOrder::prepare(
                    &faces,
                    sort.indices(),
                    layout,
                    &cancelled,
                )?
                .ok_or("Section 透明排序已取消")?;
                Ok((Arc::new(faces), prepared))
            })?);
        }
        if let Some(job) = &mut self.job {
            let Some((faces, order)) = job.poll()? else {
                return Ok(None);
            };
            order.validate_sources(&self.sources)?;
            self.faces = Some(faces);
            self.prepared = Some(order);
            self.job = None;
        }
        let order = self.prepared.as_mut().expect("Section 索引已准备");
        if !self.order.advance(order, device, queue, budget)? {
            return Ok(None);
        }
        let [x, y, z] = origin_delta(id.0, origin);
        Ok(Some(Arc::new(Resident {
            keys: std::mem::take(&mut self.keys),
            origin,
            center: [x + 8., y + 8., z + 8.],
            sources: std::mem::take(&mut self.sources),
            faces: self.faces.take().expect("Section 透明面已就绪"),
            state: Mutex::new(State {
                key: world::SortKey::camera(self.camera.as_ref()),
                order: std::mem::take(&mut self.order),
                camera_job: None,
            }),
        })))
    }
}

impl Pending {
    pub(super) fn advance_section_orders(
        &mut self,
        pipeline: &Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: Option<&crate::spectator::CameraUniform>,
        budget: &mut Budget,
    ) -> Result<bool, String> {
        let mesh = self.target.stream_mesh.as_ref().expect("Section 交接有源");
        let origin = mesh.origin;
        if self.section_orders.queue.is_none() {
            let mut by_section = BTreeMap::<Key, Vec<usize>>::new();
            for (i, &part) in self.geometry_keys.iter().enumerate() {
                by_section.entry(key(part)).or_default().push(i);
            }
            let mut work: Vec<_> = by_section.into_iter().collect();
            work.sort_by_key(|(_, parts)| parts[0]);
            let mut waiting = VecDeque::new();
            let groups = self.page_groups.as_ref().expect("Section 有实际图集分组");
            let hints = self.hints.as_ref().expect("Section 有实际材质提示");
            for (id, parts) in work {
                let reusable = |resident: &&Arc<Resident>| {
                    resident.origin == origin
                        && resident
                            .keys
                            .iter()
                            .copied()
                            .eq(parts.iter().map(|&i| self.geometry_keys[i]))
                };
                let old = self
                    .staged_section_orders
                    .get(&id)
                    .filter(reusable)
                    .or_else(|| {
                        parts
                            .iter()
                            .all(|&i| {
                                pipeline
                                    .stream_chunks
                                    .get(&self.geometry_keys[i])
                                    .is_some_and(|old| {
                                        old.hints.compatible(
                                            hints,
                                            &old.page_groups,
                                            groups,
                                            old.used_pages.iter().copied(),
                                        )
                                    })
                            })
                            .then(|| pipeline.section_orders.get(&id))
                            .flatten()
                            .filter(reusable)
                    });
                if let Some(old) = old {
                    self.section_orders.ready.insert(id, Arc::clone(old));
                } else {
                    waiting.push_back((id, parts));
                }
            }
            self.section_orders.queue = Some(waiting);
        }
        if self.section_orders.materials.is_none() {
            if self.section_orders.animation.is_none() {
                let parts = mesh.chunks.clone();
                let source = self.target.atlas.clone();
                let terrain = mesh.terrain.as_ref().unwrap_or(mesh).atlas.clone();
                let previous = pipeline.section_animations.clone();
                self.section_orders.animation =
                    Some(Worker::new("section-animation", move |flag| {
                        let mut animated = BTreeMap::new();
                        let mut sections = BTreeMap::<_, Vec<_>>::new();
                        for part in parts.iter() {
                            if part.geometry().ordered().is_empty()
                                && part.geometry().blended().is_empty()
                            {
                                continue;
                            }
                            sections.entry(key(chunk_key(part))).or_default().push(part);
                        }
                        for (id, parts) in sections {
                            if flag.load(Ordering::Acquire) {
                                break;
                            }
                            let source = if id.1 { &terrain } else { &source };
                            let keys: Vec<_> = parts.iter().map(|p| chunk_key(p)).collect();
                            if let Some(old) = previous.get(&id).filter(|old| {
                                old.keys == keys
                                    && (same_arc(old.atlas.as_ref(), source.as_ref())
                                        || source.as_ref().zip(old.atlas.as_ref()).is_some_and(
                                            |(next, prior)| {
                                                crate::textures::atlas_compatible(next, prior)
                                            },
                                        ))
                            }) {
                                animated.insert(id, Arc::clone(old));
                                continue;
                            }
                            let used = atlas::animated_materials(
                                source.as_deref(),
                                parts.into_iter().flat_map(|part| {
                                    part.geometry()
                                        .ordered()
                                        .iter()
                                        .chain(part.geometry().blended().iter())
                                        .map(|&i| {
                                            let quad = &part.quads()[i];
                                            (quad.atlas_page, quad.uv)
                                        })
                                }),
                                &|| flag.load(Ordering::Acquire),
                            )?;
                            animated.insert(
                                id,
                                Arc::new(AnimationSource {
                                    keys,
                                    atlas: source.clone(),
                                    used,
                                }),
                            );
                        }
                        Ok(animated)
                    })?);
            }
            if let Some(materials) = self
                .section_orders
                .animation
                .as_mut()
                .expect("动画工作已排队")
                .poll()?
            {
                self.section_orders.materials = Some(
                    materials
                        .iter()
                        .map(|(id, source)| (*id, source.used.clone()))
                        .collect(),
                );
                self.section_orders.animation_sources = materials;
                self.section_orders.animation = None;
                self.region_revision += 1;
            }
        }
        // 小 Section 不按每四个任务等待一次界面帧；固定线程池保持有界预取，
        // 完成后在同一帧补入下一批，仍受原有上传时间与字节预算限制。
        loop {
            while self.section_orders.running.len() < crate::render_work::CAPACITY * 2
                && !budget.exhausted()
            {
                let pending = self
                    .section_orders
                    .queue
                    .as_ref()
                    .expect("Section 队列已排好");
                let Some(next) = pending.iter().position(|(_, indices)| {
                    indices.iter().all(|&i| {
                        self.allocations[i].complete()
                            && self.chunks[&self.geometry_keys[i]]
                                .transparent_metadata
                                .is_some()
                    })
                }) else {
                    break;
                };
                let (id, indices) = self
                    .section_orders
                    .queue
                    .as_mut()
                    .unwrap()
                    .remove(next)
                    .unwrap();
                let job = Build::new(self, &indices, camera);
                self.section_orders.running.push((id, job));
            }
            let groups = self.page_groups.as_ref().expect("Section 有实际图集分组");
            let mut index = 0;
            let mut completed = false;
            while index < self.section_orders.running.len() && !budget.exhausted() {
                let (id, job) = &mut self.section_orders.running[index];
                if let Some(resident) =
                    job.advance(pipeline, device, queue, groups, origin, *id, budget)?
                {
                    let (id, _) = self.section_orders.running.remove(index);
                    self.section_orders.ready.insert(id, resident);
                    self.region_revision += 1;
                    completed = true;
                } else {
                    index += 1;
                }
            }
            if !completed || budget.exhausted() {
                break;
            }
        }
        if !self.section_orders.running.is_empty()
            || !self
                .section_orders
                .queue
                .as_ref()
                .expect("Section 队列存在")
                .is_empty()
            || self.section_orders.materials.is_none()
        {
            return Ok(false);
        }
        self.animated = self
            .regions
            .values()
            .flat_map(|region| region.animated.iter().copied())
            .chain(
                self.section_orders
                    .materials
                    .as_ref()
                    .unwrap()
                    .values()
                    .flatten()
                    .copied(),
            )
            .collect();
        Ok(true)
    }
}

pub(in crate::gpu_scene) fn resort(
    base: &BTreeMap<Key, Arc<Resident>>,
    preview: Option<&super::progressive::Preview>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    camera: Option<crate::spectator::CameraUniform>,
    groups: Arc<[usize]>,
    budget: &mut Budget,
    columns: Option<&std::collections::HashSet<(i32, i32)>>,
) -> bool {
    let camera = camera.map(world::SortKey::reference);
    let wanted = world::SortKey::camera(camera.as_ref());
    let active = residents(base, preview, camera.as_ref(), columns);
    let mut running = active
        .iter()
        .filter(|r| {
            r.state
                .lock()
                .expect("Section 索引锁被毒化")
                .camera_job
                .is_some()
        })
        .count();
    let mut changed = false;
    // 先为相机附近的 Section 更新索引，保持较远的可用索引。
    for resident in active.into_iter().rev() {
        if budget.exhausted() {
            break;
        }
        let mut state = resident.state.lock().expect("Section 索引锁被毒化");
        if state.camera_job.is_none() && state.key != wanted && running < 2 {
            state.camera_job = Some(CameraSort::new(
                Arc::clone(&resident.faces),
                camera,
                device,
                &mut state.order,
                Arc::clone(&groups),
                resident.sources.clone(),
            ));
            running += 1;
        }
        if let Some(mut job) = state.camera_job.take() {
            match job.advance(&resident.sources, device, queue, budget) {
                Ok(true) => {
                    state.key = job.key;
                    let old = std::mem::replace(&mut state.order, job.finish());
                    state.order.recycle(old);
                    changed = true;
                    running -= 1;
                }
                Ok(false) => state.camera_job = Some(job),
                Err(error) => {
                    log::error!("[gpu-sort] Section 透明索引更新失败：{error}");
                    running -= 1;
                }
            }
        }
    }
    changed
}

fn residents<'a>(
    base: &'a BTreeMap<Key, Arc<Resident>>,
    preview: Option<&'a super::progressive::Preview>,
    camera: Option<&crate::spectator::CameraUniform>,
    columns: Option<&std::collections::HashSet<(i32, i32)>>,
) -> Vec<&'a Resident> {
    let visible = |key: &Key| {
        !key.1
            || columns.is_none_or(|columns| {
                columns.contains(&(key.0[0].div_euclid(16), key.0[2].div_euclid(16)))
            })
    };
    let mut active: Vec<_> = base
        .iter()
        .filter(|(key, _)| visible(key) && !preview.is_some_and(|p| p.replaced.contains(key)))
        .map(|(_, resident)| resident.as_ref())
        .collect();
    if let Some(preview) = preview {
        active.extend(
            preview
                .transparent
                .iter()
                .filter(|(key, _)| visible(key))
                .map(|(_, resident)| resident.as_ref()),
        );
    }
    if let Some(camera) = camera {
        active.sort_by(|a, b| {
            world::sort_distance(b.center, camera)
                .total_cmp(&world::sort_distance(a.center, camera))
        });
    }
    active
}

pub(in crate::gpu_scene) fn commands(
    base: &BTreeMap<Key, Arc<Resident>>,
    preview: Option<&super::progressive::Preview>,
    camera: Option<&crate::spectator::CameraUniform>,
    device: &wgpu::Device,
    bindings: &Arc<commands::Bindings>,
    offset: &wgpu::BindGroup,
    view: Option<&crate::frustum::Frustum>,
    tripwire: bool,
    columns: Option<&std::collections::HashSet<(i32, i32)>>,
    mut append: impl FnMut(commands::Prepared),
) {
    for resident in residents(base, preview, camera, columns) {
        let mut state = resident.state.lock().expect("Section 索引锁被毒化");
        for segment in &mut state.order.segments {
            if let Some(command) = segment.prepare_commands(
                device,
                bindings,
                offset,
                &resident.sources,
                view,
                tripwire,
            ) {
                append(command);
            }
        }
    }
}
