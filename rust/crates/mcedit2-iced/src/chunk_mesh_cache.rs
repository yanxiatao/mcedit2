//! Original-source exact-chunk spectator mesh cache, keyed by source identity.
//! 流式区块按不可变来源及邻区块身份复用；限定窗口按真实方块比较。
use crate::{mesh_geometry::Geometry, spectator_stream::ChunkContext, textures::TextureBank};
use mcedit2_world::surface::SurfaceVoxel;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

pub(crate) type Key = (i32, i32, i32, i32, i32);
/// 保留资源控制块的弱引用，防止地址复用误命中，又不额外保活整包解码图片。
/// 采样质量只改变绘制过滤，不改变世界几何。
#[derive(Clone)]
pub(crate) struct Style {
    bank: Option<Weak<TextureBank>>,
    enabled: bool,
    block_layers: crate::block_visibility::Layers,
}
impl PartialEq for Style {
    fn eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.block_layers == other.block_layers
            && match (&self.bank, &other.bank) {
                (Some(a), Some(b)) => Weak::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}
impl Eq for Style {}
type CachedMesh = (Arc<Geometry>, Arc<crate::picking::Index>, [i32; 3]);
struct Entry {
    biomes_valid: bool,
    source: Vec<SurfaceVoxel>,
    origin: [i32; 3],
    geometry: Arc<Geometry>,
    picking: Arc<crate::picking::Index>,
}
/// 保存整份源 patch 生成的所有网格片；不重复复制其方块数组。
struct PatchEntry {
    context: ChunkContext,
    source_local: bool,
    /// 高度采样的地面衔接使用该范围，精确区块的完整 Section 不依赖它。
    sampled_y_max: Option<i32>,
    chunks: Vec<crate::spectator::MeshChunk>,
}
#[derive(Default)]
pub(crate) struct ChunkMeshCache {
    biomes: Option<mcedit2_world::biome_source::Region>,
    entries: HashMap<Key, Entry>,
    patches: HashMap<(i32, i32), PatchEntry>,
    style: Option<Style>,
    templates: Arc<crate::spectator::TemplateCache>,
}
impl ChunkMeshCache {
    /// 与几何相同的资源 / 显示样式身份，后台借用不可变缓存，不在逐格路径上加锁。
    pub(crate) fn templates(&mut self, style: &Style) -> Arc<crate::spectator::TemplateCache> {
        self.ensure_style(style);
        Arc::clone(&self.templates)
    }
    pub(crate) fn store_templates(&mut self, mut used: crate::spectator::TemplateCache) {
        if used.is_empty() {
            return;
        }
        // 当前工作集优先，剩余面载荷预算保留上一轮热模板，避免相邻批次交替重编译。
        const RETAIN_BYTES: usize = 32 << 20;
        let mut bytes: usize = used
            .values()
            .flat_map(|v| v.values())
            .map(|t| t.bytes())
            .sum();
        for (state, variants) in self.templates.iter() {
            for (&key, template) in variants {
                if bytes.saturating_add(template.bytes()) > RETAIN_BYTES {
                    continue;
                }
                let target = used.entry(Arc::clone(state)).or_default();
                if let std::collections::hash_map::Entry::Vacant(entry) = target.entry(key) {
                    bytes += template.bytes();
                    entry.insert(Arc::clone(template));
                }
            }
        }
        self.templates = Arc::new(used);
    }
    pub(crate) fn prepare_biomes(&mut self, source: &mcedit2_world::biome_source::Region) {
        let mut current = source.clone();
        if let Some(previous) = &self.biomes {
            current.reuse_columns(previous);
            for (key, entry) in &mut self.entries {
                // 失效只禁止跳过重建；保留旧几何供重建后的完整内容核对。
                entry.biomes_valid &= !entry.geometry.biome_dependent()
                    || previous.same_neighborhood(&current, key.0, key.1);
            }
            self.patches.retain(|&(x, z), entry| {
                !entry
                    .chunks
                    .iter()
                    .any(|part| part.geometry().biome_dependent())
                    || previous.same_neighborhood(&current, x, z)
            });
        } else {
            self.entries.clear();
            self.patches.clear();
        }
        self.biomes = Some(current);
    }
    #[cfg(test)]
    pub(crate) fn source_allocations(&self) -> impl Iterator<Item = (&[SurfaceVoxel], usize)> {
        self.entries.values().map(|entry| {
            (
                entry.source.as_slice(),
                entry.source.capacity() * std::mem::size_of::<SurfaceVoxel>(),
            )
        })
    }
    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len() + self.patches.len()
    }

