//! 自由相机增量显示已上传的新 Section，并沿用上一完整帧的其余通道。
use super::{
    atlas, chunk_key, opaque_y32, regional, same_arc, section_order, ChunkKey, GpuStreamChunk,
    Pending, Pipeline, Primitive, WorldView,
};
use crate::spectator::StreamMesh;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(in crate::gpu_scene) struct Preview {
    stamp: Option<Stamp>,
    pub mesh: Option<Arc<StreamMesh>>,
    pub regions: regional::Residents,
    pub draws: regional::DrawList,
    pub animated: atlas::AnimatedMaterials,
    pub replaced: BTreeSet<section_order::Key>,
    pub ordered: BTreeMap<ChunkKey, GpuStreamChunk>,
    pub transparent: BTreeMap<section_order::Key, Arc<section_order::Resident>>,
}

struct Stamp {
    base: Arc<StreamMesh>,
    wanted: Arc<StreamMesh>,
    revision: usize,
    checked: Instant,
}

impl Pending {
    pub(in crate::gpu_scene) fn update_preview(&mut self, pipeline: &Pipeline, wanted: &Primitive) {
        let Some((base, desired)) = self.preview_sources(pipeline, wanted) else {
            self.preview = Preview::default();
            return;
        };
        let same_source = self.preview.stamp.as_ref().is_some_and(|stamp| {
            Arc::ptr_eq(&stamp.base, &base) && Arc::ptr_eq(&stamp.wanted, &desired)
        });
        if self.preview.stamp.as_ref().is_some_and(|stamp| {
            same_source
                && (stamp.revision == self.region_revision
                    || (self.preview.mesh.is_some()
                        && stamp.checked.elapsed() < Duration::from_millis(16)))
        }) {
            return;
        }
        self.preview.stamp = Some(Stamp {
            base: Arc::clone(&base),
            wanted: Arc::clone(&desired),
            revision: self.region_revision,
            checked: Instant::now(),
        });
        let local_order = self.target.section_mode()
            && pipeline
                .active
                .as_ref()
                .is_some_and(Primitive::section_mode);
        let mut old = BTreeMap::<_, Vec<_>>::new();
        for part in base.chunks.iter().filter(|p| p.key.2 == 1) {
            old.entry(section_order::key(chunk_key(part)))
                .or_default()
                .push(part);
        }
        let sampled: HashSet<_> = base
            .chunks
            .iter()
            .filter(|p| p.key.2 > 1)
            .map(|p| (p.key.0, p.key.1))
            .collect();
        let mut wanted = BTreeMap::<_, Vec<_>>::new();
        for part in desired.chunks.iter().filter(|p| p.key.2 == 1) {
            wanted
                .entry(section_order::key(chunk_key(part)))
                .or_default()
                .push(part);
        }
        let groups = self.page_groups.as_deref().expect("预览使用已绑定的图集");
        let hints = self.hints.as_deref().expect("预览使用已绑定的材质提示");
        let (active_groups, active_hints) = pipeline
            .stream_materials
            .as_ref()
            .expect("预览使用实际已显示的材质布局");
        let mut regions = BTreeMap::new();
        let mut ordered = BTreeMap::new();
        let mut transparent = BTreeMap::new();
        let mut replaced = BTreeSet::new();
        let mut parts = Vec::new();
        let mut animated = pipeline.world_animated.clone();
        for (id, desired_parts) in &wanted {
            // 新增实体页不阻塞基础地形；只发布能用当前 GPU 材质正确绘制的 Section。
            if !active_hints.compatible(
                hints,
                active_groups,
                groups,
                desired_parts
                    .iter()
                    .flat_map(|part| part.geometry().used_pages().iter().copied()),
            ) {
                continue;
            }
            let prior = old.get(id);
            let changes = prior.is_some_and(|previous| {
                previous.len() != desired_parts.len()
                    || previous.iter().zip(desired_parts).any(|(a, b)| {
                        chunk_key(a) != chunk_key(b) || !Arc::ptr_eq(&a.picking, &b.picking)
                    })
            });
            if prior.is_some() && (!local_order || !changes || !desired.complete) {
                continue;
            }
            let [x, y, z] = id.0;
            if sampled.contains(&(x.div_euclid(16), z.div_euclid(16))) {
                continue;
            }
            let region_key = (x.div_euclid(16), y, z.div_euclid(16), 16, true);
            let regular = desired_parts.iter().any(|p| p.geometry().has_regular());
            let region = self
                .regions
                .get(&region_key)
                .or_else(|| self.staged_regions.get(&region_key))
                .filter(|region| {
                    region.offsets.origin == base.origin
                        && region
                            .source
                            .keys()
                            .eq(desired_parts.iter().map(|p| chunk_key(p)))
                        && region.reusable(&region.source, opaque_y32(pipeline), groups, hints)
                });
            if regular && region.is_none() {
                continue;
            }
            let blended: Vec<_> = desired_parts
                .iter()
                .filter(|p| !p.geometry().blended().is_empty())
                .map(|p| chunk_key(p))
                .collect();
            let ordered_keys: Vec<_> = desired_parts
                .iter()
                .filter(|p| !p.geometry().ordered().is_empty())
                .map(|p| chunk_key(p))
                .collect();
            let blend = self
                .section_orders
                .ready
                .get(id)
                .or_else(|| self.staged_section_orders.get(id))
                .filter(|resident| resident.matches(&blended, base.origin));
            let materials = self
                .section_orders
                .materials
                .as_ref()
                .and_then(|materials| materials.get(id));
            let other_passes = !blended.is_empty() || !ordered_keys.is_empty();
            let complete = !other_passes
                || (local_order
                    && (blended.is_empty() || blend.is_some())
                    && ordered_keys.iter().all(|key| self.chunks.contains_key(key))
                    && materials.is_some());
            if (!complete && (prior.is_some() || !regular)) || (!regular && !local_order) {
                continue;
            }
            if let Some(region) = region {
                let cached = self
                    .preview
                    .regions
                    .get(&region_key)
                    .filter(|old| Arc::ptr_eq(&old.buffers, &region.buffers))
                    .unwrap_or(region);
                animated.extend(region.animated.iter().copied());
                regions.insert(region_key, cached.clone());
            }
            if complete {
                if prior.is_some() {
                    replaced.insert(*id);
                }
                if let Some(blend) = blend {
                    transparent.insert(*id, Arc::clone(blend));
                }
                for key in ordered_keys {
                    let resident = self.preview.ordered.get(&key).unwrap_or(&self.chunks[&key]);
                    ordered.insert(key, resident.clone());
                }
                if let Some(materials) = materials {
                    animated.extend(materials.iter().copied());
                }
            }
            parts.extend(
                desired_parts
                    .iter()
                    .filter(|p| complete || p.geometry().has_regular())
                    .map(|p| {
                        let mut part = (*p).clone();
                        part.opaque_only = !complete;
                        part
                    }),
            );
        }
        if local_order
            && desired.complete
            && self
                .target
                .stream_mesh
                .as_ref()
                .is_some_and(|target| Arc::ptr_eq(target, &desired))
        {
            replaced.extend(old.keys().filter(|id| !wanted.contains_key(id)).copied());
        }
        let unchanged = same_source
            && replaced == self.preview.replaced
            && transparent.len() == self.preview.transparent.len()
            && transparent.iter().all(|(id, next)| {
                self.preview
                    .transparent
                    .get(id)
                    .is_some_and(|old| Arc::ptr_eq(old, next))
            })
            && ordered.keys().eq(self.preview.ordered.keys())
            && regions.len() == self.preview.regions.values().count()
            && regions.iter().all(|(key, region)| {
                self.preview
                    .regions
                    .get(key)
                    .is_some_and(|old| Arc::ptr_eq(&old.buffers, &region.buffers))
            });
        if unchanged {
            return;
        }
        self.preview.mesh = if parts.is_empty() && replaced.is_empty() {
            None
        } else {
            let base_chunks = if replaced.is_empty() {
                base.chunks.clone()
            } else {
                base.chunks
                    .iter()
                    .filter(|part| !replaced.contains(&section_order::key(chunk_key(part))))
                    .cloned()
                    .collect::<Vec<_>>()
                    .into()
            };
            log::debug!(
                "[gpu-upload] Section 渐进交接，普通区域 {}、替换 {}、透明 {}",
                regions.len(),
                replaced.len(),
                transparent.len()
            );
            Some(Arc::new(StreamMesh {
                chunks: base_chunks.join(&parts.into()),
                visibility: if replaced.is_empty() && transparent.is_empty() {
                    base.visibility.clone()
                } else {
                    None
                },
                complete: false,
                ..base.as_ref().clone()
            }))
        };
        self.preview.animated = animated;
        self.preview.regions = regions.into();
        self.preview.replaced = replaced;
        self.preview.ordered = ordered;
        self.preview.transparent = transparent;
    }

