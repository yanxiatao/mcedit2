//! Bounded exact-chunk spectator geometry cache. Never stores approximate LOD.
//! Equality of original source voxels invalidates changed seam flags, biomes,
//! weighted model states and edited blocks without hash-collision risks.
use crate::{
    gpu_mesh::{Quality, Vertex},
    textures::TextureBank,
};
use mcedit2_world::surface::{SurfaceVoxel, VoxelMaterial};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

const MAX_VERTICES: usize = 450_000;
const MAX_SOURCE_VOXELS: usize = 100_000;
const MAX_CHUNKS: usize = 2048;
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
    order: VecDeque<Key>,
    used_vertices: usize,
    used_voxels: usize,
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
            self.order.clear();
            self.used_vertices = 0;
            self.used_voxels = 0;
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
        let result = (Arc::clone(&entry.vertices), entry.origin);
        self.order.retain(|k| *k != key);
        self.order.push_back(key);
        Some(result)
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
            || vertices.len() > MAX_VERTICES
            || source.len() > MAX_SOURCE_VOXELS
        {
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.used_vertices = self.used_vertices.saturating_sub(old.vertices.len());
            self.used_voxels = self.used_voxels.saturating_sub(old.source.len());
        }
        self.order.retain(|k| *k != key);
        self.used_vertices += vertices.len();
        self.used_voxels += source.len();
        self.entries.insert(
            key,
            Entry {
                source: source.iter().map(|v| (*v).clone()).collect(),
                origin,
                vertices: Arc::new(vertices),
            },
        );
        self.order.push_back(key);
        while self.entries.len() > MAX_CHUNKS
            || self.used_vertices > MAX_VERTICES
            || self.used_voxels > MAX_SOURCE_VOXELS
        {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            if let Some(old) = self.entries.remove(&evicted) {
                self.used_vertices = self.used_vertices.saturating_sub(old.vertices.len());
                self.used_voxels = self.used_voxels.saturating_sub(old.source.len());
            }
        }
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
    fn bounded_cache_eviction() {
        let mut c = ChunkMeshCache::default();
        let s = ChunkMeshCache::style(None, false, Quality::Balanced);
        for n in 0..(MAX_CHUNKS as i32 + 3) {
            let v = sample(n * 16);
            c.insert(s, (n, 0, n * 16, 90, 0), &[&v], [0, 0, 0], vec![vertex()]);
        }
        assert!(c.entries.len() <= MAX_CHUNKS);
        assert!(c.get(s, (0, 0, 0, 90, 0), &[&sample(0)]).is_none());
        let n = (MAX_CHUNKS + 2) as i32;
        assert!(c
            .get(s, (n, 0, n * 16, 90, 0), &[&sample(n * 16)])
            .is_some());
    }
}
