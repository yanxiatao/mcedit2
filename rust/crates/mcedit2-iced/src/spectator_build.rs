//! 共享只读邻域的有界并行建模；完整列就绪即可参与交接，最终结果保持源顺序。
use super::*;
use crate::{chunk_mesh_cache, spectator_stream};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

pub(super) struct Shared<'a> {
    pub view: &'a SpectatorScene,
    pub sources: &'a [&'a [SurfaceVoxel]],
    pub frame: SourceFrame<'a>,
    pub neighbors: Option<&'a spectator_stream::ExactNeighbors<'a>>,
    pub contexts: Option<&'a [spectator_stream::ChunkContext]>,
    pub style: chunk_mesh_cache::Style,
    pub distant: models::DistantElevation<'a>,
    pub chests: crate::legacy_chest_context::Context,
    pub templates: &'a templates::Cache,
    pub previous: &'a HashMap<(i32, i32, u8, usize, usize), MeshChunk>,
}

#[derive(Default)]
pub(super) struct Stats {
    pub reused: usize,
    pub rebuilt: usize,
    pub preparation: Duration,
    pub emission: Duration,
    pub indexing: Duration,
    pub storing: Duration,
    pub template_preparation: Duration,
    pub templates: usize,
    pub model_hidden: usize,
    pub templates_used: templates::Cache,
    pub unchanged: usize,
}
impl Stats {
    fn add(&mut self, other: Self) {
        self.reused += other.reused;
        self.rebuilt += other.rebuilt;
        self.preparation += other.preparation;
        self.emission += other.emission;
        self.indexing += other.indexing;
        self.storing += other.storing;
        self.template_preparation += other.template_preparation;
        self.templates += other.templates;
        self.model_hidden += other.model_hidden;
        self.unchanged += other.unchanged;
        for (state, variants) in other.templates_used {
            self.templates_used
                .entry(state)
                .or_default()
                .extend(variants);
        }
    }
}

type Completed = (usize, Vec<MeshChunk>);