    fn preview_sources(
        &self,
        pipeline: &Pipeline,
        wanted: &Primitive,
    ) -> Option<(Arc<StreamMesh>, Arc<StreamMesh>)> {
        let active = pipeline.active.as_ref()?;
        // 资源、投影和场景原点一致时，才可以把新 Section 画到旧完整帧上。
        if !(wanted.section_mode()
            || matches!(wanted.presentation.as_ref().map(|p| &p.view),
            Some(WorldView::Free(camera)) if camera.perspective))
            || active.quality != wanted.quality
            || self.target.quality != wanted.quality
            || !same_arc(active.atlas.as_ref(), pipeline.atlas.as_ref())
            || (!super::material_hints_enabled(pipeline)
                && !same_arc(active.atlas.as_ref(), wanted.atlas.as_ref()))
            || pipeline.stream_materials.is_none()
            || self.hints.is_none()
        {
            return None;
        }
        let base = active.stream_mesh.as_ref()?;
        let desired = wanted.stream_mesh.as_ref()?;
        let target = self.target.stream_mesh.as_ref()?;
        if !same_arc(self.target.atlas.as_ref(), wanted.atlas.as_ref()) {
            // 实体运行时页变化不能挡住同一基底上的新地形；只认可实际不可变前缀。
            let terrain_atlas =
                |mesh: &Arc<StreamMesh>| mesh.terrain.as_ref().unwrap_or(mesh).atlas.clone();
            let source = terrain_atlas(&base)?;
            let a = terrain_atlas(target)?;
            let b = terrain_atlas(desired)?;
            if !super::material_hints_enabled(pipeline)
                || !Arc::ptr_eq(&source, &a)
                || !Arc::ptr_eq(&a, &b)
                || !crate::textures::atlas_compatible(self.target.atlas.as_ref()?, &a)
                || !crate::textures::atlas_compatible(wanted.atlas.as_ref()?, &a)
            {
                return None;
            }
        }
        (base.origin == desired.origin && target.origin == base.origin)
            .then(|| (Arc::clone(base), Arc::clone(desired)))
    }
}