    pub(crate) fn style(
        bank: Option<&Arc<TextureBank>>,
        enabled: bool,
        block_layers: crate::block_visibility::Layers,
    ) -> Style {
        Style {
            bank: bank.map(Arc::downgrade),
            enabled,
            block_layers,
        }
    }
    fn ensure_style(&mut self, style: &Style) {
        if self.style.as_ref() != Some(style) {
            self.entries.clear();
            self.patches.clear();
            self.style = Some(style.clone());
            self.templates = Arc::default();
        }
    }
    pub(crate) fn get(
        &mut self,
        style: &Style,
        key: Key,
        source: &[SurfaceVoxel],
    ) -> Option<CachedMesh> {
        self.ensure_style(style);
        let entry = self.entries.get(&key)?;
        if !entry.biomes_valid
            || entry.source.len() != source.len()
            || !entry.source.iter().zip(source).all(|(a, b)| a == b)
        {
            return None;
        }
        Some((
            Arc::clone(&entry.geometry),
            Arc::clone(&entry.picking),
            entry.origin,
        ))
    }
    pub(crate) fn insert(
        &mut self,
        style: &Style,
        key: Key,
        source: &[SurfaceVoxel],
        origin: [i32; 3],
        geometry: Arc<Geometry>,
        picking: Arc<crate::picking::Index>,
    ) {
        self.ensure_style(style);
        // 采样远景使用独立的完整源依赖，不能混用精确区块缓存。
        if source.iter().any(|v| v.span != 1) {
            return;
        }
        self.entries.insert(
            key,
            Entry {
                biomes_valid: true,
                source: source.to_vec(),
                origin,
                geometry,
                picking,
            },
        );
    }
    /// 重建前的同样式几何，只允许在完整面字节核对后复用，不能替代来源有效性检查。
    pub(crate) fn previous(&mut self, style: &Style, key: Key) -> Option<CachedMesh> {
        self.ensure_style(style);
        self.entries.get(&key).map(|entry| {
            (
                Arc::clone(&entry.geometry),
                Arc::clone(&entry.picking),
                entry.origin,
            )
        })
    }
    pub(crate) fn get_patch(
        &mut self,
        style: &Style,
        context: &ChunkContext,
        y_max: i32,
    ) -> Option<Vec<crate::spectator::MeshChunk>> {
        self.ensure_style(style);
        let entry = self.patches.get(&context.coordinates())?;
        ((if entry.source_local {
            entry.context.same_local_source(context)
        } else {
            entry.context.same_sources(context)
        }) && entry.sampled_y_max == context.sampled_y_max(y_max))
        .then(|| entry.chunks.clone())
    }
    pub(crate) fn insert_patch(
        &mut self,
        style: &Style,
        context: ChunkContext,
        y_max: i32,
        chunks: Vec<crate::spectator::MeshChunk>,
    ) {
        self.ensure_style(style);
        let sampled_y_max = context.sampled_y_max(y_max);
        // 完整读取已冻结九宫格剔面、流体和群系事实；旧箱子仍由显示邻格配对，保留其依赖。
        let legacy_chests = style
            .bank
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|bank| {
                bank.has_legacy_chests()
                    && context
                        .source
                        .voxels
                        .iter()
                        .any(crate::legacy_chest_context::candidate)
            });
        let source_local = sampled_y_max.is_none()
            && context.source.sections.is_some()
            && context.source.tile.source_present
            && context.source.tile.unknown_columns == 0
            && !legacy_chests;
        self.patches.insert(
            context.coordinates(),
            PatchEntry {
                context,
                source_local,
                sampled_y_max,
                chunks,
            },
        );
    }
    /// Like Amulet's chunk render manager, retain chunks in the selected
    /// visible source set instead of throwing away valid meshes after an
    /// arbitrary fixed count. Replanning camera visibility retires old meshes.
    pub(crate) fn retain_visible(&mut self, visible: &HashSet<(i32, i32)>) {
        self.entries
            .retain(|(cx, cz, _, _, _), _| visible.contains(&(*cx, *cz)));
        self.patches
            .retain(|coordinate, _| visible.contains(coordinate));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcedit2_world::{
        sections::State,
        surface::{VoxelMaterial, FACE_TOP},
    };

    fn sample(x: i32) -> SurfaceVoxel {
        SurfaceVoxel::new(
            [x, 90, 0],
            mcedit2_world::surface::VoxelData {
                state: State {
                    name: "minecraft:stone".into(),
                    properties: vec![],
                }
                .into(),
                biome: Some("minecraft:plains".into()),
                faces: FACE_TOP,
                original_block_entity: false,
                kind: VoxelMaterial::Solid,
                staged: false,
                span: 1,
                fluid: None,
                light: Default::default(),
            },
        )
    }
    fn vertex() -> crate::world_mesh::WorldQuad {
        crate::world_mesh::WorldQuad {
            xyz: [[1.5, 2.5, 3.5]; 4],
            rgba: [[1.0; 4]; 4],
            uv: [[0.0; 2]; 4],
            mode: 0,
            atlas_page: 0,
            light: crate::lighting::FULLBRIGHT,
        }
    }

    #[test]
    fn exact_mesh_cache_keeps_original_arc_without_copying_vertex_arrays() {
        let mut cache = ChunkMeshCache::default();
        let voxel = sample(0);
        let style = ChunkMeshCache::style(None, true, Default::default());
        let original = Arc::new(vec![vertex()]);
        let geometry = Arc::new(Geometry::new(Arc::clone(&original)));
        cache.insert(
            &style,
            (0, 0, 0, 90, 0),
            std::slice::from_ref(&voxel),
            [0, 0, 0],
            Arc::clone(&geometry),
            Arc::new(crate::picking::Index::default()),
        );
        let (reused, _, origin) = cache
            .get(&style, (0, 0, 0, 90, 0), std::slice::from_ref(&voxel))
            .unwrap();
        assert!(Arc::ptr_eq(reused.quads(), &original));
        assert!(Arc::ptr_eq(&reused, &geometry));
        assert_eq!(origin, [0, 0, 0]);
    }

    #[test]
    fn exact_equality_invalidates_modified_source() {
        let mut c = ChunkMeshCache::default();
        let v = sample(-1);
        let s = ChunkMeshCache::style(None, false, Default::default());
        c.insert(
            &s,
            (-1, 0, -1, 90, 0),
            std::slice::from_ref(&v),
            [-16, 80, 0],
            Arc::new(Geometry::new(Arc::new(vec![vertex()]))),
            Arc::new(crate::picking::Index::default()),
        );
        assert!(c
            .get(&s, (-1, 0, -1, 90, 0), std::slice::from_ref(&v))
            .is_some());
        let mut changed = v.clone();
        changed.faces = 0;
        assert!(c
            .get(&s, (-1, 0, -1, 90, 0), std::slice::from_ref(&changed))
            .is_none());
        changed.faces = FACE_TOP;
        changed.biome = None;
        assert!(c
            .get(&s, (-1, 0, -1, 90, 0), std::slice::from_ref(&changed))
            .is_none());
        changed.biome = v.biome.clone();
        Arc::make_mut(&mut changed.state).name = "minecraft:dirt".into();
        assert!(c
            .get(&s, (-1, 0, -1, 90, 0), std::slice::from_ref(&changed))
            .is_none());
        assert_eq!(v.state.name, "minecraft:stone");
        assert!(c
            .get(&s, (-1, 0, -1, 90, 0), std::slice::from_ref(&v))
            .is_some());
        assert!(c.get(&s, (-1, 0, -1, 90, 0), &[]).is_none());
    }

    #[test]
    fn texture_visibility_changes_clear_cache_and_fluids_accept_exact_dependencies() {
        let mut c = ChunkMeshCache::default();
        let v = sample(0);
        let a = ChunkMeshCache::style(None, false, Default::default());
        c.insert(
            &a,
            (0, 0, 0, 90, 0),
            std::slice::from_ref(&v),
            [0, 0, 0],
            Arc::new(Geometry::new(Arc::new(vec![vertex()]))),
            Arc::new(crate::picking::Index::default()),
        );
        assert!(c
            .get(
                &ChunkMeshCache::style(None, true, Default::default()),
                (0, 0, 0, 90, 0),
                std::slice::from_ref(&v)
            )
            .is_none());
        let s = ChunkMeshCache::style(None, true, Default::default());
        let mut water = v.clone();
        water.kind = VoxelMaterial::Liquid;
        c.insert(
            &s,
            (0, 0, 0, 90, 0),
            std::slice::from_ref(&water),
            [0, 0, 0],
            Arc::new(Geometry::new(Arc::new(vec![vertex()]))),
            Arc::new(crate::picking::Index::default()),
        );
        assert!(c
            .get(&s, (0, 0, 0, 90, 0), std::slice::from_ref(&water))
            .is_some());
    }

    #[test]
    fn no_fixed_chunk_count_discards_active_meshes() {
        let mut cache = ChunkMeshCache::default();
        let style = ChunkMeshCache::style(None, false, Default::default());
        let active: HashSet<_> = (0..2200).map(|cx| (cx, 0)).collect();
        for cx in 0..2200 {
            let sample = sample(cx * 16);
            cache.insert(
                &style,
                (cx, 0, sample.x, sample.y, sample.z),
                std::slice::from_ref(&sample),
                [0, 0, 0],
                Arc::new(Geometry::new(Arc::new(vec![vertex()]))),
                Arc::new(crate::picking::Index::default()),
            );
        }
        cache.retain_visible(&active);
        assert_eq!(cache.entry_count(), 2200);
        assert!(cache
            .get(&style, (0, 0, 0, 90, 0), std::slice::from_ref(&sample(0)))
            .is_some());
    }

    #[test]
    fn cache_retains_all_visible_chunks_and_discards_departed_chunks() {
        let mut c = ChunkMeshCache::default();
        let style = ChunkMeshCache::style(None, false, Default::default());
        for cx in -4..=4 {
            let voxel = sample(cx * 16);
            c.insert(
                &style,
                (cx, 0, voxel.x, voxel.y, voxel.z),
                std::slice::from_ref(&voxel),
                [0, 0, 0],
                Arc::new(Geometry::new(Arc::new(vec![vertex()]))),
                Arc::new(crate::picking::Index::default()),
            );
        }
        let active: HashSet<_> = (-4..=4).filter(|cx| *cx >= 0).map(|cx| (cx, 0)).collect();
        c.retain_visible(&active);
        assert_eq!(c.entry_count(), 5);
        let voxel = sample(0);
        assert!(c
            .get(&style, (0, 0, 0, 90, 0), std::slice::from_ref(&voxel))
            .is_some());
        let old = sample(-16);
        assert!(c
            .get(&style, (-1, 0, -16, 90, 0), std::slice::from_ref(&old))
            .is_none());
    }

    #[test]
    fn biome_changes_only_retire_geometry_that_used_biome_colors() {
        let mut cache = ChunkMeshCache::default();
        let style = ChunkMeshCache::style(None, true, Default::default());
        cache.prepare_biomes(&mcedit2_world::biome_source::Region::new(Some(1)));
        let voxel = sample(0);
        let plain = Arc::new(Geometry::with_biomes(Arc::new(vec![vertex()]), false));
        for (key, geometry) in [
            ((0, 0, 0, 0, 0), Arc::clone(&plain)),
            (
                (0, 0, 16, 0, 0),
                Arc::new(Geometry::with_biomes(Arc::new(vec![vertex()]), true)),
            ),
        ] {
            cache.insert(
                &style,
                key,
                std::slice::from_ref(&voxel),
                [0; 3],
                geometry,
                Arc::new(crate::picking::Index::default()),
            );
        }
        cache.prepare_biomes(&mcedit2_world::biome_source::Region::new(Some(2)));
        let (same, _, _) = cache
            .get(&style, (0, 0, 0, 0, 0), std::slice::from_ref(&voxel))
            .unwrap();
        assert!(Arc::ptr_eq(&plain, &same));
        assert!(
            cache
                .get(&style, (0, 0, 16, 0, 0), std::slice::from_ref(&voxel))
                .is_none(),
            "真实颜色依赖变化必须重建"
        );
    }
}