/// 只调度缓存失效的来源，线程和完成队列共用地形预算。
pub(super) fn build(
    shared: Shared<'_>,
    mut ready: Vec<Option<Vec<MeshChunk>>>,
    cancelled: &dyn Fn() -> bool,
    publish: &mut dyn FnMut(&[MeshChunk]),
) -> Option<(Vec<MeshChunk>, Stats, usize)> {
    let mut jobs = ready
        .iter()
        .enumerate()
        .filter_map(|(index, cached)| cached.is_none().then_some(index))
        .collect::<Vec<_>>();
    let reused = ready.iter().flatten().map(Vec::len).sum();
    // 限定窗口本就在网格后台线程中运行，继续逐块发布并即时观察调用方的取消。
    // 完整流式 patch 才需要额外的并行工作线程和源序重排。
    if shared.contexts.is_none() {
        let cursor = AtomicUsize::new(0);
        let (sender, receiver) = mpsc::channel();
        let mut partial = Vec::new();
        let mut source = None;
        let mut prefix = 0;
        let mut stats = run(
            &shared,
            &jobs,
            &cursor,
            cancelled,
            &sender,
            &mut |index, chunks| {
                if source != Some(index) {
                    prefix = partial.len();
                    source = Some(index);
                }
                partial.truncate(prefix);
                partial.extend_from_slice(chunks);
                publish(&partial);
            },
        )?;
        stats.reused += reused;
        drop(sender);
        let chunks = receiver
            .into_iter()
            .flat_map(|(_, chunks)| chunks)
            .collect();
        return (!cancelled()).then_some((chunks, stats, usize::from(!jobs.is_empty())));
    }
    let displayed = shared
        .view
        .presented
        .lock()
        .expect("世界呈现状态锁被毒化")
        .clone();
    let known: std::collections::HashSet<_> = displayed
        .iter()
        .flat_map(|m| m.chunks.iter())
        .filter(|p| p.key.2 == 1)
        .map(|p| (p.key.0, p.key.1))
        .collect();
    jobs.sort_by_key(|&index| {
        index == 0
            || shared
                .contexts
                .and_then(|c| c.get(index - 1))
                .is_some_and(|c| known.contains(&c.coordinates()))
    });
    let workers = crate::terrain_budget::limits().meshers.min(jobs.len());
    let cursor = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (sender, receiver) = mpsc::channel::<Completed>();
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        let mut failed = false;
        for index in 0..workers {
            let input = &shared;
            let jobs = &jobs;
            let cursor = &cursor;
            let stop = &stop;
            let sender = sender.clone();
            match std::thread::Builder::new()
                .name(format!("mcedit2-mesh-{index}"))
                .spawn_scoped(scope, move || {
                    let result = run(
                        input,
                        jobs,
                        cursor,
                        &|| stop.load(Ordering::Acquire),
                        &sender,
                        &mut |_, _| {},
                    );
                    if result.is_none() {
                        stop.store(true, Ordering::Release);
                    }
                    result
                }) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    log::error!("[mesh] 无法启动建模线程 {index}：{error}");
                    stop.store(true, Ordering::Release);
                    failed = true;
                    break;
                }
            }
        }
        drop(sender);
        let mut remaining = jobs.len();
        while !failed && remaining > 0 {
            if cancelled() || stop.load(Ordering::Acquire) {
                stop.store(true, Ordering::Release);
                failed = true;
                break;
            }
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok((index, part)) => {
                    if index >= ready.len() || ready[index].is_some() {
                        log::error!("[mesh] 拒绝重复或无效的 patch 完成结果：{index}");
                        stop.store(true, Ordering::Release);
                        failed = true;
                    } else {
                        ready[index] = Some(part);
                        remaining -= 1;
                        // 完整列成组发布，不再让旧邻列的慢任务挡住已完成的新列。
                        if (jobs.len() - remaining).is_multiple_of(4) || remaining == 0 {
                            let partial = ready
                                .iter()
                                .flatten()
                                .flat_map(|v| v.iter().cloned())
                                .collect::<Vec<_>>();
                            publish(&partial);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if !stop.load(Ordering::Acquire) {
                        log::error!("[mesh] 建模结果通道提前关闭，仍有 {remaining} 个来源未完成");
                    }
                    stop.store(true, Ordering::Release);
                    failed = true;
                }
            }
        }
        let mut stats = Stats {
            reused,
            ..Stats::default()
        };
        for handle in handles {
            match handle.join() {
                Ok(Some(result)) => stats.add(result),
                Ok(None) => failed = true,
                Err(error) => {
                    log::error!("[mesh] 建模线程异常退出：{error:?}");
                    stop.store(true, Ordering::Release);
                    failed = true;
                }
            }
        }
        (!failed && !cancelled()).then(|| {
            (
                ready.into_iter().flatten().flatten().collect(),
                stats,
                workers,
            )
        })
    })
}

