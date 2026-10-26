//! Original-source exact-chunk spectator mesh cache, keyed by source identity.
//! Equality of original source voxels invalidates changed seam flags, biomes,
//! weighted model states and edited blocks without hash-collision risks.
use crate::{
    gpu_mesh::{Quality, Vertex},
    textures::TextureBank,
};
use mcedit2_world::surface::{SurfaceVoxel, VoxelMaterial};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub(crate) type Key = (i32, i32, i32, i32, i32);
type Style = (usize, bool, Quality);
struct Entry {
    source: Vec<SurfaceVoxel>,
    origin: [i32; 3],
    vertices: Arc<Vec<Vertex>>,
}
#[derive(Default)]
pub(crate) struct ChunkMeshCache {
    entries: HashMap<Key, Entry>,
    style: Option<Style>,
}
impl ChunkMeshCache {
    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn style(bank: Option<&Arc<TextureBank>>, enabled: bool, quality: Quality) -> Style {
        (
            bank.map_or(0, |b| Arc::as_ptr(b) as usize),
            enabled,
            quality,
        )
    }
    fn ensure_style(&mut self, style: Style) {
        if self.style != Some(style) {
            self.entries.clear();
            self.style = Some(style);
        }
    }
    pub(crate) fn get(
        &mut self,
        style: Style,
        key: Key,
        source: &[&SurfaceVoxel],
    ) -> Option<(Arc<Vec<Vertex>>, [i32; 3])> {
        self.ensure_style(style);
        let entry = self.entries.get(&key)?;
        if entry.source.len() != source.len()
            || !entry.source.iter().zip(source).all(|(a, b)| a == *b)
        {
            return None;
        }
        Some((Arc::clone(&entry.vertices), entry.origin))
    }
    pub(crate) fn insert(
        &mut self,
        style: Style,
        key: Key,
        source: &[&SurfaceVoxel],
        origin: [i32; 3],
        vertices: Vec<Vertex>,
    ) {
        self.ensure_style(style);
        // Liquid faces depend on neighbors outside the sampled chunk.
        if source
            .iter()
            .any(|v| v.span != 1 || v.kind == VoxelMaterial::Liquid)
        {
            return;
        }
        self.entries.insert(
            key,
            Entry {
                source: source.iter().map(|v| (*v).clone()).collect(),
                origin,
                vertices: Arc::new(vertices),
            },
        );
    }
    /// Like Amulet's chunk render manager, retain chunks in the selected
    /// visible source set instead of throwing away valid meshes after an
    /// arbitrary fixed count. Replanning camera visibility retires old meshes.
    pub(crate) fn retain_visible(&mut self, visible: &HashSet<(i32, i32)>) {
        self.entries.retain(|(cx, cz, _, _, _), _| visible.contains(&(*cx, *cz)));
    }
    /// Chunk triangles are stored relative to the *old* world-view origin.
    /// Offset all copies to keep their absolute world positions unchanged.
    pub(crate) fn append_rebased(
        out: &mut Vec<Vertex>,
        input: &[Vertex],
        old: [i32; 3],
        new: [i32; 3],
    ) {
        let delta = [
            (old[0] as i64 - new[0] as i64) as f32,
            (old[1] as i64 - new[1] as i64) as f32,
            (old[2] as i64 - new[2] as i64) as f32,
        ];
        out.reserve(input.len());
        for value in input {
            let mut translated = *value;
            for (pos, d) in translated.xyz.iter_mut().zip(delta) {
                *pos += d;
            }
            out.push(translated);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mcedit2_world::{sections::State, surface::FACE_TOP};
    fn sample(x: i32) -> SurfaceVoxel {
        SurfaceVoxel {
            x,
            y: 90,
            z: 0,
            state: State {
                name: "minecraft:stone".into(),
                properties: vec![],
            },
            biome: Some("minecraft:plains".into()),
            faces: FACE_TOP,
            original_block_entity: false,
            kind: VoxelMaterial::Solid,
            staged: false,
            span: 1,
        }
    }
    fn vertex() -> Vertex {
        Vertex {
            xy: [0.0; 2],
            depth: 0.0,
            rgba: [1.0; 4],
            uv: [0.0; 2],
            mode: 0,
            xyz: [1.5, 2.5, 3.5],
            world_mode: 1,
            atlas_page: 0,
        }
    }
    #[test]
    fn exact_equality_invalidates_modified_source() {
        let mut c = ChunkMeshCache::default();
        let v = sample(-1);
        let s = ChunkMeshCache::style(None, false, Quality::Balanced);
        c.insert(s, (-1, 0, -1, 90, 0), &[&v], [-16, 80, 0], vec![vertex()]);
        assert!(c.get(s, (-1, 0, -1, 90, 0), &[&v]).is_some());
        let mut changed = v.clone();
        changed.faces = 0;
        assert!(c.get(s, (-1, 0, -1, 90, 0), &[&changed]).is_none());
        changed.faces = FACE_TOP;
        changed.biome = None;
        assert!(c.get(s, (-1, 0, -1, 90, 0), &[&changed]).is_none());
        changed.biome = v.biome.clone();
        changed.state.name = "minecraft:dirt".into();
        assert!(c.get(s, (-1, 0, -1, 90, 0), &[&changed]).is_none());
        assert!(c.get(s, (-1, 0, -1, 90, 0), &[]).is_none());
    }
    #[test]
    fn rebased_original_world_position_is_identical() {
        let mut result = Vec::new();
        ChunkMeshCache::append_rebased(&mut result, &[vertex()], [-16, 80, 0], [-32, 72, 16]);
        assert_eq!(result[0].xyz, [17.5, 10.5, -12.5]);
    }
    #[test]
    fn pack_and_quality_changes_clear_cache_and_liquids_are_excluded() {
        let mut c = ChunkMeshCache::default();
        let v = sample(0);
        let a = ChunkMeshCache::style(None, false, Quality::Balanced);
        c.insert(a, (0, 0, 0, 90, 0), &[&v], [0, 0, 0], vec![vertex()]);
        assert!(c
            .get(
                ChunkMeshCache::style(None, true, Quality::Balanced),
                (0, 0, 0, 90, 0),
                &[&v]
            )
            .is_none());
        let s = ChunkMeshCache::style(None, true, Quality::Balanced);
        let mut water = v.clone();
        water.kind = VoxelMaterial::Liquid;
        c.insert(s, (0, 0, 0, 90, 0), &[&water], [0, 0, 0], vec![vertex()]);
        assert!(c.get(s, (0, 0, 0, 90, 0), &[&water]).is_none());
    }
    #[test]
    fn no_fixed_chunk_count_discards_active_meshes() {
        let mut cache = ChunkMeshCache::default();
        let style = ChunkMeshCache::style(None, false, Quality::Balanced);
        let active: HashSet<_> = (0..2200).map(|cx| (cx, 0)).collect();
        for cx in 0..2200 {
            let sample = sample(cx * 16);
            cache.insert(style, (cx, 0, sample.x, sample.y, sample.z),
                &[&sample], [0, 0, 0], vec![vertex()]);
        }
        cache.retain_visible(&active);
        assert_eq!(cache.entry_count(), 2200);
        assert!(cache.get(style, (0, 0, 0, 90, 0), &[&sample(0)]).is_some());
    }
    #[test]
    fn cache_retains_all_visible_chunks_and_discards_departed_chunks() {
        let mut c = ChunkMeshCache::default();
        let style = ChunkMeshCache::style(None, false, Quality::Balanced);
        for cx in -4..=4 {
            let voxel = sample(cx * 16);
            c.insert(style, (cx, 0, voxel.x, voxel.y, voxel.z),
                &[&voxel], [0, 0, 0], vec![vertex()]);
        }
        let active: HashSet<_> = (-4..=4).filter(|cx| *cx >= 0).map(|cx| (cx, 0)).collect();
        c.retain_visible(&active);
        assert_eq!(c.entry_count(), 5);
        let voxel = sample(0);
        assert!(c.get(style, (0, 0, 0, 90, 0), &[&voxel]).is_some());
        let old = sample(-16);
        assert!(c.get(style, (-1, 0, -16, 90, 0), &[&old]).is_none());
    }
}
