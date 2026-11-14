//! Continuous Minecraft spectator terrain streaming: complete chunk rings,
//! progressive source LOD, bounded reuse, no guessed absent source chunks.
//! This is display-only: sampled voxels must never enter edit/selection APIs.
use crate::Message;
use iced::{futures::StreamExt, Task};
use mcedit2_world::{
    editing::Session,
    far_view::{self, FarChunkPatch, FarChunkRequest},
    surface::{
        exact_face_exposed_against, SurfaceView, SurfaceVoxel, FACE_NEG_X, FACE_NEG_Z, FACE_POS_X,
        FACE_POS_Z,
    },
};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

// 普通相机默认显示真实区块，使用游戏常用的 12 区块视距。
pub(crate) const DEFAULT_RADIUS: u16 = 12;
const MAX_READ_BATCH: usize = 32;
/// 仅验收构建保留上一版串行调度对照；产品始终重叠推进流水阶段。
pub(crate) fn serial_reference() -> bool {
    #[cfg(test)]
    {
        std::env::var_os("MCEDIT2_STREAM_SERIAL_REFERENCE").is_some()
    }
    #[cfg(not(test))]
    {
        false
    }
}
fn reader_threads() -> usize {
    crate::terrain_budget::limits().readers
}
#[cfg(windows)]
fn physical_memory() -> Result<u64, String> {
    #[repr(C)]
    struct Status {
        length: u32,
        load: u32,
        total: u64,
        available: u64,
        page_total: u64,
        page_available: u64,
        virtual_total: u64,
        virtual_available: u64,
        extended: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(value: *mut Status) -> i32;
    }
    let mut value = Status {
        length: std::mem::size_of::<Status>() as u32,
        load: 0,
        total: 0,
        available: 0,
        page_total: 0,
        page_available: 0,
        virtual_total: 0,
        virtual_available: 0,
        extended: 0,
    };
    // SAFETY: Windows receives a correctly sized writable MEMORYSTATUSEX structure.
    if unsafe { GlobalMemoryStatusEx(&mut value) } == 0 {
        return Err(format!(
            "读取物理内存失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(value.total)
}
#[cfg(target_os = "linux")]
fn physical_memory() -> Result<u64, String> {
    let data = std::fs::read_to_string("/proc/meminfo").map_err(|e| e.to_string())?;
    let kib = data
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|line| line.split_whitespace().next())
        .ok_or("/proc/meminfo 缺少 MemTotal")?
        .parse::<u64>()
        .map_err(|e| format!("MemTotal 无效：{e}"))?;
    kib.checked_mul(1024)
        .ok_or_else(|| "MemTotal 超出地址范围".into())
}
#[cfg(not(any(windows, target_os = "linux")))]
fn physical_memory() -> Result<u64, String> {
    Err("此平台尚未提供物理内存查询".into())
}
fn memory_budgets_for(total: u64, configured: Option<&str>) -> Result<(usize, usize), String> {
    let default = (total / 32).clamp(64 * 1024 * 1024, 2 * 1024 * 1024 * 1024);
    let cache = configured
        .map(|value| {
            value
                .trim()
                .parse::<u64>()
                .map_err(|e| format!("MCEDIT_TERRAIN_CACHE_MB 必须是非负整数 MiB：{e}"))?
                .checked_mul(1024 * 1024)
                .ok_or_else(|| "MCEDIT_TERRAIN_CACHE_MB 超出地址范围".to_owned())
        })
        .transpose()?
        .unwrap_or(default);
    let batch = (total / 16).max(128 * 1024 * 1024);
    Ok((
        usize::try_from(cache).map_err(|_| "地形缓存预算超出地址范围")?,
        usize::try_from(batch).map_err(|_| "地形批次预算超出地址范围")?,
    ))
}
fn memory_budgets() -> Result<(usize, usize), String> {
    let configured = match std::env::var("MCEDIT_TERRAIN_CACHE_MB") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(format!("MCEDIT_TERRAIN_CACHE_MB 无效：{error}")),
    };
    memory_budgets_for(physical_memory()?, configured.as_deref())
}
type Key = (i32, i32, u8);
fn key(request: FarChunkRequest) -> Key {
    (request.cx, request.cz, request.span)
}
#[derive(Debug)]
pub(crate) struct PatchCost {
    allocations: mcedit2_world::source_memory::Allocations,
    extra_bytes: usize,
}
impl PatchCost {
    pub(crate) fn new(patch: &FarChunkPatch) -> Self {
        let mut footprint = mcedit2_world::source_memory::Footprint::default();
        footprint.include(&patch.voxels, patch.voxels.capacity());
        let allocations = footprint.seal();
        let extra_bytes = std::mem::size_of::<FarChunkPatch>()
            + std::mem::size_of::<Self>()
            + 4 * std::mem::size_of::<usize>()
            + allocations.metadata_bytes()
            + patch.biomes.as_ref().map_or(0, |b| b.allocated_bytes())
            + patch.biome_neighbors.len()
                * std::mem::size_of::<((i32, i32), Arc<mcedit2_world::biome_source::Column>)>()
            + patch
                .biome_neighbors
                .iter()
                .map(|(_, column)| column.allocated_bytes())
                .sum::<usize>()
            + usize::from(!patch.biome_neighbors.is_empty()) * 2 * std::mem::size_of::<usize>()
            + patch
                .sections
                .as_ref()
                .map_or(0, |s| s.iter().map(|s| s.allocated_bytes()).sum::<usize>());
        Self {
            allocations,
            extra_bytes,
        }
    }
}
fn center_chunk(location: [f64; 3]) -> (i32, i32) {
    let convert = |value: f64| {
        // Clamp to Java's dimension coordinates before f64->i32.
        (value.floor() as i32).div_euclid(16)
    };
    (convert(location[0]), convert(location[2]))
}
fn stride(distance: i32) -> u8 {
    if distance <= 2 {
        1
    } else if distance <= 6 {
        2
    } else if distance <= 12 {
        4
    } else if distance <= 24 {
        8
    } else {
        16
    }
}
/// Unlike the old sparse 2047-chunk panorama, EVERY original chunk in the
/// chosen circular/square ring is scheduled. Only its visual sampling level
/// changes with distance; never insert synthetic blocks into unvisited holes.
pub(crate) fn plan(location: [f64; 3], radius: u16) -> Result<Vec<FarChunkRequest>, String> {
    if radius == 0 {
        return Err("Spectator streaming radius must be at least one chunk".into());
    }
    let (cx, cz) = center_chunk(location);
    let mut result = Vec::new();
    for ring in 0..=radius as i32 {
        let mut push = |dx: i32, dz: i32| {
            let x = cx as i64 + dx as i64;
            let z = cz as i64 + dz as i64;
            if x * 16 < -30_000_000
                || x * 16 + 15 > 30_000_000
                || z * 16 < -30_000_000
                || z * 16 + 15 > 30_000_000
            {
                return;
            }
            result.push(FarChunkRequest {
                cx: x as i32,
                cz: z as i32,
                span: stride(ring),
            });
        };
        if ring == 0 {
            push(0, 0);
            continue;
        }
        for n in -ring..=ring {
            push(n, -ring);
            push(n, ring);
        }
        for n in (-ring + 1)..ring {
            push(-ring, n);
            push(ring, n);
        }
    }
    Ok(result)
}
/// Quantize Minecraft yaw so mouse movement does not continuously re-sort.
fn look_sector(yaw: f32) -> u8 {
    if !yaw.is_finite() {
        return 0;
    }
    ((yaw.rem_euclid(360.0) / 45.0).round() as u8) % 8
}
fn front_vector(sector: u8) -> (i64, i64) {
    const VECTORS: [(i64, i64); 8] = [
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];
    VECTORS[sector as usize % VECTORS.len()]
}
/// 脚边四面先加载。更远处略优先视线方向，后方只延后一圈，
/// 避免首次转头时近处仍然是空的。
fn sort_facing(requests: &mut [FarChunkRequest], center: (i32, i32), sector: u8) {
    let (fx, fz) = front_vector(sector);
    requests.sort_by_key(|request| {
        let dx = i64::from(request.cx) - i64::from(center.0);
        let dz = i64::from(request.cz) - i64::from(center.1);
        let ring = dx.abs().max(dz.abs());
        let ahead = dx * fx + dz * fz;
        let behind = ring > 2 && 2 * ahead < -ring;
        (
            ring + i64::from(behind),
            ring,
            -ahead,
            (dx * fz - dz * fx).abs(),
            request.cx,
            request.cz,
        )
    });
}
fn prioritized_pending(
    requests: &[FarChunkRequest],
    center: (i32, i32),
    sector: u8,
) -> VecDeque<FarChunkRequest> {
    let mut ordered = requests.to_vec();
    sort_facing(&mut ordered, center, sector);
    ordered.into()
}
/// Per-chunk, append-only display order independent of background IO
/// scheduling. Streaming a new chunk appends it after already visible chunks
/// rather than inserting it into the middle of a large GPU vertex array.
/// Existing positions remain stable across camera rotation and LOD changes.
#[derive(Default)]
struct DisplayOrder {
    visible: Vec<FarChunkRequest>,
    positions: HashMap<(i32, i32), usize>,
}
impl DisplayOrder {
    fn publish(&mut self, request: FarChunkRequest) {
        let id = (request.cx, request.cz);
        if let Some(&index) = self.positions.get(&id) {
            self.visible[index] = request;
        } else {
            self.positions.insert(id, self.visible.len());
            self.visible.push(request);
        }
    }
    fn replan(&mut self, requested: &[FarChunkRequest]) {
        let index: HashMap<(i32, i32), FarChunkRequest> = requested
            .iter()
            .copied()
            .map(|req| ((req.cx, req.cz), req))
            .collect();
        self.visible.retain_mut(|req| {
            if let Some(new) = index.get(&(req.cx, req.cz)) {
                *req = *new;
                true
            } else {
                false
            }
        });
        self.positions = self
            .visible
            .iter()
            .enumerate()
            .map(|(index, req)| ((req.cx, req.cz), index))
            .collect();
    }
}
/// Pick the best available original source resolution for this chunk.
/// Completeness does not depend on a global visible-voxel quota.
/// When a spectator flies toward terrain, a cached coarse patch remains
/// visible until its exact read completes; on flying away, retain a cached
/// exact patch rather than degrading to a new sampled approximation.
/// All candidates refer to the same source chunk, not synthetic neighbors.
fn best_cached_resolution(
    req: FarChunkRequest,
    available: impl Fn(Key) -> Option<usize>,
) -> Option<Key> {
    if available(key(req)).is_some() {
        return Some(key(req));
    }
    for span in [1_u8, 2, 4, 8, 16] {
        let id = (req.cx, req.cz, span);
        if available(id).is_some() {
            return Some(id);
        }
    }
    None
}
/// A single background read may still be in the OS when the spectator flies
/// elsewhere. Inspect cancellation between *original* chunk reads so stale
/// jobs cannot keep scanning hundreds of now-irrelevant source chunks.
#[cfg(test)]
fn not_cancelled(flag: &AtomicBool) -> bool {
    !flag.load(Ordering::Acquire)
}
/// Reconcile exact block faces from independently loaded chunks using the
/// same source-world occlusion predicate as intra-chunk reads. Only proven
/// span=1 cells count as neighbors: approximate LOD, absent Sections, unknown
/// modded geometry and transparent boundaries must not invent occluders.
///
/// Compute masks before mutating voxel flags so borrowed source states can
/// be reused without cloning potentially tens of thousands of palettes.
/// A source-local seam index. Only boundary voxel offsets are stored;
/// the actual palette and state live in their immutable source Arc.
#[derive(Default)]
struct SeamIndex {
    cells: HashMap<(i32, i32, i32), usize>,
    chunks: HashSet<(i32, i32)>,
    chests: Vec<usize>,
    /// Section 位图及秩索引；每次邻格查询不再二分整列体素，也不复制状态。
    exact: HashMap<(i32, i32, i32), SectionCells>,
}
struct SectionCells {
    present: [u64; 64],
    prefix: [u16; 64],
    offsets: Vec<usize>,
}
impl SectionCells {
    fn at(&self, cell: usize) -> Option<usize> {
        let word = cell / 64;
        let bit = 1u64 << (cell % 64);
        let mask = self.present[word];
        (mask & bit != 0).then(|| {
            self.offsets[usize::from(self.prefix[word]) + (mask & (bit - 1)).count_ones() as usize]
        })
    }
}
fn section_cell(x: i32, y: i32, z: i32) -> ((i32, i32, i32), usize) {
    (
        (x.div_euclid(16), y.div_euclid(16), z.div_euclid(16)),
        (y.rem_euclid(16) * 256 + z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize,
    )
}
impl SeamIndex {
    fn build(voxels: &[SurfaceVoxel], cancelled: &dyn Fn() -> bool) -> Option<Self> {
        let mut result = Self::default();
        let mut exact = Vec::new();
        let mut previous_chunk = None;
        for (index, voxel) in voxels.iter().enumerate() {
            if index.is_multiple_of(256) && cancelled() {
                return None;
            }
            if crate::legacy_chest_context::candidate(voxel) {
                result.chests.push(index);
            }
            if voxel.span != 1 {
                continue;
            }
            exact.push(index);
            let chunk = (voxel.x.div_euclid(16), voxel.z.div_euclid(16));
            if previous_chunk != Some(chunk) {
                result.chunks.insert(chunk);
                previous_chunk = Some(chunk);
            }
            if !(matches!(voxel.x.rem_euclid(16), 0 | 15)
                || matches!(voxel.z.rem_euclid(16), 0 | 15))
            {
                continue;
            }
            result.cells.insert((voxel.x, voxel.y, voxel.z), index);
        }
        let key = |index: usize| {
            let v = &voxels[index];
            (v.y, v.z, v.x)
        };
        if !exact.is_sorted_by(|&a, &b| key(a) <= key(b)) {
            if cancelled() {
                return None;
            }
            exact.sort_unstable_by_key(|&index| key(index));
        }
        for index in exact {
            if index.is_multiple_of(256) && cancelled() {
                return None;
            }
            let voxel = &voxels[index];
            let (key, cell) = section_cell(voxel.x, voxel.y, voxel.z);
            let section = result.exact.entry(key).or_insert_with(|| SectionCells {
                present: [0; 64],
                prefix: [0; 64],
                offsets: Vec::new(),
            });
            let bit = 1u64 << (cell % 64);
            if section.present[cell / 64] & bit != 0 {
                log::error!(
                    "[mesh] 同一来源出现重复精确体素：{},{},{}",
                    voxel.x,
                    voxel.y,
                    voxel.z
                );
                return None;
            }
            section.present[cell / 64] |= bit;
            section.offsets.push(index);
        }
        for section in result.exact.values_mut() {
            let mut count = 0;
            for (prefix, word) in section.prefix.iter_mut().zip(section.present) {
                *prefix = count;
                count += word.count_ones() as u16;
            }
            section.offsets.shrink_to_fit();
        }
        (!cancelled()).then_some(result)
    }
}
/// Reuse boundary indexes across successive streamed snapshots. The index
/// stores integer offsets only, never borrowed references into expired data.
#[derive(Default)]
pub(crate) struct SeamIndexCache {
    near: Option<(Arc<SurfaceView>, Arc<SeamIndex>)>,
    patches: HashMap<usize, (Arc<FarChunkPatch>, Arc<SeamIndex>)>,
    /// Empty or sampled-only far patches share this index; no per-patch
    /// allocation of an empty hash table is needed at extreme view radii.
    empty: Arc<SeamIndex>,
    filtered_near: Option<(Arc<SurfaceView>, HashSet<(i32, i32)>, Arc<SurfaceView>)>,
}
impl SeamIndexCache {
    fn indexed(
        &mut self,
        near: &Arc<SurfaceView>,
        patches: &[Arc<FarChunkPatch>],
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Vec<Arc<SeamIndex>>> {
        if cancelled() {
            return None;
        }
        let near_index = match &self.near {
            Some((previous, index)) if Arc::ptr_eq(previous, near) => Arc::clone(index),
            _ => {
                let index = Arc::new(SeamIndex::build(&near.voxels, cancelled)?);
                self.near = Some((Arc::clone(near), Arc::clone(&index)));
                index
            }
        };
        let mut indexes = Vec::with_capacity(patches.len() + 1);
        indexes.push(near_index);
        let mut current = HashSet::with_capacity(patches.len());
        for patch in patches {
            if cancelled() {
                return None;
            }
            let key = Arc::as_ptr(patch) as usize;
            current.insert(key);
            let index = match self.patches.get(&key) {
                Some((old, index)) if Arc::ptr_eq(old, patch) => Arc::clone(index),
                _ => {
                    let candidate = SeamIndex::build(&patch.voxels, cancelled)?;
                    let index = if candidate.exact.is_empty() {
                        Arc::clone(&self.empty)
                    } else {
                        Arc::new(candidate)
                    };
                    self.patches
                        .insert(key, (Arc::clone(patch), Arc::clone(&index)));
                    index
                }
            };
            indexes.push(index);
        }
        self.patches.retain(|key, _| current.contains(key));
        Some(indexes)
    }
}
/// Read-only original chunk-boundary lookup. Horizontal edge candidates are
/// resolved using immutable per-source indexes; unchanged sources never
/// have to rehash all their visible voxels on every streamed publication.
pub(crate) struct ExactNeighbors<'a> {
    sources: Vec<&'a [SurfaceVoxel]>,
    indexed: Vec<Arc<SeamIndex>>,
    by_chunk: HashMap<(i32, i32), Vec<usize>>,
}
impl<'a> ExactNeighbors<'a> {
    /// 复用不可变来源索引中的少量箱子位置，不为配对重新扫描完整高度体素。
    pub(crate) fn chests(&self) -> impl Iterator<Item = &'a SurfaceVoxel> + '_ {
        self.indexed
            .iter()
            .enumerate()
            .flat_map(move |(source, index)| {
                index
                    .chests
                    .iter()
                    .map(move |&offset| &self.sources[source][offset])
            })
    }
    fn from_indexed(sources: Vec<&'a [SurfaceVoxel]>, indexed: Vec<Arc<SeamIndex>>) -> Self {
        let mut by_chunk: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (index, source) in indexed.iter().enumerate() {
            for chunk in &source.chunks {
                by_chunk.entry(*chunk).or_default().push(index);
            }
        }
        Self {
            sources,
            indexed,
            by_chunk,
        }
    }
    #[cfg(test)]
    pub(crate) fn new(sources: &[&'a [SurfaceVoxel]]) -> Self {
        Self::new_cancellable(sources, &|| false).expect("不可取消的源索引构建")
    }
    pub(crate) fn new_cancellable(
        sources: &[&'a [SurfaceVoxel]],
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        let indexed = sources
            .iter()
            .map(|source| SeamIndex::build(source, cancelled).map(Arc::new))
            .collect::<Option<Vec<_>>>()?;
        Some(Self::from_indexed(sources.to_vec(), indexed))
    }
    pub(crate) fn at(&self, position: (i32, i32, i32)) -> Option<&'a SurfaceVoxel> {
        let sources = self
            .by_chunk
            .get(&(position.0.div_euclid(16), position.2.div_euclid(16)))?;
        for &source in sources {
            let voxels = self.sources[source];
            let (section, cell) = section_cell(position.0, position.1, position.2);
            if let Some(index) = self.indexed[source]
                .exact
                .get(&section)
                .and_then(|s| s.at(cell))
            {
                return Some(&voxels[index]);
            }
        }
        None
    }
    pub(crate) fn faces(&self, voxel: &SurfaceVoxel) -> u8 {
        if voxel.span != 1 {
            return voxel.faces;
        }
        let mut faces = voxel.faces;
        let source_chunk = (voxel.x.div_euclid(16), voxel.z.div_euclid(16));
        for (dx, dz, face) in [
            (1, 0, FACE_POS_X),
            (-1, 0, FACE_NEG_X),
            (0, 1, FACE_POS_Z),
            (0, -1, FACE_NEG_Z),
        ] {
            if faces & face == 0 {
                continue;
            }
            let (Some(x), Some(z)) = (voxel.x.checked_add(dx), voxel.z.checked_add(dz)) else {
                continue;
            };
            let other_chunk = (x.div_euclid(16), z.div_euclid(16));
            if other_chunk == source_chunk {
                continue;
            }
            if let Some(candidates) = self.by_chunk.get(&other_chunk) {
                for &index in candidates {
                    if let Some(&position) = self.indexed[index].cells.get(&(x, voxel.y, z)) {
                        let neighbor = &self.sources[index][position];
                        if !exact_face_exposed_against(
                            &voxel.state,
                            voxel.kind,
                            Some((&neighbor.state, neighbor.kind)),
                        ) {
                            faces &= !face;
                        }
                        break;
                    }
                }
            }
        }
        faces
    }
}
#[cfg(test)]
fn reconcile_exact_seams(view: &mut SurfaceView) {
    let masks = {
        let index = ExactNeighbors::new(&[view.voxels.as_slice()]);
        view.voxels
            .iter()
            .map(|v| index.faces(v))
            .collect::<Vec<_>>()
    };
    for (voxel, faces) in view.voxels.iter_mut().zip(masks) {
        voxel.faces = faces;
    }
}
/// 精确/采样区块共用的不可变来源证明，覆盖接缝、液面八邻格与近景窗口。
/// 保留真实 Arc，既不复制方块，也不把可复用的地址值当作来源证明。
#[derive(Clone)]
pub(crate) struct ChunkContext {
    pub source: Arc<FarChunkPatch>,
    pub neighbors: Vec<Arc<FarChunkPatch>>,
    pub near: Option<Arc<SurfaceView>>,
}
impl ChunkContext {
    pub(crate) fn coordinates(&self) -> (i32, i32) {
        (self.source.tile.chunk_x, self.source.tile.chunk_z)
    }
    pub(crate) fn sampled_y_max(&self, y_max: i32) -> Option<i32> {
        self.source
            .voxels
            .first()
            .is_some_and(|v| v.span > 1)
            .then_some(y_max)
    }
    pub(crate) fn same_sources(&self, other: &Self) -> bool {
        self.same_local_source(other)
            && self.neighbors.len() == other.neighbors.len()
            && self
                .neighbors
                .iter()
                .zip(&other.neighbors)
                .all(|(a, b)| Arc::ptr_eq(a, b))
    }
    pub(crate) fn same_local_source(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source)
            && match (&self.near, &other.near) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}
pub(crate) type Columns = Arc<HashSet<(i32, i32)>>;
pub(crate) type DisplayColumns = Arc<Mutex<Option<Columns>>>;
pub(crate) fn prepare_ahead() -> bool {
    #[cfg(test)]
    if std::env::var_os("MCEDIT2_READ_AHEAD_ONLY").is_some() {
        return false;
    }
    true
}
/// 不可变准备来源与独立的当前显示范围；外围 GPU 常驻领先实际显示。
#[derive(Clone)]
pub(crate) struct SceneSnapshot {
    pub(crate) visible_columns: Option<Columns>,
    pub(crate) display_columns: DisplayColumns,
    pub(crate) edit_refresh: bool,
    seed: Option<i64>,
    geometry: Option<mcedit2_world::world_data::Geometry>,
    near: Arc<SurfaceView>,
    patches: Vec<Arc<FarChunkPatch>>,
    seam_cache: Arc<Mutex<SeamIndexCache>>,
    /// The complete immutable chunk mesh generation is committed by the
    /// worker only after every source part finishes successfully.
    pub(crate) ready: Arc<Mutex<Option<Arc<crate::spectator::StreamMesh>>>>,
}
impl SceneSnapshot {
    pub(crate) fn biomes(&self) -> mcedit2_world::biome_source::Region {
        let mut biomes = (*self.near.biomes).clone();
        biomes.set_seed(self.seed);
        if let Some(geometry) = self.geometry {
            biomes.set_geometry(geometry);
        }
        for patch in &self.patches {
            for (coordinate, column) in patch.biome_neighbors.iter() {
                biomes.insert(coordinate.0, coordinate.1, Arc::clone(column));
            }
        }
        // 明确本列和近景优先于邻域副本；暂存群系编辑不能被较早的邻列快照盖回去。
        for (coordinate, column) in self.near.biomes.columns() {
            biomes.insert(coordinate.0, coordinate.1, Arc::clone(column));
        }
        for patch in &self.patches {
            if let Some(column) = &patch.biomes {
                biomes.insert(patch.tile.chunk_x, patch.tile.chunk_z, Arc::clone(column));
            }
        }
        biomes
    }
    /// 在后台消除 exact patch 和 near 重叠，主线程只搬运 Arc。
    pub(crate) fn filtered_for_mesh(&self) -> Self {
        let replaced: HashSet<_> = self
            .patches
            .iter()
            .filter(|p| {
                (p.tile.source_present || !p.voxels.is_empty())
                    && p.voxels.first().is_none_or(|v| v.span == 1)
            })
            .filter(|p| {
                let x = i64::from(p.tile.chunk_x) * 16;
                let z = i64::from(p.tile.chunk_z) * 16;
                x < i64::from(self.near.x) + i64::from(self.near.width)
                    && x + 16 > i64::from(self.near.x)
                    && z < i64::from(self.near.z) + i64::from(self.near.depth)
                    && z + 16 > i64::from(self.near.z)
            })
            .map(|p| (p.tile.chunk_x, p.tile.chunk_z))
            .collect();
        let mut cache = self.seam_cache.lock().expect("区块流接缝缓存锁被毒化");
        let near = if let Some((old, coverage, result)) = &cache.filtered_near {
            if Arc::ptr_eq(old, &self.near) && coverage == &replaced {
                Some(Arc::clone(result))
            } else {
                None
            }
        } else {
            None
        };
        let near = near.unwrap_or_else(|| {
            let result = if replaced.is_empty()
                || !self
                    .near
                    .voxels
                    .iter()
                    .any(|v| replaced.contains(&(v.x.div_euclid(16), v.z.div_euclid(16))))
            {
                Arc::clone(&self.near)
            } else {
                let mut result = (*self.near).clone();
                result
                    .voxels
                    .retain(|v| !replaced.contains(&(v.x.div_euclid(16), v.z.div_euclid(16))));
                Arc::new(result)
            };
            cache.filtered_near = Some((Arc::clone(&self.near), replaced, Arc::clone(&result)));
            result
        });
        drop(cache);
        Self {
            edit_refresh: self.edit_refresh,
            visible_columns: self.visible_columns.clone(),
            display_columns: Arc::clone(&self.display_columns),
            near,
            seed: self.seed,
            geometry: self.geometry,
            patches: self.patches.clone(),
            seam_cache: Arc::clone(&self.seam_cache),
            ready: Arc::clone(&self.ready),
        }
    }
    /// All entries borrow their original backing Arc buffers. Consuming the
    /// iterator does not build a global contiguous voxel array.
    pub(crate) fn sources(&self) -> impl Iterator<Item = &[SurfaceVoxel]> {
        std::iter::once(self.near.voxels.as_slice())
            .chain(self.patches.iter().map(|patch| patch.voxels.as_slice()))
    }
    pub(crate) fn origin(&self) -> [i32; 3] {
        [self.near.x, self.near.y_min, self.near.z]
    }
    pub(crate) fn y_max(&self) -> i32 {
        self.near.y_max
    }
    pub(crate) fn exact_neighbors_cancellable(
        &self,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<ExactNeighbors<'_>> {
        let indexed = self
            .seam_cache
            .lock()
            .expect("区块流接缝缓存锁被毒化")
            .indexed(&self.near, &self.patches, cancelled)?;
        Some(ExactNeighbors::from_indexed(
            self.sources().collect(),
            indexed,
        ))
    }
    /// Build an O(N) world-coordinate lookup once per scene publication.
    /// Calling a linear scan per patch would scale quadratically for a
    /// 128-chunk-radius world (tens of thousands of loaded patches).
    pub(crate) fn chunk_contexts(&self) -> Vec<ChunkContext> {
        let by_coordinate = self
            .patches
            .iter()
            .map(|patch| ((patch.tile.chunk_x, patch.tile.chunk_z), Arc::clone(patch)))
            .collect::<HashMap<_, _>>();
        self.patches
            .iter()
            .map(|source| {
                let cx = source.tile.chunk_x;
                let cz = source.tile.chunk_z;
                let mut neighbors = Vec::new();
                for dx in -1..=1_i32 {
                    for dz in -1..=1_i32 {
                        if dx == 0 && dz == 0 {
                            continue;
                        }
                        let Some(x) = cx.checked_add(dx) else {
                            continue;
                        };
                        let Some(z) = cz.checked_add(dz) else {
                            continue;
                        };
                        if let Some(patch) = by_coordinate.get(&(x, z)) {
                            neighbors.push(Arc::clone(patch));
                        }
                    }
                }
                let x = cx as i64 * 16;
                let z = cz as i64 * 16;
                let near = (x + 32 > self.near.x as i64
                    && x - 16 < self.near.x as i64 + self.near.width as i64
                    && z + 32 > self.near.z as i64
                    && z - 16 < self.near.z as i64 + self.near.depth as i64)
                    .then(|| Arc::clone(&self.near));
                ChunkContext {
                    source: Arc::clone(source),
                    neighbors,
                    near,
                }
            })
            .collect()
    }
    #[cfg(test)]
    fn patch_count(&self) -> usize {
        self.patches.len()
    }
    #[cfg(test)]
    pub(crate) fn materialize_for_test(&self) -> SurfaceView {
        let filtered = self.filtered_for_mesh();
        let mut result = (*filtered.near).clone();
        result.biomes = Arc::new(filtered.biomes());
        for patch in &self.patches {
            result.voxels.extend(patch.voxels.iter().cloned());
            result.source_tiles.push(patch.tile.clone());
            result.source_chunks += usize::from(patch.tile.source_present);
            result.missing_columns += patch.tile.unknown_columns;
        }
        reconcile_exact_seams(&mut result);
        result
    }
}
#[derive(Clone, Debug)]
pub(crate) struct PreparedPlan {
    priority_only: bool,
    requests: Vec<FarChunkRequest>,
    ordered: VecDeque<FarChunkRequest>,
    center: (i32, i32),
    radius: u16,
    sector: u8,
}
#[derive(Clone)]
struct PlanRequest {
    location: [f64; 3],
    radius: u16,
    sector: u8,
    nonce: u64,
    id: u64,
    sampled: bool,
    pending: Option<Arc<VecDeque<FarChunkRequest>>>,
}
impl PlanRequest {
    fn prepare(&self, cancelled: &AtomicBool) -> Result<PreparedPlan, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("区块计划已取消".into());
        }
        let center = center_chunk(self.location);
        let priority_only = self.pending.is_some();
        let requests = if let Some(pending) = &self.pending {
            pending.iter().copied().collect::<Vec<_>>()
        } else {
            let mut requests = plan(self.location, self.radius)?;
            if !self.sampled {
                for item in &mut requests {
                    item.span = 1;
                }
            }
            requests
        };
        let mut ordered = prioritized_pending(&requests, center, self.sector);
        // 外围两圈低优先级准备；格式边界之外不规划，保留既有源缓存预算。
        let outer = self.radius.saturating_add(2);
        if !priority_only
            && !self.sampled
            && !serial_reference()
            && outer <= far_view::MAX_DISTANCE_CHUNKS
        {
            let ahead: Vec<_> = plan(self.location, outer)?
                .into_iter()
                .filter(|r| {
                    (r.cx - center.0).abs().max((r.cz - center.1).abs()) > i32::from(self.radius)
                })
                .map(|mut r| {
                    r.span = 1;
                    r
                })
                .collect();
            ordered.extend(prioritized_pending(&ahead, center, self.sector));
        }
        if cancelled.load(Ordering::Acquire) {
            return Err("区块计划已取消".into());
        }
        Ok(PreparedPlan {
            priority_only,
            requests: if priority_only { Vec::new() } else { requests },
            ordered,
            center,
            radius: self.radius,
            sector: self.sector,
        })
    }
}

pub(crate) struct Stream {
    pub key: (usize, usize),
    pub source: Arc<Session>,
    assets: Option<Arc<crate::textures::TextureBank>>,
    show_textures: bool,
    block_layers: crate::block_visibility::Layers,
    pub center: (i32, i32),
    pub radius: u16,
    look_sector: u8,
    plan_id: u64,
    active_plan: Option<(u64, u64)>,
    active_request: Option<PlanRequest>,
    plan_ready: bool,
    /// 仅明确切换候选来源时重新排队；计划失败本身不触发自动重试。
    source_dirty: bool,
    pending_sector: u8,
    queued_plan: Option<PlanRequest>,
    plan_cancel: Arc<AtomicBool>,
    /// Display source chunks in publication order, not re-sorted with yaw.
    display_order: DisplayOrder,
    pub sampled: bool,
    pub nonce: u64,
    pub running: bool,
    batch_id: u64,
    batch_cancelled: bool,
    active_nonce: u64,
    cancellation: Arc<AtomicBool>,
    reader_pool: Option<Result<Arc<crate::stream_readers::Readers>, String>>,
    pub complete: usize,
    published: usize,
    last_publication: std::time::Instant,
    pub(crate) read_rate: iced_wgpu::window::frame_statistics::RollingRate,
    pub errors: usize,
    pub total: usize,
    /// Each source chunk has at most one active LOD in this view.
    wanted: Vec<FarChunkRequest>,
    wanted_keys: HashSet<Key>,
    /// 当前显示列及外围预备列；显示、拾取和完成度继续只采用 wanted。
    read_keys: HashSet<Key>,
    display_columns: DisplayColumns,
    prefetch_dirty: bool,
    pending: VecDeque<FarChunkRequest>,
    inflight: HashSet<Key>,
    cache: BTreeMap<Key, Arc<FarChunkPatch>>,
    cache_costs: BTreeMap<Key, Arc<PatchCost>>,
    cache_memory: mcedit2_world::source_memory::Ledger,
    cache_extra_bytes: usize,
    seam_cache: Arc<Mutex<SeamIndexCache>>,
    cached_voxels: usize,
    cache_bytes: usize,
    cache_budget: usize,
    batch_budget: usize,
    reader_threads: usize,
    pub dirty: bool,
    pub edit_refresh: bool,
    edit_reload: HashSet<(i32, i32)>,
}
impl Stream {
    /// 候选场景切换仅撤销改动及邻接区块，保留其余源 Arc 与网格复用依据。
    pub(crate) fn replace_source(
        &mut self,
        source: Arc<Session>,
        changed: &std::collections::BTreeSet<(i32, i32)>,
    ) {
        assert_eq!(
            self.source.original_world().root,
            source.original_world().root
        );
        assert_eq!(self.source.dimension(), source.dimension());
        self.cancel_readers();
        self.reader_pool = None;
        self.source = source;
        self.read_rate = Default::default();
        self.cancellation = Arc::new(AtomicBool::new(false));
        self.plan_cancel = Arc::new(AtomicBool::new(false));
        self.active_plan = None;
        self.active_request = None;
        self.queued_plan = None;
        self.plan_ready = false;
        self.source_dirty = true;
        self.running = false;
        self.batch_cancelled = false;
        self.batch_id = self.batch_id.wrapping_add(1);
        self.inflight.clear();
        self.pending.clear();
        let affected: HashSet<_> = changed
            .iter()
            .flat_map(|&(cx, cz)| {
                (-1..=1).flat_map(move |dx| {
                    (-1..=1).filter_map(move |dz| Some((cx.checked_add(dx)?, cz.checked_add(dz)?)))
                })
            })
            .collect();
        self.edit_refresh = true;
        self.edit_reload.extend(
            self.cache
                .keys()
                .filter_map(|&(x, z, _)| affected.contains(&(x, z)).then_some((x, z))),
        );
        self.cache_costs.retain(|id, cost| {
            if affected.contains(&(id.0, id.1)) {
                self.cache_memory.remove(&cost.allocations);
                self.cache_extra_bytes -= cost.extra_bytes;
                false
            } else {
                true
            }
        });
        self.cache.retain(|id, _| !affected.contains(&(id.0, id.1)));
        self.refresh_cache_usage();
        self.seam_cache = Default::default();
        self.complete = self
            .wanted
            .iter()
            .filter(|r| self.cache.contains_key(&key(**r)))
            .count();
        self.published = self.complete;
        self.last_publication = std::time::Instant::now();
        self.errors = 0;
        self.dirty = true;
    }
    pub(crate) fn ready_to_publish_edit(&self) -> bool {
        !self.edit_refresh || (self.plan_ready && self.edit_reload.is_empty())
    }

    fn prioritize_edit_reads(&mut self) {
        if !self.edit_reload.is_empty() {
            self.pending
                .make_contiguous()
                .sort_by_key(|request| !self.edit_reload.contains(&(request.cx, request.cz)));
        }
    }
    /// 已安装源的实际共享载荷，读取不扫描世界或保留旧来源。
    pub(crate) fn source_memory(&self) -> mcedit2_world::source_memory::Usage {
        self.cache_memory.usage()
    }
    #[cfg(test)]
    pub(crate) fn verify_source_memory(&self) {
        let mut actual = mcedit2_world::source_memory::Footprint::default();
        for patch in self.cache.values() {
            actual.include(&patch.voxels, patch.voxels.capacity());
        }
        assert_eq!(
            actual.usage(),
            self.cache_memory.usage(),
            "活动缓存账本必须与实际持有的源缓冲及共享载荷一致"
        );
        assert!(self.cache.keys().eq(self.cache_costs.keys()));
        assert_eq!(
            self.cache_extra_bytes,
            self.cache_costs
                .values()
                .map(|cost| cost.extra_bytes)
                .sum::<usize>()
        );
    }
    fn refresh_cache_usage(&mut self) {
        let usage = self.cache_memory.usage();
        self.cached_voxels = usage.voxels;
        self.cache_bytes =
            usage.total_bytes() + self.cache_extra_bytes + self.cache_memory.metadata_bytes();
    }
    fn install_patch(&mut self, id: Key, patch: Arc<FarChunkPatch>, cost: Arc<PatchCost>) {
        if let Some(old) = self.cache_costs.remove(&id) {
            self.cache_memory.remove(&old.allocations);
            self.cache_extra_bytes -= old.extra_bytes;
        }
        self.cache.insert(id, patch);
        self.cache_memory.add(&cost.allocations);
        self.cache_extra_bytes += cost.extra_bytes;
        self.cache_costs.insert(id, cost);
        self.refresh_cache_usage();
    }
    pub(crate) fn is_idle(&self) -> bool {
        self.plan_ready
            && self.active_plan.is_none()
            && self.queued_plan.is_none()
            && !self.running
            && self.pending.is_empty()
            && self.inflight.is_empty()
    }
    pub(crate) fn with_resources(
        mut self,
        assets: Option<Arc<crate::textures::TextureBank>>,
        show_textures: bool,
        block_layers: crate::block_visibility::Layers,
    ) -> Self {
        self.assets = assets;
        self.show_textures = show_textures;
        self.block_layers = block_layers;
        self.reader_pool = None;
        self
    }
    pub(crate) fn matches_resources(
        &self,
        assets: Option<&Arc<crate::textures::TextureBank>>,
        show_textures: bool,
        block_layers: crate::block_visibility::Layers,
    ) -> bool {
        crate::source_visibility::identity(
            self.assets.as_ref(),
            self.show_textures,
            self.block_layers,
        ) == crate::source_visibility::identity(assets, show_textures, block_layers)
    }
    pub(crate) fn cancel_readers(&self) {
        self.cancellation.store(true, Ordering::Release);
        self.plan_cancel.store(true, Ordering::Release);
        if let Some(Ok(pool)) = &self.reader_pool {
            pool.stop();
        }
    }
    pub(crate) fn source_coordinates(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.wanted.iter().map(|r| (r.cx, r.cz))
    }
    #[cfg(test)]
    pub(crate) fn loaded_column_nonempty(&self, column: (i32, i32)) -> Option<bool> {
        self.cache
            .get(&(column.0, column.1, 1))
            .map(|patch| !patch.voxels.is_empty())
    }
    /// 元数据读取必须绑定已发布的范围；计划中不能把旧范围或初始空表当成新来源。
    pub(crate) fn source_plan_nonce(&self) -> Option<u64> {
        self.plan_ready.then_some(self.nonce)
    }
    /// 只认可当前显示范围中已读取的完整高度精确列；采样、缺失和未知列不作覆盖证明。
    /// 使用已有位置索引，飞行检查只访问窗口相交的列，不扫描整圈来源。
    pub(crate) fn covers_exact(&self, bounds: mcedit2_world::editing::Bounds) -> bool {
        // 新范围仍在后台规划时，旧显示范围中的真实不可变来源依然有效。
        // 等收到新计划后，positions 和 cache 一起裁剪，再按新显示范围核对。
        (bounds.minimum[0].div_euclid(16)..=bounds.maximum[0].div_euclid(16)).all(|x| {
            (bounds.minimum[2].div_euclid(16)..=bounds.maximum[2].div_euclid(16)).all(|z| {
                self.display_order.positions.contains_key(&(x, z))
                    && self.cache.get(&(x, z, 1)).is_some_and(|patch| {
                        patch.tile.source_present && patch.tile.unknown_columns == 0
                    })
            })
        })
    }
    pub fn new(
        key: (usize, usize),
        source: Arc<Session>,
        location: [f64; 3],
        radius: u16,
        yaw: f32,
        nonce: u64,
        sampled: bool,
    ) -> Result<Self, String> {
        let (cache_budget, batch_budget) = memory_budgets()?;
        let state = Self {
            key,
            source,
            assets: None,
            show_textures: false,
            block_layers: Default::default(),
            center: center_chunk(location),
            radius,
            look_sector: look_sector(yaw),
            plan_id: 0,
            active_plan: None,
            active_request: None,
            plan_ready: false,
            source_dirty: false,
            pending_sector: look_sector(yaw),
            queued_plan: None,
            plan_cancel: Arc::new(AtomicBool::new(false)),
            display_order: DisplayOrder::default(),
            sampled,
            nonce,
            running: false,
            batch_id: 0,
            batch_cancelled: false,
            active_nonce: nonce,
            cancellation: Arc::new(AtomicBool::new(false)),
            reader_pool: None,
            complete: 0,
            published: 0,
            last_publication: std::time::Instant::now(),
            read_rate: Default::default(),
            errors: 0,
            total: 0,
            wanted: Vec::new(),
            wanted_keys: HashSet::new(),
            read_keys: HashSet::new(),
            display_columns: Default::default(),
            prefetch_dirty: false,
            pending: VecDeque::new(),
            inflight: HashSet::new(),
            cache: BTreeMap::new(),
            cache_costs: BTreeMap::new(),
            cache_memory: Default::default(),
            cache_extra_bytes: 0,
            seam_cache: Arc::new(Mutex::new(SeamIndexCache::default())),
            cached_voxels: 0,
            cache_bytes: 0,
            cache_budget,
            batch_budget,
            reader_threads: reader_threads(),
            dirty: true,
            edit_refresh: false,
            edit_reload: HashSet::new(),
        };
        if radius == 0 {
            return Err("Spectator streaming radius must be at least one chunk".into());
        }
        Ok(state)
    }
    fn launch_plan(&mut self, request: PlanRequest) -> Task<Message> {
        self.active_plan = Some((request.nonce, request.id));
        self.active_request = Some(request.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        self.plan_cancel = Arc::clone(&cancel);
        let failure_id = (request.nonce, request.id);
        Task::perform(
            async move {
                let (send, recv) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let result = request.prepare(&cancel);
                    let _ = send.send((request.nonce, request.id, result));
                });
                recv.await.unwrap_or((
                    failure_id.0,
                    failure_id.1,
                    Err("区块计划线程异常退出".into()),
                ))
            },
            |(nonce, id, result)| Message::SpectatorStreamPlan(nonce, id, result),
        )
    }
    /// One active planner and one replaceable pending request bound thread count.
    pub fn replan(
        &mut self,
        location: [f64; 3],
        radius: u16,
        yaw: f32,
        nonce: u64,
    ) -> Task<Message> {
        if radius == 0 {
            log::error!("[terrain] 区块计划半径不能为零");
            return Task::none();
        }
        self.center = center_chunk(location);
        self.radius = radius;
        self.look_sector = look_sector(yaw);
        self.plan_ready = false;
        self.source_dirty = false;
        self.nonce = nonce;
        self.plan_id = self.plan_id.wrapping_add(1);
        let request = PlanRequest {
            location,
            radius,
            sector: self.look_sector,
            nonce,
            id: self.plan_id,
            sampled: self.sampled,
            pending: None,
        };
        // 相机移动只改变读取需求，不改变磁盘 / 编辑来源。保留已在读取的同源区块，
        // 逐块交接时按最新范围筛选；真正的来源更换由 replace_source 撤销整组任务。
        if !self.running {
            self.cancellation.store(true, Ordering::Release);
            self.cancellation = Arc::new(AtomicBool::new(false));
            self.inflight.clear();
        }
        if self.active_plan.is_some() {
            self.plan_cancel.store(true, Ordering::Release);
            self.queued_plan = Some(request);
            Task::none()
        } else {
            self.launch_plan(request)
        }
    }
    pub fn receive_plan(
        &mut self,
        nonce: u64,
        id: u64,
        result: Result<PreparedPlan, String>,
    ) -> Task<Message> {
        if self.active_plan != Some((nonce, id)) {
            return Task::none();
        }
        self.active_plan = None;
        let active_request = self.active_request.take();
        if let Some(next) = self.queued_plan.take() {
            return self.launch_plan(next);
        }
        if nonce != self.nonce || id != self.plan_id {
            return Task::none();
        }
        let plan = match result {
            Ok(value) => value,
            Err(error) => {
                log::error!("[terrain] 区块计划失败：{error}");
                self.errors += 1;
                if let Some(pending) = active_request.and_then(|r| r.pending) {
                    // 排序失败不丢失真实未读请求；明确报告错误并继续原合法队列。
                    self.pending = Arc::unwrap_or_clone(pending);
                    self.pending_sector = self.look_sector;
                } else {
                    // 新范围规划失败后不能把上一范围的未读请求当作新世代继续读取。
                    self.pending.clear();
                }
                return Task::none();
            }
        };
        if plan.priority_only {
            self.pending = plan
                .ordered
                .into_iter()
                .filter(|r| {
                    !self.cache.contains_key(&key(*r))
                        && !(self.running
                            && !self.batch_cancelled
                            && self.inflight.contains(&key(*r)))
                })
                .collect();
            self.pending
                .make_contiguous()
                .sort_by_key(|r| !self.wanted_keys.contains(&key(*r)));
            self.pending_sector = plan.sector;
            self.prioritize_edit_reads();
            let reorder = self.reorder_pending();
            return Task::batch([reorder, self.fetch()]);
        }
        self.plan_ready = true;
        self.display_order.replan(&plan.requests);
        self.read_keys = plan.ordered.iter().copied().map(key).collect();
        let needed: HashSet<_> = self.read_keys.iter().map(|r| (r.0, r.1)).collect();
        self.edit_reload.retain(|key| needed.contains(key));
        self.cache_costs.retain(|id, cost| {
            if needed.contains(&(id.0, id.1)) {
                true
            } else {
                self.cache_memory.remove(&cost.allocations);
                self.cache_extra_bytes -= cost.extra_bytes;
                false
            }
        });
        self.cache.retain(|id, _| needed.contains(&(id.0, id.1)));
        self.refresh_cache_usage();
        self.complete = plan
            .requests
            .iter()
            .filter(|r| self.cache.contains_key(&key(**r)))
            .count();
        self.total = plan.requests.len();
        for request in &plan.requests {
            if self.cache.contains_key(&key(*request)) {
                self.display_order.publish(*request);
            }
        }
        self.pending = plan
            .ordered
            .into_iter()
            .filter(|r| {
                !self.cache.contains_key(&key(*r))
                    && !(self.running && !self.batch_cancelled && self.inflight.contains(&key(*r)))
            })
            .collect();
        self.center = plan.center;
        self.radius = plan.radius;
        self.pending_sector = plan.sector;
        self.wanted_keys = plan.requests.iter().copied().map(key).collect();
        *self.display_columns.lock().expect("显示区块范围锁被毒化") =
            (!self.sampled && !serial_reference() && prepare_ahead())
                .then(|| Arc::new(self.wanted_keys.iter().map(|r| (r.0, r.1)).collect()));
        self.wanted = plan.requests;
        self.prioritize_edit_reads();
        self.published = self.complete;
        self.last_publication = std::time::Instant::now();
        self.errors = 0;
        // 换区时先保留已显示场景；没有新源数据的裁边帧只会占用一次完整 GPU 交接。
        // 初次显示和编辑继续保留原 dirty；纯缓存范围变化在没有待收数据时立即发布。
        self.dirty |= self.complete == self.total;
        let reorder = self.reorder_pending();
        Task::batch([reorder, self.fetch()])
    }
    pub fn requires_replan(&self, location: [f64; 3], radius: u16) -> bool {
        let target = center_chunk(location);
        self.source_dirty
            || radius != self.radius
            || (target != self.center && (!serial_reference() || !self.running))
    }
    /// 转头只调整未读请求的优先级；不撤销读取、缓存、源快照或显示世代。
    pub fn orient(&mut self, yaw: f32) -> Task<Message> {
        let sector = look_sector(yaw);
        if sector == self.look_sector {
            return Task::none();
        }
        self.look_sector = sector;
        if let Some(request) = &mut self.queued_plan {
            request.sector = sector;
        }
        self.reorder_pending()
    }
    fn reorder_pending(&mut self) -> Task<Message> {
        if !self.plan_ready || self.active_plan.is_some() || self.pending_sector == self.look_sector
        {
            return Task::none();
        }
        if self.pending.len() < 2 {
            self.pending_sector = self.look_sector;
            return Task::none();
        }
        self.plan_id = self.plan_id.wrapping_add(1);
        let request = PlanRequest {
            location: [
                f64::from(self.center.0) * 16.,
                0.,
                f64::from(self.center.1) * 16.,
            ],
            radius: self.radius,
            sector: self.look_sector,
            nonce: self.nonce,
            id: self.plan_id,
            sampled: self.sampled,
            pending: Some(Arc::new(std::mem::take(&mut self.pending))),
        };
        self.launch_plan(request)
    }
    pub fn fetch(&mut self) -> Task<Message> {
        if !self.plan_ready || self.active_plan.is_some() || self.running || self.pending.is_empty()
        {
            return Task::none();
        }
        // 分辨率相同的请求用同一批次，以该分辨率的成本控制发布粒度。
        let span = self.pending.front().unwrap().span;
        let count = self
            .pending
            .iter()
            .take_while(|request| request.span == span)
            .count()
            .min(if span == 1 && !serial_reference() {
                (self.reader_threads * 4).min(MAX_READ_BATCH)
            } else {
                far_view::batch_size_for(span).min(MAX_READ_BATCH)
            });
        let requests: Vec<_> = self.pending.drain(..count).collect();
        self.inflight = requests.iter().copied().map(key).collect();
        let fallback = requests.clone();
        let nonce = self.nonce;
        self.active_nonce = nonce;
        self.batch_id = self.batch_id.wrapping_add(1);
        let batch_id = self.batch_id;
        let cancellation = Arc::clone(&self.cancellation);
        let batch_budget = self.batch_budget;
        if self.reader_pool.is_none() {
            self.reader_pool = Some(
                crate::stream_readers::Readers::start(
                    Arc::clone(&self.source),
                    self.assets.clone(),
                    self.show_textures,
                    self.block_layers,
                    self.reader_threads,
                )
                .map(Arc::new),
            );
        }
        let pool = self
            .reader_pool
            .as_ref()
            .expect("已创建读取线程池状态")
            .clone();
        self.running = true;
        // 一批仍共享冻结来源，界面按线程宽度成组接收；CPU / GPU 独立合并新候选。
        let group = if serial_reference() { count } else { 4 };
        let delivery_cancelled = Arc::clone(&cancellation);
        let reads = iced::futures::stream::once(async move {
            match pool {
                Ok(pool) => pool.read(requests, cancellation, batch_budget),
                Err(error) => Err(error),
            }
        });
        Task::run(
            reads.flat_map(move |receiver| {
                crate::stream_readers::deliveries(
                    receiver,
                    fallback.clone(),
                    group,
                    Arc::clone(&delivery_cancelled),
                )
            }),
            move |patches| Message::SpectatorStreamBatch(nonce, batch_id, patches),
        )
    }

    /// 完成结果只按实际源任务身份接收；相机计划变化不会使仍有效的同源结果过期。
    fn receive_part(&mut self, nonce: u64, batch_id: u64, results: crate::stream_readers::Patches) {
        if batch_id != self.batch_id || nonce != self.active_nonce || self.batch_cancelled {
            return;
        }
        let mut stored = 0;
        for (request, result) in results {
            let id = key(request);
            if !self.inflight.remove(&id) || !self.read_keys.contains(&id) {
                continue;
            }
            self.edit_reload.remove(&(request.cx, request.cz));
            if self.cache.contains_key(&id) {
                continue;
            }
            let demanded = self.wanted_keys.contains(&id);
            self.complete += usize::from(demanded);
            match result {
                Ok((patch, cost)) => {
                    if !demanded
                        && self.cache_bytes
                            + cost.allocations.usage().total_bytes()
                            + cost.extra_bytes
                            > self.cache_budget
                    {
                        log::debug!(
                            "[terrain-prefetch] 预读预算已满，不保留 {},{}",
                            request.cx,
                            request.cz
                        );
                        continue;
                    }
                    stored += u64::from(patch.tile.source_present);
                    self.install_patch(id, Arc::new(patch), cost);
                    self.prefetch_dirty |= !demanded;
                    if demanded {
                        self.display_order.publish(request);
                    }
                }
                Err(error) => {
                    self.errors += 1;
                    log::error!(
                        "[terrain] 原始区块 {},{} 读取失败：{error}",
                        request.cx,
                        request.cz
                    );
                }
            }
        }
        self.read_rate.record(stored, std::time::Instant::now());
    }
    #[cfg(test)]
    pub fn receive(&mut self, nonce: u64, batch_id: u64, results: crate::stream_readers::Patches) {
        let error = self.receive_delivery(
            nonce,
            batch_id,
            crate::stream_readers::Delivery {
                patches: results,
                finished: true,
                error: None,
            },
        );
        assert!(error.is_none(), "受控完整批次发生错误：{error:?}");
    }
    pub(crate) fn receive_delivery(
        &mut self,
        nonce: u64,
        batch_id: u64,
        delivery: crate::stream_readers::Delivery,
    ) -> Option<String> {
        if batch_id != self.batch_id || nonce != self.active_nonce {
            return None;
        }
        let error = if self.batch_cancelled {
            None
        } else {
            delivery.error
        };
        self.receive_part(nonce, batch_id, delivery.patches);
        if self.complete > self.published
            && (self.published < 25
                || (self.complete.saturating_sub(self.published)
                    >= if serial_reference() { 12 } else { 1 }
                    && self.last_publication.elapsed()
                        >= std::time::Duration::from_millis(if serial_reference() {
                            150
                        } else {
                            16
                        }))
                || (delivery.finished && self.pending.is_empty()))
        {
            self.dirty = true;
            self.published = self.complete;
            self.last_publication = std::time::Instant::now();
        }
        if !delivery.finished {
            return None;
        }
        if self.prefetch_dirty {
            self.dirty = true;
            self.prefetch_dirty = false;
        }
        self.running = false;
        self.inflight.clear();
        if let Some(error) = error {
            self.errors += 1;
            self.pending.clear();
            log::error!("[terrain] 批次读取失败：{error}");
            return Some(error);
        }
        if self.batch_cancelled {
            self.batch_cancelled = false;
            if self.plan_ready && self.active_plan.is_none() {
                let mut queued: HashSet<_> = self.pending.iter().copied().map(key).collect();
                for request in self.wanted.iter().copied() {
                    if !self.cache.contains_key(&key(request)) && queued.insert(key(request)) {
                        self.pending.push_back(request);
                    }
                }
            }
        }
        None
    }
    pub(crate) fn snapshot(&self, near: Arc<SurfaceView>) -> SceneSnapshot {
        // The source near-field window can be unaligned with chunk borders.
        // Never overlay an approximate patch onto the exact near volume.
        let overlaps = |req: FarChunkRequest| {
            let x = req.cx as i64 * 16;
            let z = req.cz as i64 * 16;
            x < near.x as i64 + near.width as i64
                && x + 16 > near.x as i64
                && z < near.z as i64 + near.depth as i64
                && z + 16 > near.z as i64
        };
        let mut patches = Vec::new();
        for req in &self.display_order.visible {
            if overlaps(*req) && req.span != 1 {
                continue;
            }
            let Some(best) = best_cached_resolution(*req, |id| {
                self.cache.get(&id).map(|patch| patch.voxels.len())
            }) else {
                continue;
            };
            if let Some(patch) = self.cache.get(&best) {
                patches.push(Arc::clone(patch));
            }
        }
        let visible_columns = self
            .display_columns
            .lock()
            .expect("显示区块范围锁被毒化")
            .clone();
        if visible_columns.is_some() {
            // 只扩展准备范围。窗口显示和拾取使用独立的当前范围，不能把预备列当成已显示列。
            let mut ahead: Vec<_> = self
                .cache
                .iter()
                .filter(|(id, _)| id.2 == 1 && !self.wanted_keys.contains(id))
                .map(|(_, patch)| Arc::clone(patch))
                .collect();
            ahead.sort_by_key(|patch| (patch.tile.chunk_x, patch.tile.chunk_z));
            patches.extend(ahead);
        }
        SceneSnapshot {
            visible_columns,
            display_columns: Arc::clone(&self.display_columns),
            edit_refresh: self.edit_refresh,
            near,
            seed: self.source.original_world().seed,
            geometry: self
                .source
                .original_world()
                .world_data
                .geometry(self.source.dimension())
                .ok(),
            patches,
            seam_cache: Arc::clone(&self.seam_cache),
            ready: Arc::new(Mutex::new(None)),
        }
    }
    pub fn status(&self) -> String {
        let prefetched = self
            .cache
            .keys()
            .filter(|key| !self.wanted_keys.contains(key))
            .count();
        format!(
            "原始区块 {}/{}，错误 {}，缓存 {}（预读 {}；{} MiB，参考预算 {} MiB），{} 源方块；读取线程最多 {}，排队 {}，批次预算 {} MiB；{}，半径 {} 区块。",
            self.complete,
            self.total,
            self.errors,
            self.cache.len(),
            prefetched,
            self.cache_bytes / 1048576, self.cache_budget / 1048576,
            self.cached_voxels,
            self.reader_threads, self.pending.len(), self.batch_budget / 1048576,
            if self.sampled {
                "完整近景 / 高度采样远景"
            } else {
                "全部 Section 完整几何"
            },
            self.radius
        )
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        // 切换世界同时关闭空闲读取线程，及时释放其资源快照。
        self.cancel_readers();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcedit2_world::surface::{SurfaceVoxel, VoxelMaterial};
    use std::collections::BTreeSet;

    #[test]
    fn seam_indexes_are_reused_for_unchanged_sources_and_retired_on_replacement() {
        use mcedit2_world::surface::SourceChunkTile;
        let near = Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 64,
            y_max: 95,
            width: 16,
            depth: 16,
            voxels: Vec::new(),
            source_tiles: Vec::new(),
            source_chunks: 0,
            missing_columns: 0,
        });
        let patch = || {
            Arc::new(FarChunkPatch {
                biome_neighbors: Default::default(),
                biomes: Default::default(),
                sections: None,
                tile: SourceChunkTile {
                    chunk_x: 1,
                    chunk_z: 0,
                    source_present: true,
                    known_columns: 256,
                    unknown_columns: 0,
                    exposed_voxels: 1,
                },
                voxels: vec![SurfaceVoxel::new(
                    [16, 70, 0],
                    mcedit2_world::surface::VoxelData {
                        span: 1,
                        fluid: None,
                        state: mcedit2_world::sections::State {
                            name: "minecraft:stone".into(),
                            properties: Vec::new(),
                        }
                        .into(),
                        biome: None,
                        kind: VoxelMaterial::Solid,
                        staged: false,
                        original_block_entity: false,
                        faces: mcedit2_world::surface::FACE_TOP,
                        light: Default::default(),
                    },
                )],
            })
        };
        let original = patch();
        let mut cache = SeamIndexCache::default();
        let first = cache
            .indexed(&near, &[Arc::clone(&original)], &|| false)
            .expect("不可取消的接缝索引");
        let same = cache
            .indexed(&near, &[Arc::clone(&original)], &|| false)
            .expect("不可取消的接缝索引");
        assert!(Arc::ptr_eq(&first[0], &same[0]));
        assert!(Arc::ptr_eq(&first[1], &same[1]));
        assert_eq!(cache.patches.len(), 1);
        let replacement = patch();
        let changed = cache
            .indexed(&near, &[replacement], &|| false)
            .expect("不可取消的接缝索引");
        assert!(Arc::ptr_eq(&first[0], &changed[0]));
        assert!(!Arc::ptr_eq(&first[1], &changed[1]));
        assert_eq!(cache.patches.len(), 1);
        let no_far = cache
            .indexed(&near, &[], &|| false)
            .expect("不可取消的接缝索引");
        assert!(Arc::ptr_eq(&first[0], &no_far[0]));
        assert!(cache.patches.is_empty());
        let moved = Arc::new((*near).clone());
        let new_near = cache
            .indexed(&moved, &[], &|| false)
            .expect("不可取消的接缝索引");
        assert!(!Arc::ptr_eq(&no_far[0], &new_near[0]));
        // 乱序、负坐标、跨位图字边界和空格均通过实际邻格入口核对。
        let mut cells = Vec::new();
        for cell in (0..4096).rev().filter(|cell| cell % 7 != 0) {
            let mut voxel = original.voxels[0].clone();
            voxel.x = -32 + cell % 16;
            voxel.z = -16 + cell / 16 % 16;
            voxel.y = -16 + cell / 256;
            cells.push(voxel);
        }
        let sources = [cells.as_slice()];
        let index = ExactNeighbors::new(&sources);
        for cell in 0..4096 {
            let position = (-32 + cell % 16, -16 + cell / 256, -16 + cell / 16 % 16);
            let found = index.at(position);
            assert_eq!(found.is_some(), cell % 7 != 0);
            if let Some(voxel) = found {
                assert_eq!((voxel.x, voxel.y, voxel.z), position);
            }
        }
    }

    #[test]
    fn direct_chunk_meshing_culls_original_seams_and_reuses_unchanged_parts() {
        use mcedit2_world::{
            sections::State,
            surface::{SourceChunkTile, SurfaceVoxel, ALL_FACES, FACE_NEG_X, FACE_POS_X},
        };
        let voxel = |x: i32| {
            SurfaceVoxel::new(
                [x, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: "minecraft:stone".into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    kind: VoxelMaterial::Solid,
                    faces: ALL_FACES,
                    light: Default::default(),
                },
            )
        };
        let near = Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 64,
            y_max: 95,
            width: 16,
            depth: 16,
            voxels: vec![voxel(15)],
            source_tiles: vec![],
            source_chunks: 1,
            missing_columns: 0,
        });
        let patch = Arc::new(FarChunkPatch {
            biome_neighbors: Default::default(),
            biomes: Default::default(),
            sections: None,
            tile: SourceChunkTile {
                chunk_x: 1,
                chunk_z: 0,
                source_present: true,
                known_columns: 256,
                unknown_columns: 0,
                exposed_voxels: 1,
            },
            voxels: vec![voxel(16), {
                let mut upper = voxel(16);
                upper.y = 86;
                upper
            }],
        });
        let snapshot = SceneSnapshot {
            visible_columns: None,
            display_columns: Default::default(),
            edit_refresh: false,
            geometry: None,
            seed: Default::default(),
            near: Arc::clone(&near),
            patches: vec![Arc::clone(&patch)],
            seam_cache: Arc::new(Mutex::new(SeamIndexCache::default())),
            ready: Arc::new(Mutex::new(None)),
        };
        let sources = snapshot.sources().collect::<Vec<_>>();
        let adjacent = ExactNeighbors::new(&sources);
        assert_eq!(adjacent.faces(&near.voxels[0]) & FACE_POS_X, 0);
        assert_eq!(adjacent.faces(&patch.voxels[0]) & FACE_NEG_X, 0);
        assert_eq!(near.voxels[0].faces, ALL_FACES);
        assert_eq!(patch.voxels[0].faces, ALL_FACES);
        let scene = crate::spectator::SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::clone(&near),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: crate::spectator::Camera::default(),
            quality: crate::gpu_mesh::Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: Some(Arc::new(snapshot.clone())),
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        let first =
            crate::spectator::build_stream_cancellable(&scene, &snapshot, &|| false).unwrap();
        assert_eq!(first.chunks.len(), 3);
        assert_eq!(
            first
                .chunks
                .iter()
                .map(|c| c.quads().len())
                .collect::<Vec<_>>(),
            vec![5, 5, 6]
        );
        assert_eq!(first.origin, [0, 64, 0]);
        let second =
            crate::spectator::build_stream_cancellable(&scene, &snapshot, &|| false).unwrap();
        assert!(first
            .chunks
            .iter()
            .zip(&second.chunks)
            .all(|(a, b)| Arc::ptr_eq(&a.quads(), &b.quads())));
        let mut edit = snapshot.clone();
        let patch = Arc::make_mut(&mut edit.patches[0]);
        patch.voxels.remove(0);
        let edited = crate::spectator::build_stream_cancellable(&scene, &edit, &|| false).unwrap();
        assert_eq!(edited.chunks.len(), 2);
        assert_eq!(edited.chunks[0].quads().len(), 6, "边界邻块应露出原面");
        assert!(
            Arc::ptr_eq(&first.chunks[2].quads(), &edited.chunks[1].quads()),
            "上方 Section 复用原网格"
        );
        // Losing the original neighboring chunk re-exposes the exact face
        // and must invalidate just that affected source chunk's mesh.
        let without_neighbor = SceneSnapshot {
            visible_columns: None,
            display_columns: Default::default(),
            edit_refresh: false,
            geometry: None,
            seed: Default::default(),
            near: Arc::clone(&near),
            patches: Vec::new(),
            seam_cache: Arc::new(Mutex::new(SeamIndexCache::default())),
            ready: Arc::new(Mutex::new(None)),
        };
        let after =
            crate::spectator::build_stream_cancellable(&scene, &without_neighbor, &|| false)
                .unwrap();
        assert_eq!(after.chunks.len(), 1);
        assert_eq!(after.chunks[0].quads().len(), 6);
    }

    #[test]
    fn distant_lod_reuses_unchanged_chunk_and_invalidates_adjacent_source() {
        use mcedit2_world::{
            sections::State,
            surface::{SourceChunkTile, SurfaceVoxel, ALL_FACES},
        };
        let near = Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 64,
            y_max: 96,
            width: 16,
            depth: 16,
            voxels: vec![],
            source_tiles: vec![],
            source_chunks: 0,
            missing_columns: 0,
        });
        let patch = |cx: i32| {
            Arc::new(FarChunkPatch {
                biome_neighbors: Default::default(),
                biomes: Default::default(),
                sections: None,
                tile: SourceChunkTile {
                    chunk_x: cx,
                    chunk_z: 0,
                    source_present: true,
                    known_columns: 256,
                    unknown_columns: 0,
                    exposed_voxels: 1,
                },
                voxels: vec![SurfaceVoxel::new(
                    [cx * 16, 72, 0],
                    mcedit2_world::surface::VoxelData {
                        span: 4,
                        fluid: None,
                        state: State {
                            name: "minecraft:stone".into(),
                            properties: vec![],
                        }
                        .into(),
                        biome: None,
                        kind: VoxelMaterial::Solid,
                        faces: ALL_FACES,
                        staged: false,
                        original_block_entity: false,
                        light: Default::default(),
                    },
                )],
            })
        };
        let target = patch(4);
        let unrelated = patch(10);
        let adjacent = patch(5);
        let snapshot = |patches: Vec<Arc<FarChunkPatch>>| SceneSnapshot {
            visible_columns: None,
            display_columns: Default::default(),
            edit_refresh: false,
            geometry: None,
            seed: Default::default(),
            near: Arc::clone(&near),
            patches,
            seam_cache: Arc::new(Mutex::new(SeamIndexCache::default())),
            ready: Arc::new(Mutex::new(None)),
        };
        let scene = crate::spectator::SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface: Arc::clone(&near),
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            camera: crate::spectator::Camera::default(),
            quality: crate::gpu_mesh::Quality::Balanced,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        let first_scene = snapshot(vec![Arc::clone(&target)]);
        let first =
            crate::spectator::build_stream_cancellable(&scene, &first_scene, &|| false).unwrap();
        let first_part = first.chunks.iter().find(|c| c.key.0 == 4).unwrap();
        assert!(!first_part.quads().is_empty());
        let second_scene = snapshot(vec![Arc::clone(&unrelated), Arc::clone(&target)]);
        let second =
            crate::spectator::build_stream_cancellable(&scene, &second_scene, &|| false).unwrap();
        let second_part = second.chunks.iter().find(|c| c.key.0 == 4).unwrap();
        // Arrival of an unrelated chunk changes the patch list index but
        // must neither change the GPU identity nor rebuild the old mesh.
        assert_eq!(first_part.key, second_part.key);
        assert!(Arc::ptr_eq(&first_part.quads(), &second_part.quads()));
        let third_scene = snapshot(vec![Arc::clone(&target), Arc::clone(&adjacent)]);
        let third =
            crate::spectator::build_stream_cancellable(&scene, &third_scene, &|| false).unwrap();
        let third_part = third.chunks.iter().find(|c| c.key.0 == 4).unwrap();
        assert_eq!(first_part.key, third_part.key);
        assert!(!Arc::ptr_eq(&first_part.quads(), &third_part.quads()));
        // A near-scene refresh far outside this patch halo is unrelated.
        let mut moved = (*near).clone();
        moved.y_max += 1;
        let fourth_scene = SceneSnapshot {
            visible_columns: None,
            display_columns: Default::default(),
            edit_refresh: false,
            geometry: None,
            seed: Default::default(),
            near: Arc::new(moved),
            patches: vec![Arc::clone(&target)],
            seam_cache: Arc::new(Mutex::new(SeamIndexCache::default())),
            ready: Arc::new(Mutex::new(None)),
        };
        let fourth =
            crate::spectator::build_stream_cancellable(&scene, &fourth_scene, &|| false).unwrap();
        assert_eq!(fourth.chunks[0].key, first_part.key);
    }

    #[test]
    fn immutable_chunk_snapshot_reuses_patch_arcs_and_materializes_on_demand() {
        use mcedit2_world::surface::{SourceChunkTile, SurfaceView};
        let near = Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 47,
            width: 16,
            depth: 16,
            voxels: Vec::new(),
            source_tiles: Vec::new(),
            source_chunks: 0,
            missing_columns: 0,
        });
        let patch = Arc::new(FarChunkPatch {
            biome_neighbors: Default::default(),
            biomes: Default::default(),
            sections: None,
            tile: SourceChunkTile {
                chunk_x: 4,
                chunk_z: 0,
                source_present: true,
                known_columns: 256,
                unknown_columns: 0,
                exposed_voxels: 0,
            },
            voxels: Vec::new(),
        });
        let snapshot = SceneSnapshot {
            visible_columns: None,
            display_columns: Default::default(),
            edit_refresh: false,
            geometry: None,
            seed: Default::default(),
            near: Arc::clone(&near),
            patches: vec![Arc::clone(&patch)],
            seam_cache: Arc::new(Mutex::new(SeamIndexCache::default())),
            ready: Arc::new(Mutex::new(None)),
        };
        let cloned = snapshot.clone();
        assert!(Arc::ptr_eq(&cloned.near, &near));
        assert!(Arc::ptr_eq(&cloned.patches[0], &patch));
        assert_eq!(snapshot.patch_count(), 1);
        let materialized = snapshot.materialize_for_test();
        assert_eq!(materialized.source_chunks, 1);
        assert_eq!(materialized.source_tiles.len(), 1);
        assert_eq!(near.source_chunks, 0);

        // 同一个源快照生命周期也覆盖候选会话切换：只重读改动与接缝，保留其他 Arc。
        let (mut app, scene, _, _) = crate::picking_integration_tests::fixture();
        let source = Arc::new(app.current_session((0, 0)).unwrap().clone());
        let mut stream =
            Stream::new((0, 0), Arc::clone(&source), [0.; 3], 8, 0., 1, false).unwrap();
        let source_voxel = scene.surface.voxels[0].clone();
        for (cx, cz, span) in [(0, 0, 1), (1, 1, 1), (2, 0, 1), (2, 0, 4)] {
            let mut voxel = source_voxel.clone();
            voxel.x = cx * 16;
            voxel.z = cz * 16;
            let patch = Arc::new(FarChunkPatch {
                biome_neighbors: Default::default(),
                biomes: Default::default(),
                sections: None,
                tile: SourceChunkTile {
                    chunk_x: cx,
                    chunk_z: cz,
                    source_present: true,
                    known_columns: 256,
                    unknown_columns: 0,
                    exposed_voxels: 1,
                },
                voxels: vec![voxel],
            });
            let cost = Arc::new(PatchCost::new(&patch));
            stream.install_patch((cx, cz, span), patch, cost);
        }
        let kept = Arc::clone(&stream.cache[&(2, 0, 1)]);
        let dropped = Arc::downgrade(&stream.cache[&(1, 1, 1)]);
        stream.verify_source_memory();
        // 旧工作线程结果不能在来源已替换后重新安装旧区块。
        stream.running = true;
        stream.batch_id = 7;
        stream.active_nonce = 1;
        stream.replace_source(source, &[(0, 0)].into_iter().collect());
        stream.receive(1, 7, Vec::new());
        assert_eq!(stream.cache.len(), 2);
        assert!(Arc::ptr_eq(&kept, &stream.cache[&(2, 0, 1)]));
        assert!(dropped.upgrade().is_none());
        assert!(stream.requires_replan([0.; 3], 8));
        stream.verify_source_memory();

        // 将同一源缓存交给真实应用流程：候选、确认、撤销、重做、放弃均保留远处源。
        let point = &scene.surface.voxels[0];
        let coordinate = (point.x.div_euclid(16), point.z.div_euclid(16));
        let target_patch = Arc::new(FarChunkPatch {
            biome_neighbors: Default::default(),
            biomes: Default::default(),
            sections: None,
            tile: SourceChunkTile {
                chunk_x: coordinate.0,
                chunk_z: coordinate.1,
                source_present: true,
                known_columns: 256,
                unknown_columns: 0,
                exposed_voxels: 1,
            },
            voxels: vec![point.clone()],
        });
        stream.install_patch(
            (coordinate.0, coordinate.1, 1),
            Arc::clone(&target_patch),
            Arc::new(PatchCost::new(&target_patch)),
        );
        app.spectator_stream = Some(stream);
        app.surface_exposed = true;
        app.edit_area = format!(
            "{},{},{};{},{},{}",
            point.x, point.y, point.z, point.x, point.y, point.z
        );
        app.edit_target = "numeric:0[data=0]".into();
        crate::block_edit::start(&mut app, crate::block_edit::Tool::Selection);
        crate::picking_integration_tests::finish_edit_preview(&mut app);
        let stream = app.spectator_stream.as_ref().expect("候选更新保留世界缓存");
        assert!(!stream.cache.contains_key(&(coordinate.0, coordinate.1, 1)));
        assert!(Arc::ptr_eq(&kept, &stream.cache[&(2, 0, 1)]));
        let task = crate::block_edit::apply(&mut app);
        let message = crate::picking_integration_tests::edit_task_output(task);
        let _ = crate::update(&mut app, message);
        assert_eq!(app.sessions[&(0, 0)].pending_cells(), 1);
        for message in [crate::Message::UndoEdit, crate::Message::RedoEdit] {
            let task = crate::update(&mut app, message);
            let message = crate::picking_integration_tests::edit_task_output(task);
            let _ = crate::update(&mut app, message);
            assert!(Arc::ptr_eq(
                &kept,
                &app.spectator_stream.as_ref().unwrap().cache[&(2, 0, 1)]
            ));
            app.spectator_stream
                .as_ref()
                .unwrap()
                .verify_source_memory();
        }
        let _ = crate::update(&mut app, crate::Message::DiscardEdits);
        assert_eq!(app.sessions[&(0, 0)].pending_cells(), 0);
        assert!(Arc::ptr_eq(
            &kept,
            &app.spectator_stream.as_ref().unwrap().cache[&(2, 0, 1)]
        ));
        app.spectator_stream
            .as_ref()
            .unwrap()
            .verify_source_memory();
    }

    #[test]
    fn full_chunks_replace_near_window_without_losing_outside_height() {
        use mcedit2_world::surface::{SourceChunkTile, VoxelMaterial, ALL_FACES};
        let world = mcedit2_world::inspect_world(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../test_files/AnvilWorld_1.8"),
        )
        .unwrap();
        let session =
            Arc::new(Session::new(world.clone(), world.dimensions[0].name.clone()).unwrap());
        let mut stream = Stream::new((0, 0), session, [0.0, 125.0, 0.0], 3, 0.0, 1, false).unwrap();
        assert!(stream.wanted.iter().all(|r| r.span == 1));
        let voxel = |y| {
            SurfaceVoxel::new(
                [0, y, 0],
                mcedit2_world::surface::VoxelData {
                    state: mcedit2_world::editing::parse_state("minecraft:stone")
                        .unwrap()
                        .into(),
                    biome: None,
                    original_block_entity: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    staged: false,
                    span: 1,
                    fluid: None,
                    light: Default::default(),
                },
            )
        };
        let tile = SourceChunkTile {
            chunk_x: 0,
            chunk_z: 0,
            source_present: true,
            known_columns: 256,
            unknown_columns: 0,
            exposed_voxels: 3,
        };
        let patch = Arc::new(FarChunkPatch {
            biome_neighbors: Default::default(),
            biomes: Default::default(),
            sections: None,
            tile: tile.clone(),
            voxels: vec![voxel(-64), voxel(125), voxel(320)],
        });
        stream.install_patch(
            (0, 0, 1),
            Arc::clone(&patch),
            Arc::new(PatchCost::new(&patch)),
        );
        stream.display_order.publish(FarChunkRequest {
            cx: 0,
            cz: 0,
            span: 1,
        });
        let covered = mcedit2_world::editing::Bounds {
            minimum: [0, -64, 0],
            maximum: [15, 320, 15],
        };
        assert!(stream.covers_exact(covered));
        assert!(!stream.covers_exact(mcedit2_world::editing::Bounds {
            minimum: [-1, -64, 0],
            ..covered
        }));
        let mut unknown = (*patch).clone();
        unknown.tile.unknown_columns = 1;
        let unknown = Arc::new(unknown);
        stream.install_patch(
            (0, 0, 1),
            Arc::clone(&unknown),
            Arc::new(PatchCost::new(&unknown)),
        );
        assert!(!stream.covers_exact(covered));
        stream.install_patch(
            (0, 0, 1),
            Arc::clone(&patch),
            Arc::new(PatchCost::new(&patch)),
        );
        let near = Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 101,
            y_max: 148,
            width: 16,
            depth: 16,
            voxels: vec![voxel(125)],
            missing_columns: 0,
            source_chunks: 1,
            source_tiles: vec![tile],
        });
        let result = stream.snapshot(near).materialize_for_test();
        assert_eq!(
            result.voxels.iter().map(|v| v.y).collect::<Vec<_>>(),
            [-64, 125, 320]
        );
        assert!(!stream.requires_replan([0.0, -64.0, 0.0], 3));
        let _ = stream.replan([0.0, 320.0, 0.0], 3, 0.0, 2);
        assert!(Arc::ptr_eq(&stream.cache[&(0, 0, 1)], &patch));
    }

    #[test]
    fn requested_lod_replaces_cached_near_geometry_after_camera_retreats() {
        let req = FarChunkRequest {
            cx: 29,
            cz: -7,
            span: 16,
        };
        let mut available = BTreeMap::new();
        available.insert((29, -7, 16), 4_usize);
        available.insert((29, -7, 4), 40);
        available.insert((29, -7, 1), 150);
        assert_eq!(
            best_cached_resolution(req, |key| available.get(&key).copied()),
            Some((29, -7, 16))
        );
        available.remove(&(29, -7, 16));
        assert_eq!(
            best_cached_resolution(req, |key| available.get(&key).copied()),
            Some((29, -7, 1))
        );
        available.remove(&(29, -7, 1));
        assert_eq!(
            best_cached_resolution(req, |key| available.get(&key).copied()),
            Some((29, -7, 4))
        );
        available.clear();
        assert_eq!(
            best_cached_resolution(req, |key| available.get(&key).copied()),
            None
        );
    }

    #[test]
    fn cached_sample_is_only_a_visual_bridge_until_exact_chunk_arrives() {
        let req = FarChunkRequest {
            cx: -12,
            cz: 33,
            span: 1,
        };
        let mut available = BTreeMap::from([((-12, 33, 16), 5_usize), ((-12, 33, 8), 15_usize)]);
        let select = |cache: &BTreeMap<Key, usize>| {
            best_cached_resolution(req, |key| cache.get(&key).copied())
        };
        assert_eq!(select(&available), Some((-12, 33, 8)));
        available.insert((-12, 33, 1), 80);
        assert_eq!(select(&available), Some((-12, 33, 1)));
        // Source chunk identity is exact, including negative coordinates.
        assert_eq!(
            best_cached_resolution(req, |key| { ((key.0, key.1) == (33, -12)).then_some(1) }),
            None
        );
    }

    #[test]
    fn first_batches_fill_exact_rings_before_sampled_horizon() {
        let requests = plan([0.0, 80.0, 0.0], 16).unwrap();
        let pending = prioritized_pending(&requests, (0, 0), 0);
        assert_eq!(pending.len(), requests.len());
        assert!(pending.iter().take(25).all(|r| r.span == 1));
        assert!(pending.iter().skip(25).take(24).all(|r| r.span == 2));
        let original: BTreeSet<_> = requests.iter().copied().map(key).collect();
        let scheduled: BTreeSet<_> = pending.iter().copied().map(key).collect();
        assert_eq!(scheduled, original);
    }

    #[test]
    fn display_order_is_append_only_and_independent_of_camera_facing() {
        let requests = plan([0.0, 72.0, 0.0], 12).unwrap();
        let mut a = requests.clone();
        let mut b = requests.clone();
        sort_facing(&mut a, (0, 0), look_sector(0.0));
        sort_facing(&mut b, (0, 0), look_sector(180.0));
        assert_ne!(a, b);
        let mut order = DisplayOrder::default();
        let first = requests[0];
        let second = requests[1];
        let third = requests[2];
        order.publish(second);
        order.publish(first);
        order.publish(second);
        assert_eq!(order.visible, vec![second, first]);
        order.replan(&b);
        assert_eq!(order.visible, vec![second, first]);
        order.publish(third);
        assert_eq!(order.visible, vec![second, first, third]);
        // Changing requested source resolution never moves the existing slot.
        let mut finer = requests.clone();
        let target = finer
            .iter_mut()
            .find(|r| (r.cx, r.cz) == (second.cx, second.cz))
            .unwrap();
        target.span = 1;
        order.replan(&finer);
        assert_eq!(order.visible[0].span, 1);
        assert_eq!(
            (order.visible[0].cx, order.visible[0].cz),
            (second.cx, second.cz)
        );
        order.replan(&[first, third]);
        assert_eq!(order.visible, vec![first, third]);
    }

    #[test]
    fn camera_facing_prioritizes_complete_chunk_rings_without_skipping_sources() {
        let planned = plan([0.0, 80.0, 0.0], 8).unwrap();
        let unique = planned.iter().copied().map(key).collect::<BTreeSet<_>>();
        for (yaw, ahead) in [
            (0.0, (0, 1)),
            (90.0, (-1, 0)),
            (180.0, (0, -1)),
            (270.0, (1, 0)),
        ] {
            let sector = look_sector(yaw);
            let mut sorted = planned.clone();
            sort_facing(&mut sorted, (0, 0), sector);
            assert_eq!(
                sorted.iter().copied().map(key).collect::<BTreeSet<_>>(),
                unique
            );
            assert_eq!((sorted[0].cx, sorted[0].cz), (0, 0));
            assert_eq!((sorted[1].cx, sorted[1].cz), ahead);
            let pending = prioritized_pending(&sorted, (0, 0), sector);
            assert_eq!(pending.len(), sorted.len());
            assert_eq!((pending[0].cx, pending[0].cz), (0, 0));
            assert_eq!((pending[1].cx, pending[1].cz), ahead);
        }
    }

    #[test]
    fn reorientation_never_reschedules_already_fetched_chunks() {
        let planned = plan([0.0, 80.0, 0.0], 8).unwrap();
        let mut pending = prioritized_pending(&planned, (0, 0), 0);
        let consumed = pending.drain(..32).map(key).collect::<BTreeSet<_>>();
        let expected = pending.iter().copied().map(key).collect::<BTreeSet<_>>();
        let reordered = prioritized_pending(&pending.into_iter().collect::<Vec<_>>(), (0, 0), 4);
        assert_eq!(
            reordered.iter().copied().map(key).collect::<BTreeSet<_>>(),
            expected
        );
        assert!(reordered.iter().all(|r| !consumed.contains(&key(*r))));
        let world = mcedit2_world::inspect_world(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../test_files/AnvilWorld_1.8"),
        )
        .unwrap();
        let session =
            Arc::new(Session::new(world.clone(), world.dimensions[0].name.clone()).unwrap());
        let mut stream = Stream::new((0, 0), session, [0., 80., 0.], 8, 0., 1, true).unwrap();
        stream.wanted = planned.clone();
        stream.wanted_keys = planned.iter().copied().map(key).collect();
        stream.read_keys = stream.wanted_keys.clone();
        stream.plan_ready = true;
        stream.pending = prioritized_pending(&planned, (0, 0), 0);
        let cached = stream.pending.pop_front().unwrap();
        // 两次相机中心分别是 (0, 0) 和 (4, 4)，该区块在两边都是完整区块。
        let shared = stream
            .pending
            .iter()
            .position(|r| (r.cx, r.cz) == (2, 2))
            .unwrap();
        let inflight = stream.pending.remove(shared).unwrap();
        let patch = |r: FarChunkRequest| {
            Arc::new(FarChunkPatch {
                biome_neighbors: Default::default(),
                biomes: Default::default(),
                sections: None,
                voxels: vec![],
                tile: mcedit2_world::surface::SourceChunkTile {
                    chunk_x: r.cx,
                    chunk_z: r.cz,
                    source_present: true,
                    known_columns: 256,
                    unknown_columns: 0,
                    exposed_voxels: 0,
                },
            })
        };
        let original = patch(cached);
        stream.install_patch(
            key(cached),
            Arc::clone(&original),
            Arc::new(PatchCost::new(&original)),
        );
        stream.display_order.publish(cached);
        stream.running = true;
        stream.inflight.insert(key(inflight));
        stream.dirty = false;
        let remaining = stream
            .pending
            .iter()
            .copied()
            .map(key)
            .collect::<BTreeSet<_>>();
        let cancellation = Arc::clone(&stream.cancellation);
        let wanted = stream.wanted.as_ptr();
        let _ = stream.orient(90.);
        let request = stream.active_request.clone().unwrap();
        assert!(request.pending.is_some());
        assert!(!cancellation.load(Ordering::Acquire));
        assert!(Arc::ptr_eq(&cancellation, &stream.cancellation));
        assert!(stream.running && !stream.batch_cancelled && !stream.dirty);
        assert_eq!(stream.wanted.as_ptr(), wanted);
        assert_eq!(stream.nonce, 1);
        let _ = stream.orient(180.);
        assert_eq!(stream.active_plan, Some((request.nonce, request.id)));
        assert!(stream.queued_plan.is_none());
        let result = request.prepare(&stream.plan_cancel);
        let _ = stream.receive_plan(request.nonce, request.id, result);
        let latest = stream.active_request.clone().unwrap();
        assert_eq!(latest.sector, look_sector(180.));
        let result = latest.prepare(&stream.plan_cancel);
        let _ = stream.receive_plan(latest.nonce, latest.id, result);
        assert!(stream.active_plan.is_none());
        assert_eq!(
            stream
                .pending
                .iter()
                .copied()
                .map(key)
                .collect::<BTreeSet<_>>(),
            remaining
        );
        assert!(stream.inflight.contains(&key(inflight)) && stream.running);
        assert!(stream.requires_replan([64., 80., 64.], 8));
        assert!(stream.requires_replan([64., 80., 64.], 12));
        assert!(Arc::ptr_eq(&stream.cache[&key(cached)], &original));
        assert!(!stream.dirty && stream.wanted.as_ptr() == wanted);
        // 受控排序故障不吞掉未读请求，也不取消真正的读盘批次。
        let _ = stream.orient(270.);
        let failed = stream.active_request.clone().unwrap();
        let _ = stream.receive_plan(failed.nonce, failed.id, Err("受控优先级排序故障".into()));
        assert_eq!(
            stream
                .pending
                .iter()
                .copied()
                .map(key)
                .collect::<BTreeSet<_>>(),
            remaining
        );
        assert!(!cancellation.load(Ordering::Acquire));
        assert!(!stream.dirty);
        // 真正完成后连续转向不产生计划、读盘或新网格世代。
        stream.pending.clear();
        let plan_id = stream.plan_id;
        for yaw in (0..1440).step_by(15) {
            let _ = stream.orient(yaw as f32);
        }
        assert_eq!(stream.plan_id, plan_id);
        assert!(stream.active_request.is_none() && !stream.dirty);
        assert!(Arc::ptr_eq(&stream.cache[&key(cached)], &original));
        // 飞行批次结束后更新范围；计划期间转头，最终计划使用最新方向。
        stream.running = false;
        assert!(stream.requires_replan([64., 80., 64.], 8));
        let _ = stream.replan([64., 80., 64.], 8, 0., 2);
        assert!(cancellation.load(Ordering::Acquire));
        let full = stream.active_request.clone().unwrap();
        assert!(full.pending.is_none());
        let _ = stream.orient(180.);
        let result = full.prepare(&stream.plan_cancel);
        let _ = stream.receive_plan(full.nonce, full.id, result);
        assert_eq!(stream.center, (4, 4));
        assert_eq!(stream.nonce, 2);
        let reordered = stream.active_request.clone().unwrap();
        assert!(reordered.pending.is_some() && reordered.sector == look_sector(180.));
        assert!(
            reordered
                .pending
                .as_ref()
                .unwrap()
                .iter()
                .any(|r| key(*r) == key(inflight)),
            "新范围必须重新排入上一批已取消的在途来源"
        );
        stream.receive(1, stream.batch_id, Vec::new());
        let result = reordered.prepare(&stream.plan_cancel);
        let _ = stream.receive_plan(reordered.nonce, reordered.id, result);
        let scheduled = stream
            .pending
            .iter()
            .copied()
            .map(key)
            .chain(stream.inflight.iter().copied())
            .collect::<BTreeSet<_>>();
        assert!(stream
            .wanted
            .iter()
            .all(|r| stream.cache.contains_key(&key(*r)) || scheduled.contains(&key(*r))));

        assert_eq!(look_sector(f32::NAN), 0);
        assert_eq!(look_sector(359.0), 0);
        assert_eq!(look_sector(90.0), 2);
    }

    #[test]
    fn cancelled_stale_batch_stops_at_next_chunk_boundary() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = Arc::clone(&cancelled);
        assert!(not_cancelled(&worker));
        cancelled.store(true, Ordering::Release);
        assert!(!not_cancelled(&worker));
    }

    #[test]
    fn cached_exact_source_is_never_discarded_due_to_its_voxel_count() {
        let physical = 8 * 1024 * 1024 * 1024;
        assert_eq!(
            memory_budgets_for(physical, None).unwrap().0,
            256 * 1024 * 1024
        );
        assert_eq!(memory_budgets_for(physical, Some(" 0 ")).unwrap().0, 0);
        assert_eq!(
            memory_budgets_for(physical, Some("4096")).unwrap().0,
            4096 * 1024 * 1024
        );
        for invalid in ["-1", "abc", "18446744073709551615"] {
            assert!(memory_budgets_for(physical, Some(invalid)).is_err());
        }
        let request = FarChunkRequest {
            cx: 30,
            cz: -19,
            span: 1,
        };
        assert_eq!(
            best_cached_resolution(request, |key| (key == (30, -19, 1)).then_some(usize::MAX)),
            Some((30, -19, 1))
        );
    }

    #[test]
    fn selected_radius_has_no_hidden_sixty_four_chunk_limit() {
        let radius = 128;
        let selected = plan([0.0, 70.0, 0.0], radius).unwrap();
        assert_eq!(selected.len(), (radius as usize * 2 + 1).pow(2));
        assert!(selected.iter().any(|req| req.cx == 128 && req.cz == 0));
    }

    #[test]
    fn exact_seams_cull_only_proven_full_opaque_neighbors() {
        use mcedit2_world::sections::State;
        let mk = |x, name: &str| {
            SurfaceVoxel::new(
                [x, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: name.into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: FACE_POS_X | FACE_NEG_X,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )
        };
        let mut view = SurfaceView {
            biomes: Default::default(),
            x: 15,
            z: 0,
            y_min: 64,
            y_max: 111,
            width: 32,
            depth: 16,
            voxels: vec![mk(15, "minecraft:stone"), mk(16, "minecraft:stone")],
            missing_columns: 0,
            source_chunks: 2,
            source_tiles: vec![],
        };
        reconcile_exact_seams(&mut view);
        assert_eq!(view.voxels[0].faces & FACE_POS_X, 0);
        assert_eq!(view.voxels[1].faces & FACE_NEG_X, 0);
        Arc::make_mut(&mut view.voxels[1].state).name = "minecraft:oak_stairs".into();
        view.voxels[0].faces = FACE_POS_X;
        reconcile_exact_seams(&mut view);
        assert_ne!(view.voxels[0].faces & FACE_POS_X, 0);
    }

    #[test]
    fn exact_chunk_seams_keep_faces_behind_ice_and_partial_blocks() {
        use mcedit2_world::sections::State;
        let mk = |x: i32, name: &str, props: &[(&str, &str)]| {
            SurfaceVoxel::new(
                [x, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: name.into(),
                        properties: props
                            .iter()
                            .map(|(key, value)| (key.to_string(), value.to_string()))
                            .collect(),
                    }
                    .into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: FACE_POS_X | FACE_NEG_X,
                    kind: if name.ends_with("ice") {
                        VoxelMaterial::Transparent
                    } else {
                        VoxelMaterial::Solid
                    },
                    light: Default::default(),
                },
            )
        };
        let mut view = SurfaceView {
            biomes: Default::default(),
            x: 15,
            z: 0,
            y_min: 64,
            y_max: 111,
            width: 32,
            depth: 16,
            voxels: vec![mk(15, "minecraft:stone", &[]), mk(16, "minecraft:ice", &[])],
            missing_columns: 0,
            source_chunks: 2,
            source_tiles: vec![],
        };
        reconcile_exact_seams(&mut view);
        assert_ne!(view.voxels[0].faces & FACE_POS_X, 0); // stone through ice
                                                          // The original source masks ice against full stone but keeps the
                                                          // stone face, which must remain visible through translucent ice.
        assert_eq!(view.voxels[1].faces & FACE_NEG_X, 0);
        view.voxels[1] = mk(16, "minecraft:stone_slab", &[("type", "bottom")]);
        reconcile_exact_seams(&mut view);
        assert_ne!(view.voxels[0].faces & FACE_POS_X, 0);
        view.voxels[1] = mk(16, "minecraft:stone_slab", &[("type", "double")]);
        reconcile_exact_seams(&mut view);
        assert_eq!(view.voxels[0].faces & FACE_POS_X, 0);
        assert_eq!(view.voxels[1].faces & FACE_NEG_X, 0);
        view.voxels[1] = mk(16, "minecraft:snow", &[("layers", "8")]);
        view.voxels[0].faces |= FACE_POS_X;
        reconcile_exact_seams(&mut view);
        assert_eq!(view.voxels[0].faces & FACE_POS_X, 0);
        view.voxels[1] = mk(16, "modded:opaque_machine", &[]);
        view.voxels[0].faces |= FACE_POS_X;
        reconcile_exact_seams(&mut view);
        assert_ne!(view.voxels[0].faces & FACE_POS_X, 0);
    }

    #[test]
    fn exact_chunk_seams_match_source_for_glass_leaves_and_liquid_boundaries() {
        use mcedit2_world::sections::State;
        let mk = |x, name: &str, kind| {
            SurfaceVoxel::new(
                [x, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: name.into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: FACE_NEG_X | FACE_POS_X,
                    kind,
                    light: Default::default(),
                },
            )
        };
        let opaque = VoxelMaterial::Solid;
        let trans = VoxelMaterial::Transparent;
        let foliage = VoxelMaterial::Foliage;
        let liquid = VoxelMaterial::Liquid;
        let evaluate = |left_name, left_kind, right_name, right_kind| {
            let mut view = SurfaceView {
                biomes: Default::default(),
                x: 15,
                z: 0,
                y_min: 64,
                y_max: 111,
                width: 32,
                depth: 16,
                voxels: vec![mk(15, left_name, left_kind), mk(16, right_name, right_kind)],
                missing_columns: 0,
                source_chunks: 2,
                source_tiles: vec![],
            };
            reconcile_exact_seams(&mut view);
            (
                view.voxels[0].faces & FACE_POS_X != 0,
                view.voxels[1].faces & FACE_NEG_X != 0,
            )
        };
        assert_eq!(
            evaluate("minecraft:glass", trans, "minecraft:glass", trans),
            (false, false)
        );
        assert_eq!(
            evaluate("minecraft:glass", trans, "minecraft:ice", trans),
            (true, true)
        );
        assert_eq!(
            evaluate(
                "minecraft:oak_leaves",
                foliage,
                "minecraft:oak_leaves",
                foliage
            ),
            (false, false)
        );
        assert_eq!(
            evaluate("minecraft:stone", opaque, "minecraft:ice", trans),
            (true, false)
        );
        assert_eq!(
            evaluate("minecraft:stone", opaque, "minecraft:glass", trans),
            (true, false)
        );
        assert_eq!(
            evaluate("minecraft:stone", opaque, "minecraft:water", liquid),
            // 缝合阶段保留液体候选，最终遮挡由实际液面与半格邻域处理。
            (true, true)
        );
        assert_eq!(
            evaluate("minecraft:water", liquid, "minecraft:lava", liquid),
            (true, true)
        );
        assert_eq!(
            evaluate("minecraft:water", liquid, "minecraft:water", liquid),
            (false, false)
        );
    }

    #[test]
    fn streamed_water_seams_hide_only_verified_internal_faces() {
        use mcedit2_world::sections::State;
        let fluid = |x, name: &str, level: &str| {
            SurfaceVoxel::new(
                [x, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: name.into(),
                        properties: vec![("level".into(), level.into())],
                    }
                    .into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: FACE_POS_X | FACE_NEG_X,
                    kind: VoxelMaterial::Liquid,
                    light: Default::default(),
                },
            )
        };
        let mut view = SurfaceView {
            biomes: Default::default(),
            x: 15,
            z: 0,
            y_min: 64,
            y_max: 111,
            width: 32,
            depth: 16,
            voxels: vec![
                fluid(15, "minecraft:water", "0"),
                fluid(16, "minecraft:water", "4"),
            ],
            missing_columns: 0,
            source_chunks: 2,
            source_tiles: vec![],
        };
        reconcile_exact_seams(&mut view);
        assert_eq!(view.voxels[0].faces & FACE_POS_X, 0);
        assert_eq!(view.voxels[1].faces & FACE_NEG_X, 0);
        Arc::make_mut(&mut view.voxels[1].state).name = "minecraft:lava".into();
        view.voxels[0].faces = FACE_POS_X;
        reconcile_exact_seams(&mut view);
        assert_ne!(view.voxels[0].faces & FACE_POS_X, 0);
        view.voxels[1].span = 8; // An LOD sample is not a real adjacent block.
        reconcile_exact_seams(&mut view);
        assert_ne!(view.voxels[0].faces & FACE_POS_X, 0);
    }

    #[test]
    fn complete_rings_schedule_each_source_chunk_once() {
        for radius in [1, 4, 8, 16, 32, 64] {
            let requests = plan([0.0, 80.0, 0.0], radius).unwrap();
            assert_eq!(requests.len(), (radius as usize * 2 + 1).pow(2));
            let keys: BTreeSet<_> = requests.iter().map(|r| (r.cx, r.cz)).collect();
            assert_eq!(keys.len(), requests.len());
            assert!(requests.iter().any(|r| r.cx == radius as i32 && r.cz == 0));
        }
    }

    #[test]
    fn signed_origin_floor_and_lod_are_deterministic() {
        let requests = plan([-0.1, 80.0, -16.1], 24).unwrap();
        assert_eq!((requests[0].cx, requests[0].cz), (-1, -2));
        assert_eq!(requests[0].span, 1);
        assert!(requests.iter().any(|r| r.span == 4));
        assert!(requests.iter().any(|r| r.span == 8));
        assert!(!requests.iter().any(|r| r.span == 16));
        assert!(plan([0.0, 0.0, 0.0], 0).is_err());
    }

    #[test]
    fn world_border_does_not_overflow_or_synthesize_chunks() {
        let requests = plan([29_999_992.0, 40.0, -29_999_992.0], 32).unwrap();
        assert!(requests.iter().all(|r| {
            let x = r.cx as i64 * 16;
            let z = r.cz as i64 * 16;
            x >= -30_000_000 && x + 15 <= 30_000_000 && z >= -30_000_000 && z + 15 <= 30_000_000
        }));
    }

    #[test]
    fn decoded_sources_reuse_verified_images_and_stream_while_flying() {
        for version in [1343, 3465, 3955] {
            let mut fixture = crate::metadata_fixture::Fixture::new(version);
            crate::object_fixture::populate(&mut fixture);
            let source = Arc::new(
                Session::new(
                    fixture.world.clone(),
                    fixture.world.dimensions[0].name.clone(),
                )
                .unwrap(),
            );
            let first = source.shared_source_reader(64 * 1024 * 1024).unwrap();
            let decoded = first.read_chunk(0, 0).unwrap().unwrap();
            let mut next = source.shared_source_reader(64 * 1024 * 1024).unwrap();
            next.reuse_decoded_from(&first).unwrap();
            assert!(Arc::ptr_eq(
                &decoded,
                &next.read_chunk(0, 0).unwrap().unwrap()
            ));
            // 磁盘镜像改变后不复用旧解码；损坏当前源也不能被旧缓存掩盖。
            let path = fixture.root.join("region/r.0.0.mca");
            let original = std::fs::read(&path).unwrap();
            let mut changed = original.clone();
            changed[4096] ^= 1;
            std::fs::write(&path, &changed).unwrap();
            let mut current = source.shared_source_reader(64 * 1024 * 1024).unwrap();
            current.reuse_decoded_from(&next).unwrap();
            let reread = current.read_chunk(0, 0).unwrap().unwrap();
            assert!(!Arc::ptr_eq(&decoded, &reread));
            assert_eq!(decoded.block(0, 0, 0), reread.block(0, 0, 0));
            changed[8196] = 127;
            std::fs::write(&path, &changed).unwrap();
            let mut corrupt = source.shared_source_reader(64 * 1024 * 1024).unwrap();
            corrupt.reuse_decoded_from(&current).unwrap();
            assert!(corrupt.read_chunk(0, 0).is_err());
            std::fs::write(&path, &original).unwrap();
            let mut stream = Stream::new((0, 0), source, [0., 64., 0.], 2, 0., 1, false).unwrap();
            let _ = stream.replan([0., 64., 0.], 2, 0., 1);
            let request = stream.active_request.clone().unwrap();
            let task = stream.receive_plan(
                request.nonce,
                request.id,
                request.prepare(&stream.plan_cancel),
            );
            let requested = stream.inflight.len();
            let outputs = crate::picking_integration_tests::task_outputs(task);
            assert!(stream.running);
            assert!(stream.requires_replan([16., 64., 0.], 2));
            let cancel = Arc::clone(&stream.cancellation);
            let _ = stream.replan([16., 64., 0.], 2, 0., 2);
            assert!(!cancel.load(Ordering::Acquire));
            let request = stream.active_request.clone().unwrap();
            let _ = stream.receive_plan(
                request.nonce,
                request.id,
                request.prepare(&stream.plan_cancel),
            );
            assert_eq!(
                outputs.len(),
                requested.div_ceil(4),
                "按四个结果成组发布，不能逐块刷新或等待完整读取批次"
            );
            for message in outputs {
                match message {
                    Message::SpectatorStreamBatch(nonce, id, results) => {
                        stream.receive_delivery(nonce, id, results);
                    }
                    _ => panic!("实际读取任务只能发布来源数据与完成通知"),
                }
            }
            assert!(!stream.running);
            assert!(!stream.cache.is_empty());
            assert!(stream.complete <= stream.total);
            assert!(stream
                .cache
                .keys()
                .all(|key| stream.read_keys.contains(key)));
            assert!(stream
                .display_order
                .visible
                .iter()
                .all(|request| stream.wanted_keys.contains(&key(*request))));
            stream.verify_source_memory();
            while !stream.pending.is_empty() {
                for message in crate::picking_integration_tests::task_outputs(stream.fetch()) {
                    match message {
                        Message::SpectatorStreamBatch(nonce, id, results) => {
                            stream.receive_delivery(nonce, id, results);
                        }
                        _ => panic!("读取任务只发布实际批次结果"),
                    }
                }
            }
            stream.dirty = false;
            let _ = stream.replan([32., 64., 0.], 2, 0., 3);
            let request = stream.active_request.clone().unwrap();
            let task = stream.receive_plan(
                request.nonce,
                request.id,
                request.prepare(&stream.plan_cancel),
            );
            assert!(stream.running);
            if stream.complete < stream.total {
                assert!(!stream.dirty, "尚未读完的新范围不能只因裁边触发 GPU 交接");
            } else {
                assert!(stream.dirty, "已经预读的新范围必须立即建模");
            }
            for message in crate::picking_integration_tests::task_outputs(task) {
                match message {
                    Message::SpectatorStreamBatch(nonce, id, results) => {
                        stream.receive_delivery(nonce, id, results);
                    }
                    _ => panic!("读取任务只发布实际批次结果"),
                }
            }
            assert!(stream.dirty, "收到新范围的实际批次后必须发布新场景");
            stream.verify_source_memory();
            assert_eq!(std::fs::read(path).unwrap(), original);
        }
    }

    #[test]
    fn lookahead_reads_real_columns_without_expanding_the_visible_plan() {
        fn drain(stream: &mut Stream, mut task: Task<Message>) {
            loop {
                for message in crate::picking_integration_tests::task_outputs(task) {
                    match message {
                        Message::SpectatorStreamBatch(nonce, id, delivery) => {
                            stream.receive_delivery(nonce, id, delivery);
                        }
                        _ => panic!("预读只允许交付真实来源批次"),
                    }
                }
                if stream.pending.is_empty() {
                    assert!(!stream.running);
                    break;
                }
                task = stream.fetch();
            }
        }
        let mut fixture = crate::metadata_fixture::Fixture::new(3955);
        crate::object_fixture::populate(&mut fixture);
        let source = Arc::new(
            Session::new(
                fixture.world.clone(),
                fixture.world.dimensions[0].name.clone(),
            )
            .unwrap(),
        );
        for budget in [usize::MAX, 0] {
            let mut stream =
                Stream::new((0, 0), Arc::clone(&source), [0., 64., 0.], 2, 0., 1, false).unwrap();
            stream.cache_budget = budget;
            let _ = stream.replan([0., 64., 0.], 2, 0., 1);
            let request = stream.active_request.clone().unwrap();
            let task = stream.receive_plan(
                request.nonce,
                request.id,
                request.prepare(&stream.plan_cancel),
            );
            drain(&mut stream, task);
            assert_eq!((stream.complete, stream.total), (25, 25));
            assert_eq!(stream.display_order.visible.len(), 25);
            assert!(!stream.display_order.positions.contains_key(&(3, 0)));
            assert!(stream
                .display_order
                .visible
                .iter()
                .all(|r| r.cx.abs().max(r.cz.abs()) <= 2));
            if budget == 0 {
                assert!(stream
                    .cache
                    .keys()
                    .all(|id| stream.wanted_keys.contains(id)));
                continue;
            }
            let ahead = Arc::clone(&stream.cache[&(3, 0, 1)]);
            assert_eq!(stream.cache.len(), 81);
            let display = Arc::clone(&stream.display_columns);
            let frozen_columns = display.lock().unwrap().clone().unwrap();
            assert_eq!(frozen_columns.len(), 25);
            assert!(!frozen_columns.contains(&(3, 0)));
            stream.dirty = false;
            let _ = stream.replan([16., 64., 0.], 2, 0., 2);
            let request = stream.active_request.clone().unwrap();
            let task = stream.receive_plan(
                request.nonce,
                request.id,
                request.prepare(&stream.plan_cancel),
            );
            assert_eq!((stream.complete, stream.total), (25, 25));
            assert!(stream.dirty);
            assert!(stream.display_order.positions.contains_key(&(3, 0)));
            assert!(Arc::ptr_eq(&ahead, &stream.cache[&(3, 0, 1)]));
            assert!(
                display.lock().unwrap().as_ref().unwrap().contains(&(3, 0)),
                "显示范围须先于新一轮建模更新"
            );
            assert!(
                !frozen_columns.contains(&(3, 0)),
                "旧来源的可见性证明仍保持不可变"
            );
            assert!(stream
                .display_order
                .visible
                .iter()
                .all(|r| (r.cx - 1).abs().max(r.cz.abs()) <= 2));
            drain(&mut stream, task);
            stream.verify_source_memory();
        }
    }

    #[test]
    fn test_crate_stream_readers_verify_delivery_coalescing() {
        crate::stream_readers::verify_delivery_coalescing();
    }
}