#[inline(never)]
fn run(
    shared: &Shared<'_>,
    jobs: &[usize],
    cursor: &AtomicUsize,
    cancelled: &dyn Fn() -> bool,
    sender: &mpsc::Sender<Completed>,
    publish: &mut dyn FnMut(usize, &[MeshChunk]),
) -> Option<Stats> {
    let mut render_palette = mcedit2_world::surface::VoxelPalette::default();
    let view = shared.view;
    let sources = shared.sources;
    let contexts = shared.contexts;
    let neighbors = shared.neighbors;
    let distant = &shared.distant;
    let style = &shared.style;
    let SourceFrame { origin, y_max, .. } = shared.frame;
    #[cfg(test)]
    let use_templates = !shared.frame.reference;
    #[cfg(not(test))]
    let use_templates = true;
    let atlas = view.texture_bank.as_ref().and_then(|b| b.atlas());
    let default_uv = atlas
        .map(|a| {
            [
                a.white.sample(0., 0.),
                a.white.sample(1., 0.),
                a.white.sample(1., 1.),
                a.white.sample(0., 1.),
            ]
        })
        .unwrap_or([[0.5, 0.5]; 4]);
    let bank = view.texture_bank.as_deref();
    let mut colors = bank.map(|bank| bank.tint_context(shared.frame.biomes));
    let mut fluid_materials = crate::fluid_material::Cache::new(bank);
    let mut reused = 0;
    let mut rebuilt = 0;
    let mut preparation = Duration::ZERO;
    let mut emission = Duration::ZERO;
    let mut indexing = Duration::ZERO;
    let mut storing = Duration::ZERO;
    let mut template_preparation = Duration::ZERO;
    let section = |v: &SurfaceVoxel| source_chunk_group(v).map(|(x, z)| (x, z, v.y.div_euclid(16)));
    let sorted_near = std::cell::OnceCell::new();
    let mut variants = templates::Variants::default();
    let mut model_occlusion = crate::model_occlusion::Cache::default();
    let mut templates: HashMap<templates::Key<'_>, Option<Arc<templates::Template>>> =
        HashMap::new();
    let mut templates_used = templates::Cache::new();
    let mut unchanged = 0;
    let mut cache: HashMap<ModelCacheKey<'_>, Vec<models::Part>> = HashMap::new();
    let mut static_cache: HashMap<
        ModelCacheKey<'_>,
        Option<Vec<crate::textures::StaticObjTriangle>>,
    > = HashMap::new();
    let mut styles: HashMap<LodFaceStyleKey<'_>, Option<crate::gpu_mesh::FaceStyle>> =
        HashMap::new();
    let mut visibility: HashMap<LodFaceStyleKey<'_>, FaceVisibility> = HashMap::new();
    let mut internal: HashMap<LodFaceStyleKey<'_>, bool> = HashMap::new();
    let mut repeating: HashMap<LodFaceStyleKey<'_>, RepeatingFace> = HashMap::new();
    loop {
        if cancelled() {
            return None;
        }
        let slot = cursor.fetch_add(1, Ordering::AcqRel);
        let Some(&source_index) = jobs.get(slot) else {
            break;
        };
        let source = &sources[source_index];
        let chunks = (|| {
            let mut chunks = Vec::new();
            let first_chunk = chunks.len();
            let context = source_index
                .checked_sub(1)
                .and_then(|index| contexts.and_then(|contexts| contexts.get(index)));
            // 近景读取按列逐层扫描。先稳定聚合同一 Section；完整区块来源已经按 Section 排列。
            let source = if source_index == 0 && !source.is_sorted_by_key(section) {
                sorted_near
                    .get_or_init(|| {
                        let mut sorted = source.to_vec();
                        sorted.sort_by_key(section);
                        sorted
                    })
                    .as_slice()
            } else {
                source
            };
            for (run_index, original) in source
                .chunk_by(|left, right| section(left) == section(right))
                .enumerate()
            {
                if cancelled() {
                    return None;
                }
                let prepare_started = std::time::Instant::now();
                // Keep original source arrays immutable; only rewrite a chunk's
                // affected face flags if exact neighboring chunks prove occlusion.
                let corrected = (neighbors.is_some() || !shared.chests.is_empty())
                    .then(|| {
                        // Most source runs have unchanged original face masks. Avoid
                        // allocating a masks Vec for every loaded chunk: copy the run
                        // only after the first actual seam correction is found.
                        let mut corrected: Option<Vec<SurfaceVoxel>> = None;
                        for (offset, voxel) in original.iter().enumerate() {
                            let faces = neighbors.map_or(voxel.faces, |index| {
                                if use_templates
                                    && bank.is_some()
                                    && context.is_some_and(|c| c.source.sections.is_some())
                                    && voxel.kind != VoxelMaterial::Liquid
                                    && voxel.fluid_surface().is_none()
                                {
                                    // 资源感知的完整读取已用真实九宫格和位置变体剔面，不再重做同一模型查询。
                                    voxel.faces
                                } else if bank.is_some() {
                                    model_occlusion.faces(
                                        bank,
                                        index,
                                        voxel,
                                        voxel.faces,
                                        view.show_textures,
                                    )
                                } else {
                                    index.faces(voxel)
                                }
                            });
                            let state = shared.chests.state(voxel);
                            if faces != voxel.faces || !Arc::ptr_eq(state, &voxel.state) {
                                let patched = corrected.get_or_insert_with(|| original.to_vec());
                                patched[offset] = render_palette.updated(voxel, |data| {
                                    data.faces = faces;
                                    data.state = Arc::clone(state);
                                });
                            }
                        }
                        corrected
                    })
                    .flatten();
                let run = corrected.as_deref().unwrap_or(original);
                let chunk = source_chunk_group(&run[0]);
                let first = &run[0];
                let origin = if first.span == 1 {
                    [
                        first.x.div_euclid(16) * 16,
                        first.y.div_euclid(16) * 16,
                        first.z.div_euclid(16) * 16,
                    ]
                } else {
                    origin
                };
                // The position in the arriving patch list is NOT a stable GPU
                // identity. Its index shifts whenever a different patch completes.
                // 精确 Section 使用高度身份，删除低处 Section 不会移动上方缓存键。
                let chunk_id = (
                    first.x.div_euclid(16),
                    first.z.div_euclid(16),
                    first.span,
                    usize::from(source_index != 0),
                    if first.span == 1 {
                        first.y.div_euclid(16) as u32 as usize
                    } else {
                        run_index
                    },
                );
                let mut out = Vec::new();
                let mut biome_dependent = false;
                let mut pick_groups = Vec::new();
                let mut previous = None;
                let id = chunk.map(|(cx, cz)| (cx, cz, origin[1], i32::from(context.is_some()), 0));
                if let Some(id) = id.filter(|_| {
                    shared.chests.is_empty()
                        || original.iter().zip(run).all(|(a, b)| a.state == b.state)
                }) {
                    let mut cache = view.chunk_cache.lock().expect("世界网格构建状态锁被毒化");
                    let cached = cache.get(style, id, run);
                    previous = cache.previous(style, id);
                    drop(cache);
                    if let Some((geometry, picking, from_origin)) = cached {
                        reused += 1;
                        chunks.push(MeshChunk::from_geometry(
                            chunk_id,
                            from_origin,
                            geometry,
                            picking,
                        ));
                        preparation += prepare_started.elapsed();
                        publish(source_index, &chunks);
                        continue;
                    }
                }
                preparation += prepare_started.elapsed();
                let emit_started = std::time::Instant::now();
                rebuilt += 1;
                for (voxel_index, voxel) in run.iter().enumerate() {
                    if cancelled() {
                        return None;
                    }
                    let display =
                        bank.map_or(voxel.state.as_ref(), |bank| bank.render_state(&voxel.state));
                    let passes = models::Passes::new(voxel, display, &mut render_palette);
                    for (fluid_layer, voxel) in passes.iter() {
                        if (fluid_layer && !view.block_layers.blocks)
                            || (!fluid_layer && !view.block_layers.visible(&voxel.state, bank))
                        {
                            continue;
                        }
                        let cache_state = if fluid_layer {
                            models::fluid_state(
                                voxel.fluid_surface().expect("水层保留源流体").kind(),
                            )
                            .as_ref()
                        } else {
                            shared.chests.state(&original[voxel_index])
                        };
                        // Only witnessed height transitions, without a global face cap.
                        let wall_parts = if voxel.span > 1 {
                            distant.walls(voxel)
                        } else {
                            Vec::new()
                        };
                        let pos = (voxel.x, voxel.y, voxel.z);
                        let start_face = out.len();
                        let owner = (voxel.span == 1).then(|| crate::picking::Owner {
                            position: [voxel.x, voxel.y, voxel.z],
                            state: Arc::clone(&original[voxel_index].state),
                        });
                        // Distant samples came from the CENTER of their source tile,
                        // not the lower corner. Resolve variant/texture using that same
                        // source coordinate to avoid mismatched random blockstate textures.
                        let sample = (
                            voxel.x.saturating_add(voxel.span as i32 / 2),
                            voxel.y,
                            voxel.z.saturating_add(voxel.span as i32 / 2),
                        );
                        let variant = if use_templates {
                            variants.at(bank, cache_state, sample)
                        } else {
                            bank.map_or(0, |b| b.model_variant_key(&voxel.state, sample))
                        };
                        if use_templates
                            && voxel.span == 1
                            && bank
                                .map_or(voxel.kind, |b| b.render_material(&voxel.state, voxel.kind))
                                != VoxelMaterial::Liquid
                        {
                            let key = (model_cache_key(cache_state, variant), voxel.kind as u8);
                            let template = templates
                                .entry(key)
                                .or_insert_with(|| {
                                    let template_key = (variant, voxel.kind as u8);
                                    let template = shared
                                        .templates
                                        .get(cache_state)
                                        .and_then(|variants| variants.get(&template_key))
                                        .cloned()
                                        .or_else(|| {
                                            let start = std::time::Instant::now();
                                            let template = templates::Template::compile(
                                                view, voxel, default_uv, cancelled,
                                            )
                                            .map(Arc::new);
                                            template_preparation += start.elapsed();
                                            template
                                        });
                                    if let Some(template) = &template {
                                        templates_used
                                            .entry(Arc::new(cache_state.clone()))
                                            .or_default()
                                            .insert(template_key, Arc::clone(template));
                                    }
                                    template
                                })
                                .as_ref()?;
                            let delta = [
                                (voxel.x as i64 - origin[0] as i64) as f32,
                                (voxel.y as i64 - origin[1] as i64) as f32,
                                (voxel.z as i64 - origin[2] as i64) as f32,
                            ];
                            biome_dependent |= template.biome_dependent();
                            template.append(
                                &mut out,
                                voxel.faces,
                                delta,
                                voxel,
                                &mut colors,
                                cancelled,
                            )?;
                            if out.len() > start_face {
                                pick_groups.push(crate::picking::Group {
                                    quads: start_face..out.len(),
                                    owner,
                                });
                            }
                            continue;
                        }
                        // 非模板路径包含流体与其他运行时外观，保留完整颜色邻域依赖。
                        biome_dependent |= bank.is_some();
                        // 静态三角形与 JSON 元素共同组成同一个模型；不得提前跳过另一部分。
                        if voxel.span == 1
                            && bank.map_or(voxel.kind, |bank| {
                                bank.render_material(&voxel.state, voxel.kind)
                            }) != VoxelMaterial::Liquid
                        {
                            if let Some(triangles) = static_cache
                                .entry(model_cache_key(cache_state, variant))
                                .or_insert_with(|| {
                                    bank.and_then(|b| b.static_obj_at(&voxel.state, pos))
                                })
                                .as_ref()
                            {
                                for tri in triangles {
                                    if tri.cull.is_some_and(|face| voxel.faces & face.flag() == 0) {
                                        continue;
                                    }
                                    let xyz = tri.xyz.map(|point| {
                                        [
                                            (voxel.x as i64 - origin[0] as i64) as f32 + point[0],
                                            (voxel.y as i64 - origin[1] as i64) as f32 + point[1],
                                            (voxel.z as i64 - origin[2] as i64) as f32 + point[2],
                                        ]
                                    });
                                    let tint = if let Some(tint_index) = tri.tint_index {
                                        colors.as_mut().map_or([1.; 3], |c| {
                                            c.block(
                                                &voxel.state,
                                                tint_index,
                                                voxel.biome.as_deref(),
                                                pos,
                                            )
                                        })
                                    } else {
                                        [1.0; 3]
                                    };
                                    let plain = rgba(voxel, Face::Top);
                                    let rgba = tri.rgba.map(|mut color| {
                                        for i in 0..3 {
                                            color[i] *= tint[i];
                                        }
                                        if !view.show_textures {
                                            color[0] *= plain.r;
                                            color[1] *= plain.g;
                                            color[2] *= plain.b;
                                            color[3] *= plain.a;
                                        }
                                        color
                                    });
                                    let uv = if view.show_textures {
                                        tri.uv
                                    } else {
                                        [default_uv[0]; 3]
                                    };
                                    out.push(WorldQuad {
                                        xyz: [xyz[0], xyz[1], xyz[2], xyz[2]],
                                        uv: [uv[0], uv[1], uv[2], uv[2]],
                                        rgba: [rgba[0], rgba[1], rgba[2], rgba[2]],
                                        mode: tri.mode,
                                        atlas_page: if view.show_textures {
                                            tri.page
                                        } else {
                                            atlas.map_or(0, |a| a.white.page)
                                        },
                                        light: crate::lighting::FULLBRIGHT,
                                    });
                                }
                            }
                        }
                        // Borrow the original palette strings. Cloning the whole blockstate for
                        // every face caused avoidable allocations during stream updates.
                        // Cache keys always borrow immutable source palette entries. Seam
                        // corrections may be temporary chunk-local voxel copies; retaining
                        // references to those copies would outlive their storage.
                        let key = model_cache_key(cache_state, variant);
                        let display_state =
                            bank.map_or(voxel.state.as_ref(), |b| b.render_state(&voxel.state));
                        let display_kind = bank
                            .map_or(voxel.kind, |b| b.render_material(&voxel.state, voxel.kind));
                        let far = [models::fluid_part(display_state, false)
                            .unwrap_or_else(|| models::Part::cube(models::Box3::FULL))];
                        let fluid = if voxel.span == 1 {
                            match models::fluid_part_at(display_state, voxel) {
                                Ok(part) => part,
                                Err(error) => {
                                    log::error!("[fluid] {error}");
                                    continue;
                                }
                            }
                        } else {
                            None
                        };
                        let elements: &[models::Part] = if voxel.span > 1 {
                            &far
                        } else if let Some(ref fluid) = fluid {
                            std::slice::from_ref(fluid)
                        } else {
                            cache
                                .entry(key)
                                .or_insert_with(|| {
                                    bank.and_then(|b| b.geometry_at(&voxel.state, pos))
                                        .unwrap_or_else(|| {
                                            models::boxes(display_state)
                                                .into_iter()
                                                .map(models::Part::cube)
                                                .collect()
                                        })
                                })
                                .as_slice()
                        };
                        for (index, element) in elements.iter().enumerate() {
                            let face_parts = FACES
                                .into_iter()
                                .filter(|f| voxel.span == 1 || *f == Face::Top)
                                .map(|f| (f, element, false))
                                .chain(wall_parts.iter().map(|(f, part)| (*f, part, true)));
                            for (face, element, _is_wall) in face_parts {
                                if display_kind == VoxelMaterial::Liquid
                                    && voxel.span == 1
                                    && voxel.faces & face.flag() == 0
                                {
                                    continue;
                                }
                                // Sampled LOD geometry has no exact original JSON faces;
                                // never let one source model's cullface erase a verified
                                // height-transition wall or a horizon sample.
                                let face_key = (key, face as u8, index);
                                if voxel.span == 1 {
                                    let details =
                                        *visibility.entry(face_key).or_insert_with(|| {
                                            (
                                                bank.and_then(|b| {
                                                    b.face_defined_at(
                                                        &voxel.state,
                                                        face,
                                                        index,
                                                        pos,
                                                    )
                                                }),
                                                bank.and_then(|b| {
                                                    b.face_cull_direction_at(
                                                        &voxel.state,
                                                        face,
                                                        index,
                                                        pos,
                                                    )
                                                }),
                                            )
                                        });
                                    if !should_emit_resolved(voxel, element, face, details) {
                                        continue;
                                    }
                                }
                                // Explicit JSON faces without cullface are still subject to
                                // intra-model occlusion only when both boxes are proven solid.
                                if *internal
                                    .entry(face_key)
                                    .or_insert_with(|| internally_occluded(elements, index, face))
                                {
                                    continue;
                                }
                                let local_corners = if let Some(surface) =
                                    voxel.fluid_surface().filter(|_| {
                                        voxel.span == 1 && display_kind == VoxelMaterial::Liquid
                                    }) {
                                    models::fluid_face_vertices(surface, face)
                                } else {
                                    element.face_vertices(face.corners())
                                };
                                let corners = local_corners.map(|p| {
                                    let span = voxel.span as f32;
                                    [
                                        (voxel.x as i64 - origin[0] as i64) as f32
                                            + (span - 1.0) * 0.5
                                            + p[0] * span,
                                        (voxel.y as i64 - origin[1] as i64) as f32 + p[1],
                                        (voxel.z as i64 - origin[2] as i64) as f32
                                            + (span - 1.0) * 0.5
                                            + p[2] * span,
                                    ]
                                });
                                // The far-level tile is a visual height sample, but it
                                // still carries the actual original block material. Apply
                                // the corresponding PNG atlas UV rather than flat color.
                                let fluid_style = if view.show_textures
                                    && voxel.span == 1
                                    && display_kind == VoxelMaterial::Liquid
                                {
                                    fluid_materials.style_in(
                                        voxel,
                                        &voxel.state,
                                        face,
                                        colors.as_mut(),
                                    )
                                } else {
                                    None
                                };
                                let style = if view.show_textures {
                                    if voxel.span == 1 && display_kind == VoxelMaterial::Liquid {
                                        fluid_style.map(|style| style.face)
                                    } else {
                                        *styles.entry((key, face as u8, index)).or_insert_with(
                                            || {
                                                bank.and_then(|b| {
                                                    b.atlas_face_style_at(
                                                        &voxel.state,
                                                        face,
                                                        index,
                                                        sample,
                                                    )
                                                })
                                            },
                                        )
                                    }
                                } else {
                                    None
                                };
                                // PNG RGB / Alpha 保持原值；水走透明通道，
                                // 岩浆不因共享 Liquid 类别被强制淡化。
                                let (uv, shade, mut mode, page) = match style {
                                    Some((uv, alpha, page)) => {
                                        let tint = if let Some(style) = fluid_style {
                                            style.tint
                                        } else {
                                            colors.as_mut().map_or([1.; 3], |c| {
                                                c.face(
                                                    &voxel.state,
                                                    face,
                                                    index,
                                                    sample,
                                                    voxel.biome.as_deref(),
                                                )
                                            })
                                        };
                                        (
                                            uv,
                                            Color::from_rgb(tint[0], tint[1], tint[2]),
                                            alpha,
                                            page,
                                        )
                                    }
                                    None => {
                                        let base = rgba(voxel, face);
                                        (
                                            default_uv,
                                            Color::from_rgba(base.r, base.g, base.b, base.a),
                                            AlphaMode::Opaque,
                                            atlas.map_or(0, |a| a.white.page),
                                        )
                                    }
                                };
                                let mut shade = shade;
                                let (classified, alpha) = crate::textures::classify_block_alpha(
                                    display_state,
                                    display_kind,
                                    mode,
                                );
                                mode = classified;
                                shade.a *= alpha;
                                if shade.a < 0.999 {
                                    mode = AlphaMode::Blend;
                                }
                                let normal = (voxel.span == 1
                                    && (display_kind != VoxelMaterial::Liquid
                                        || face == Face::Bottom
                                        || voxel.fluid_surface().is_some_and(|fluid| {
                                            fluid.overlay_faces() & face.flag() != 0
                                        })))
                                .then(|| element.normal(face.normal()));
                                // Keep every repeated UV period within its own atlas slot;
                                // using sampler Repeat would sample adjacent Mod sprites.
                                if voxel.span == 1
                                    && view.show_textures
                                    && display_kind != VoxelMaterial::Liquid
                                {
                                    if let Some((raw_uv, region, sampler)) = repeating
                                        .entry(face_key)
                                        .or_insert_with(|| {
                                            bank.and_then(|b| {
                                                b.atlas_repeating_face_at(
                                                    &voxel.state,
                                                    face,
                                                    index,
                                                    sample,
                                                )
                                            })
                                        })
                                        .as_ref()
                                    {
                                        if !crate::repeating_uv::address(
                                            corners,
                                            *raw_uv,
                                            region,
                                            *sampler,
                                            bank.and_then(TextureBank::geometry_budget),
                                            |xyz, tex| {
                                                quad_vertices(
                                                    &mut out, xyz, tex, shade, mode, page, normal,
                                                );
                                            },
                                        ) {
                                            log::error!(
                                                "[models] {} 的源 UV 寻址超出安全范围",
                                                voxel.state.name
                                            );
                                            return None;
                                        }
                                    } else {
                                        quad_vertices(
                                            &mut out, corners, uv, shade, mode, page, normal,
                                        );
                                    }
                                } else {
                                    quad_vertices(&mut out, corners, uv, shade, mode, page, normal);
                                }
                            }
                        }
                        if out.len() > start_face {
                            pick_groups.push(crate::picking::Group {
                                quads: start_face..out.len(),
                                owner,
                            });
                        }
                    }
                }
                emission += emit_started.elapsed();
                if !out.is_empty() {
                    let index_started = std::time::Instant::now();
                    // Share one allocation between the source-exact cache, the
                    // completed frame and GPU residency. Previous code cloned
                    // every newly built chunk just to populate its cache.
                    // 完成的区块不再追加面，释放 Vec 增长留下的空余容量。
                    out.shrink_to_fit();
                    let previous = shared
                        .previous
                        .get(&chunk_id)
                        .map(|part| {
                            (
                                Arc::clone(part.geometry()),
                                Arc::clone(&part.picking),
                                part.origin,
                            )
                        })
                        .into_iter()
                        .chain(previous)
                        .find(|(geometry, _, previous_origin)| {
                            *previous_origin == origin
                                && geometry.biome_dependent() == biome_dependent
                                && bytemuck::cast_slice::<WorldQuad, u8>(geometry.quads())
                                    == bytemuck::cast_slice::<WorldQuad, u8>(&out)
                        });
                    let previous = previous.as_ref();
                    let geometry = if let Some((geometry, _, _)) = previous {
                        unchanged += 1;
                        Arc::clone(geometry)
                    } else {
                        Arc::new(crate::mesh_geometry::Geometry::with_biomes(
                            Arc::new(out),
                            biome_dependent,
                        ))
                    };
                    let picking = if let Some((_, picking, _)) =
                        previous.filter(|(_, picking, _)| picking.source_groups() == pick_groups)
                    {
                        Arc::clone(picking)
                    } else {
                        match crate::picking::Index::build(
                            origin,
                            Arc::clone(geometry.quads()),
                            pick_groups,
                        ) {
                            Ok(index) => Arc::new(index),
                            Err(error) => {
                                log::error!("[mesh] 区块 {chunk_id:?} 拾取索引失败：{error}");
                                return None;
                            }
                        }
                    };
                    indexing += index_started.elapsed();
                    let store_started = std::time::Instant::now();
                    if !cancelled() {
                        let mut cache = view.chunk_cache.lock().expect("世界网格构建状态锁被毒化");
                        if let Some(id) = id.filter(|_| {
                            shared.chests.is_empty()
                                || original.iter().zip(run).all(|(a, b)| a.state == b.state)
                        }) {
                            cache.insert(
                                style,
                                id,
                                run,
                                origin,
                                Arc::clone(&geometry),
                                Arc::clone(&picking),
                            );
                        }
                    }
                    chunks.push(MeshChunk::from_geometry(
                        chunk_id, origin, geometry, picking,
                    ));
                    storing += store_started.elapsed();
                    publish(source_index, &chunks);
                }
            }
            if let Some(context) = context {
                if cancelled() {
                    return None;
                }
                view.chunk_cache
                    .lock()
                    .expect("世界网格构建状态锁被毒化")
                    .insert_patch(
                        style,
                        context.clone(),
                        y_max,
                        chunks[first_chunk..].to_vec(),
                    );
            }

            Some(chunks)
        })()?;
        if sender.send((source_index, chunks)).is_err() {
            if !cancelled() {
                log::debug!("[mesh] 完整 patch 的接收端已关闭");
            }
            return None;
        }
    }
    Some(Stats {
        reused,
        rebuilt,
        preparation,
        emission,
        indexing,
        storing,
        template_preparation,
        templates: templates.len(),
        model_hidden: model_occlusion.removed,
        templates_used,
        unchanged,
    })
}
