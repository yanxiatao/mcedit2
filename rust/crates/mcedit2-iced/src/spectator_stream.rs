//! Continuous Minecraft spectator terrain streaming: complete chunk rings,
//! progressive source LOD, bounded reuse, no guessed absent source chunks.
//! This is display-only: sampled voxels must never enter edit/selection APIs.
use crate::Message;
use iced::Task;
use mcedit2_world::{
    editing::Session,
    far_view::{self, FarChunkPatch, FarChunkRequest},
    surface::{
        exact_face_exposed_against, SurfaceView, VoxelMaterial, FACE_BOTTOM,
        FACE_NEG_X, FACE_NEG_Z, FACE_POS_X, FACE_POS_Z, FACE_TOP,
    },
    WorldError,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub(crate) const DEFAULT_RADIUS: u16 = 32;
const MAX_READ_BATCH: usize = 32;
// Start by sampling one cheap distant ring before the costly nearby full
// voxel scans. The next batches return to the exact ring, preserving detail.
const HORIZON_WARMUP: usize = 32;
type Key = (i32, i32, u8);
fn key(request: FarChunkRequest) -> Key {
    (request.cx, request.cz, request.span)
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
        (0, 1), (-1, 1), (-1, 0), (-1, -1),
        (0, -1), (1, -1), (1, 0), (1, 1),
    ];
    VECTORS[sector as usize % VECTORS.len()]
}
/// Sort strictly by distance ring, then prefer chunks in front of the
/// spectator. Stable integer scoring avoids float ordering anomalies.
fn sort_facing(
    requests: &mut [FarChunkRequest],
    center: (i32, i32),
    sector: u8,
) {
    let (fx, fz) = front_vector(sector);
    requests.sort_by_key(|request| {
        let dx = i64::from(request.cx) - i64::from(center.0);
        let dz = i64::from(request.cz) - i64::from(center.1);
        (
            dx.abs().max(dz.abs()),
            -(dx * fx + dz * fz),
            (dx * fz - dz * fx).abs(),
            request.cx,
            request.cz,
        )
    });
}
/// Every source chunk remains queued, including behind the camera.
/// Only the first distant warmup and the subsequent exact/LOD priority
/// change; this source data never enters editing operations.
fn prioritized_pending(
    requests: &[FarChunkRequest],
    cached: &BTreeMap<Key, Arc<FarChunkPatch>>,
    center: (i32, i32),
    sector: u8,
) -> VecDeque<FarChunkRequest> {
    let mut exact = VecDeque::new();
    let mut distant = VecDeque::new();
    for request in requests.iter().copied() {
        if cached.contains_key(&key(request)) {
            continue;
        }
        if request.span == 1 {
            exact.push_back(request);
        } else {
            distant.push_back(request);
        }
    }
    let mut exact = exact.into_iter().collect::<Vec<_>>();
    let mut distant = distant.into_iter().collect::<Vec<_>>();
    sort_facing(&mut exact, center, sector);
    sort_facing(&mut distant, center, sector);
    let mut exact = VecDeque::from(exact);
    let mut distant = VecDeque::from(distant);
    let mut pending = VecDeque::with_capacity(exact.len() + distant.len());
    // Include only the nearest LOD samples in the initial warmup. We never
    // skip or synthesize a source chunk: all remaining requests follow.
    for _ in 0..HORIZON_WARMUP {
        let Some(request) = distant.pop_front() else {
            break;
        };
        pending.push_back(request);
    }
    pending.append(&mut exact);
    pending.append(&mut distant);
    pending
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
        let index: HashMap<(i32, i32), FarChunkRequest> = requested.iter()
            .copied().map(|req| ((req.cx, req.cz), req)).collect();
        self.visible.retain_mut(|req| {
            if let Some(new) = index.get(&(req.cx, req.cz)) {
                *req = *new;
                true
            } else {
                false
            }
        });
        self.positions = self.visible.iter().enumerate()
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
    for span in [1_u8, 4, 8, 16] {
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
fn reconcile_exact_seams(view: &mut SurfaceView) {
    let hidden: Vec<u8> = {
        // Face-neighbor lookup is independent of coordinate order. Hashing
        // avoids six O(log N) BTreeMap probes per exact source voxel.
        let occupied: HashMap<(i32, i32, i32), (&mcedit2_world::sections::State, VoxelMaterial)> =
            view.voxels
                .iter()
                .filter(|v| v.span == 1)
                .map(|v| ((v.x, v.y, v.z), (&v.state, v.kind)))
                .collect();
        view.voxels
            .iter()
            .map(|v| {
                if v.span != 1 {
                    return 0;
                }
                let mut mask = 0;
                for (dx, dy, dz, face) in [
                    (0, 1, 0, FACE_TOP),
                    (0, -1, 0, FACE_BOTTOM),
                    (1, 0, 0, FACE_POS_X),
                    (-1, 0, 0, FACE_NEG_X),
                    (0, 0, 1, FACE_POS_Z),
                    (0, 0, -1, FACE_NEG_Z),
                ] {
                    if v.faces & face == 0 {
                        continue;
                    }
                    let (Some(x), Some(y), Some(z)) = (
                        v.x.checked_add(dx),
                        v.y.checked_add(dy),
                        v.z.checked_add(dz),
                    ) else {
                        continue;
                    };
                    if let Some(&(neighbor, kind)) = occupied.get(&(x, y, z)) {
                        if !exact_face_exposed_against(&v.state, v.kind, Some((neighbor, kind))) {
                            mask |= face;
                        }
                    }
                }
                mask
            })
            .collect()
    };
    for (voxel, mask) in view.voxels.iter_mut().zip(hidden) {
        voxel.faces &= !mask;
    }
}
/// Immutable incremental scene publication: the near view is shared and
/// unchanged distant chunks are Arc-referenced, not copied on the UI thread.
/// A mesh worker materializes an exact face-seam snapshot only when it needs
/// to rebuild geometry, leaving the previous complete GPU frame visible.
#[derive(Clone)]
pub(crate) struct SceneSnapshot {
    near: Arc<SurfaceView>,
    patches: Vec<Arc<FarChunkPatch>>,
}
impl SceneSnapshot {
    pub(crate) fn materialize(&self) -> SurfaceView {
        let mut result = (*self.near).clone();
        for patch in &self.patches {
            result.voxels.extend(patch.voxels.iter().cloned());
            result.source_tiles.push(patch.tile.clone());
            result.source_chunks += usize::from(patch.tile.source_present);
            result.missing_columns += patch.tile.unknown_columns;
        }
        reconcile_exact_seams(&mut result);
        result
    }
    #[cfg(test)]
    fn patch_count(&self) -> usize { self.patches.len() }
}
pub(crate) struct Stream {
    pub key: (usize, usize),
    pub source: Arc<Session>,
    pub center: (i32, i32),
    pub radius: u16,
    look_sector: u8,
    /// Display source chunks in publication order, not re-sorted with yaw.
    display_order: DisplayOrder,
    pub exact_y_min: i32,
    pub nonce: u64,
    pub running: bool,
    cancellation: Arc<AtomicBool>,
    pub complete: usize,
    published: usize,
    pub errors: usize,
    pub total: usize,
    /// Each source chunk has at most one active LOD in this view.
    wanted: Vec<FarChunkRequest>,
    pending: VecDeque<FarChunkRequest>,
    cache: BTreeMap<Key, Arc<FarChunkPatch>>,
    cached_voxels: usize,
    pub dirty: bool,
}
impl Stream {
    fn height_window(source: &Session, location: [f64; 3]) -> i32 {
        let (min, max) = if source
            .original_world()
            .data_version
            .is_some_and(|v| v >= 2860)
        {
            (-64, 272)
        } else {
            (0, 208)
        };
        (location[1].floor() as i32)
            .saturating_sub(24)
            .clamp(min, max)
    }
    pub fn new(
        key: (usize, usize),
        source: Arc<Session>,
        location: [f64; 3],
        radius: u16,
        yaw: f32,
        nonce: u64,
    ) -> Result<Self, String> {
        let exact_y_min = Self::height_window(&source, location);
        let mut state = Self {
            key,
            source,
            center: center_chunk(location),
            radius,
            look_sector: look_sector(yaw),
            display_order: DisplayOrder::default(),
            exact_y_min,
            nonce,
            running: false,
            cancellation: Arc::new(AtomicBool::new(false)),
            complete: 0,
            published: 0,
            errors: 0,
            total: 0,
            wanted: Vec::new(),
            pending: VecDeque::new(),
            cache: BTreeMap::new(),
            cached_voxels: 0,
            dirty: true,
        };
        state.replan(location, radius, yaw, nonce)?;
        Ok(state)
    }
    /// Camera navigation never mutates world state. A pending old batch
    /// may finish; the UI nonce check discards it before publishing.
    pub fn replan(
        &mut self,
        location: [f64; 3],
        radius: u16,
        yaw: f32,
        nonce: u64,
    ) -> Result<(), String> {
        let mut requests = plan(location, radius)?;
        let center = center_chunk(location);
        // Retain the draw offset of every still-requested source chunk.
        // New chunks get their slot only after their read successfully ends.
        self.display_order.replan(&requests);
        self.look_sector = look_sector(yaw);
        sort_facing(&mut requests, center, self.look_sector);
        let exact_y_min = Self::height_window(&self.source, location);
        let changed_height = self.exact_y_min != exact_y_min;
        let needed: HashSet<_> = requests.iter()
            .map(|req| (req.cx, req.cz)).collect();
        // Keep verified coarser patches while a finer source request reads;
        // otherwise moving toward a chunk temporarily erases it. Exact
        // vertical windows must still be reread after a vertical shift.
        self.cache.retain(|id, _| needed.contains(&(id.0, id.1))
            && !(changed_height && id.2 == 1));
        self.exact_y_min = exact_y_min;
        self.cached_voxels = self.cache.values().map(|p| p.voxels.len()).sum();
        self.complete = requests
            .iter()
            .filter(|r| self.cache.contains_key(&key(**r)))
            .count();
        self.total = requests.len();
        self.published = self.complete;
        self.pending = prioritized_pending(&requests, &self.cache, center, self.look_sector);
        self.center = center;
        self.radius = radius;
        self.nonce = nonce;
        // Invalidate old queued batches before a fresh one starts. The
        // in-progress source chunk can finish, but stale jobs stop before
        // their next expensive chunk read; nonce checks remain authoritative.
        self.cancellation.store(true, Ordering::Release);
        self.cancellation = Arc::new(AtomicBool::new(false));
        self.running = false;
        self.wanted = requests;
        self.errors = 0;
        self.dirty = true;
        Ok(())
    }
    pub fn requires_replan(&self, location: [f64; 3], radius: u16) -> bool {
        let target = center_chunk(location);
        radius != self.radius
            || (target.0 - self.center.0).abs() >= 2
            || (target.1 - self.center.1).abs() >= 2
            || (Self::height_window(&self.source, location) - self.exact_y_min).abs() >= 16
    }
    /// Only rotate queued requests on eight yaw sectors, preserving any
    /// active read and its nonce. Reorder displayed budget by facing too.
    pub fn orient(&mut self, yaw: f32) {
        let sector = look_sector(yaw);
        if sector == self.look_sector {
            return;
        }
        self.look_sector = sector;
        sort_facing(&mut self.wanted, self.center, sector);
        let unread = self.pending.drain(..).collect::<Vec<_>>();
        self.pending = prioritized_pending(&unread, &self.cache, self.center, sector);
        // Geometry has not changed. A new camera orientation updates the GPU
        // camera uniform and transparent ordering, not the source mesh.
    }
    pub fn fetch(&mut self) -> Task<Message> {
        if self.running || self.pending.is_empty() {
            return Task::none();
        }
        // Do not mix full voxel reads with cheap LOD samples in one worker
        // batch. The first distant warmup can publish without waiting for a
        // slower exact chunk in the same batch.
        let span = self.pending.front().unwrap().span;
        let count = self
            .pending
            .iter()
            .take_while(|request| request.span == span)
            .count()
            .min(far_view::batch_size_for(span).min(MAX_READ_BATCH));
        let requests: Vec<_> = self.pending.drain(..count).collect();
        let fallback = requests.clone();
        let (nonce, source) = (self.nonce, Arc::clone(&self.source));
        let exact_y_min = self.exact_y_min;
        let cancellation = Arc::clone(&self.cancellation);
        self.running = true;
        // Each request has a validated original Region/chunk coordinate,
        // shared reader index per OS worker and bounded batch result.
        Task::perform(
            async move {
                let (send, recv) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    if !not_cancelled(&cancellation) {
                        let _ = send.send(Vec::new());
                        return;
                    }
                    let mut region = mcedit2_world::block_lookup::ReadOnlyRegionCache::new(
                        source.original_world(),
                        source.dimension(),
                    );
                    let (ymin, ymax) = if source
                        .original_world()
                        .data_version
                        .is_some_and(|v| v >= 2860)
                    {
                        (-64, 319)
                    } else {
                        (0, 255)
                    };
                    let mut patches = Vec::with_capacity(requests.len());
                    for req in requests {
                        if !not_cancelled(&cancellation) {
                            break;
                        }
                        let result = if req.span == 1 {
                            // Nearby full source voxel geometry: caves, cliffs,
                            // glass, stairs and resource JSON models. The editor
                            // source path never receives an approximate LOD.
                            let x = req.cx.checked_mul(16);
                            let z = req.cz.checked_mul(16);
                            match (x, z) {
                                (Some(x), Some(z)) => {
                                    mcedit2_world::surface::inspect_exposed_volume(
                                        &source,
                                        x,
                                        z,
                                        exact_y_min,
                                        exact_y_min + 47,
                                        16,
                                        16,
                                    )
                                    .map(|surface| FarChunkPatch {
                                        tile: surface
                                            .source_tiles
                                            .into_iter()
                                            .next()
                                            .expect("Exact source chunk has one tile"),
                                        voxels: surface.voxels,
                                    })
                                    .map_err(|e| e.to_string())
                                }
                                _ => Err("Exact source chunk coordinate overflow".into()),
                            }
                        } else {
                            match &mut region {
                                Ok(reader) => far_view::inspect_far_chunk_cached(
                                    &source, reader, req, ymin, ymax,
                                )
                                .map_err(|e: WorldError| e.to_string()),
                                Err(e) => Err(e.to_string()),
                            }
                        };
                        patches.push((req, result));
                    }
                    let _ = send.send(patches);
                });
                let results = recv.await.unwrap_or_else(|_| {
                    fallback
                        .into_iter()
                        .map(|r| (r, Err("Original Region streaming worker stopped".into())))
                        .collect()
                });
                (nonce, results)
            },
            |(nonce, results)| Message::SpectatorStreamBatch(nonce, results),
        )
    }
    pub fn receive(
        &mut self,
        nonce: u64,
        results: Vec<(FarChunkRequest, Result<FarChunkPatch, String>)>,
    ) {
        if nonce != self.nonce {
            return;
        }
        self.running = false;
        for (req, result) in results {
            self.complete += 1;
            match result {
                Ok(patch) => {
                    let id = key(req);
                    let n = patch.voxels.len();
                    // Do not drop valid source chunks because an arbitrary
                    // fixed cache size was reached. The requested render
                    // radius is already the ownership and eviction boundary.
                    if let Some(previous) = self.cache.insert(id, Arc::new(patch)) {
                        self.cached_voxels = self.cached_voxels.saturating_sub(previous.voxels.len());
                    }
                    self.cached_voxels = self.cached_voxels.saturating_add(n);
                    self.display_order.publish(req);
                }
                Err(_) => self.errors += 1,
            }
        }
        // Publish near-field updates promptly, but do not wait for 128
        // chunks before exposing the first distant tiles. A large batch
        // commonly contains the first visible horizon after an empty ring.
        if self.complete <= 64
            || self.complete.saturating_sub(self.published) >= 32
            || self.pending.is_empty()
        {
            self.dirty = true;
            self.published = self.complete;
        }
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
            if overlaps(*req) { continue; }
            let Some(best) = best_cached_resolution(*req,
                |id| self.cache.get(&id).map(|patch| patch.voxels.len())) else {
                continue;
            };
            if let Some(patch) = self.cache.get(&best) {
                patches.push(Arc::clone(patch));
            }
        }
        SceneSnapshot { near, patches }
    }
    #[cfg(test)]
    pub fn combined(&self, near: &SurfaceView) -> SurfaceView {
        self.snapshot(Arc::new(near.clone())).materialize()
    }
    pub fn status(&self) -> String {
        format!("Streaming real Java terrain: {}/{} chunks, {} errors, {} cached (exact caves/blocks nearby, textured LOD farther), radius {} chunks.",
            self.complete,self.total,self.errors,self.cache.len(),self.radius)
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        // A world switch should also stop old detached reader threads.
        self.cancellation.store(true, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mcedit2_world::surface::SurfaceVoxel;
    #[test]
    fn immutable_chunk_snapshot_reuses_patch_arcs_and_materializes_on_demand() {
        use mcedit2_world::surface::{SurfaceView, SourceChunkTile};
        let near = Arc::new(SurfaceView {
            x: 0, z: 0, y_min: 0, y_max: 47, width: 16, depth: 16,
            voxels: Vec::new(), source_tiles: Vec::new(),
            source_chunks: 0, missing_columns: 0,
        });
        let patch = Arc::new(FarChunkPatch {
            tile: SourceChunkTile {
                chunk_x: 4, chunk_z: 0, source_present: true,
                known_columns: 256, unknown_columns: 0, exposed_voxels: 0,
            },
            voxels: Vec::new(),
        });
        let snapshot = SceneSnapshot {
            near: Arc::clone(&near),
            patches: vec![Arc::clone(&patch)],
        };
        let cloned = snapshot.clone();
        assert!(Arc::ptr_eq(&cloned.near, &near));
        assert!(Arc::ptr_eq(&cloned.patches[0], &patch));
        assert_eq!(snapshot.patch_count(), 1);
        let materialized = snapshot.materialize();
        assert_eq!(materialized.source_chunks, 1);
        assert_eq!(materialized.source_tiles.len(), 1);
        assert_eq!(near.source_chunks, 0);
    }
    #[test]
    fn exact_cached_chunk_wins_over_requested_coarse_lod() {
        let req = FarChunkRequest { cx: 29, cz: -7, span: 16 };
        let mut available = BTreeMap::new();
        available.insert((29, -7, 16), 4_usize);
        available.insert((29, -7, 4), 40);
        available.insert((29, -7, 1), 150);
        assert_eq!(best_cached_resolution(req, |key| available.get(&key).copied()),
            Some((29, -7, 1)));
        available.remove(&(29, -7, 1));
        assert_eq!(best_cached_resolution(req, |key| available.get(&key).copied()),
            Some((29, -7, 4)));
        available.remove(&(29, -7, 4));
        assert_eq!(best_cached_resolution(req, |key| available.get(&key).copied()),
            Some((29, -7, 16)));
        available.clear();
        assert_eq!(best_cached_resolution(req, |key| available.get(&key).copied()), None);
    }
    #[test]
    fn cached_sample_is_only_a_visual_bridge_until_exact_chunk_arrives() {
        let req = FarChunkRequest { cx: -12, cz: 33, span: 1 };
        let mut available = BTreeMap::from([
            ((-12, 33, 16), 5_usize),
            ((-12, 33, 8), 15_usize),
        ]);
        let select = |cache: &BTreeMap<Key, usize>| best_cached_resolution(
            req, |key| cache.get(&key).copied(),
        );
        assert_eq!(select(&available), Some((-12, 33, 8)));
        available.insert((-12, 33, 1), 80);
        assert_eq!(select(&available), Some((-12, 33, 1)));
        // Source chunk identity is exact, including negative coordinates.
        assert_eq!(best_cached_resolution(req, |key| {
            ((key.0, key.1) == (33, -12)).then_some(1)
        }), None);
    }
    #[test]
    fn first_batch_previews_horizon_then_restores_nearby_detail() {
        let requests = plan([0.0, 80.0, 0.0], 16).unwrap();
        let pending = prioritized_pending(&requests, &BTreeMap::new(), (0, 0), 0);
        assert_eq!(pending.len(), requests.len());
        let first: Vec<_> = pending.iter().take(HORIZON_WARMUP).copied().collect();
        assert!(first.iter().all(|r| r.span == 4));
        assert!(pending
            .iter()
            .skip(HORIZON_WARMUP)
            .take(25)
            .all(|r| r.span == 1));
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
        let target = finer.iter_mut().find(|r|
            (r.cx, r.cz) == (second.cx, second.cz)).unwrap();
        target.span = 1;
        order.replan(&finer);
        assert_eq!(order.visible[0].span, 1);
        assert_eq!((order.visible[0].cx, order.visible[0].cz),
            (second.cx, second.cz));
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
            assert_eq!(sorted.iter().copied().map(key).collect::<BTreeSet<_>>(), unique);
            assert_eq!((sorted[0].cx, sorted[0].cz), (0, 0));
            assert_eq!((sorted[1].cx, sorted[1].cz), ahead);
            let pending = prioritized_pending(&sorted, &BTreeMap::new(), (0, 0), sector);
            assert_eq!(pending.len(), sorted.len());
            assert!(pending.iter().take(HORIZON_WARMUP).all(|r| r.span == 4));
            let first = pending.front().unwrap();
            let (fx, fz) = front_vector(sector);
            assert!(i64::from(first.cx) * fx + i64::from(first.cz) * fz > 0);
        }
    }
    #[test]
    fn reorientation_never_reschedules_already_fetched_chunks() {
        let planned = plan([0.0, 80.0, 0.0], 8).unwrap();
        let mut pending = prioritized_pending(&planned, &BTreeMap::new(), (0, 0), 0);
        let consumed = pending.drain(..32).map(key).collect::<BTreeSet<_>>();
        let expected = pending.iter().copied().map(key).collect::<BTreeSet<_>>();
        let reordered = prioritized_pending(
            &pending.into_iter().collect::<Vec<_>>(),
            &BTreeMap::new(),
            (0, 0),
            4,
        );
        assert_eq!(reordered.iter().copied().map(key).collect::<BTreeSet<_>>(), expected);
        assert!(reordered.iter().all(|r| !consumed.contains(&key(*r))));
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
        let request = FarChunkRequest { cx: 30, cz: -19, span: 1 };
        assert_eq!(best_cached_resolution(request, |key|
            (key == (30, -19, 1)).then_some(usize::MAX)),
            Some((30, -19, 1)));
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
        let mk = |x, name: &str| SurfaceVoxel {
            x,
            y: 70,
            z: 0,
            span: 1,
            state: State {
                name: name.into(),
                properties: vec![],
            },
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: FACE_POS_X | FACE_NEG_X,
            kind: VoxelMaterial::Solid,
        };
        let mut view = SurfaceView {
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
        view.voxels[1].state.name = "minecraft:oak_stairs".into();
        view.voxels[0].faces = FACE_POS_X;
        reconcile_exact_seams(&mut view);
        assert_ne!(view.voxels[0].faces & FACE_POS_X, 0);
    }
    #[test]
    fn exact_chunk_seams_keep_faces_behind_ice_and_partial_blocks() {
        use mcedit2_world::sections::State;
        let mk = |x: i32, name: &str, props: &[(&str, &str)]| SurfaceVoxel {
            x,
            y: 70,
            z: 0,
            span: 1,
            state: State {
                name: name.into(),
                properties: props
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect(),
            },
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: FACE_POS_X | FACE_NEG_X,
            kind: if name.ends_with("ice") {
                VoxelMaterial::Transparent
            } else {
                VoxelMaterial::Solid
            },
        };
        let mut view = SurfaceView {
            x: 15,
            z: 0,
            y_min: 64,
            y_max: 111,
            width: 32,
            depth: 16,
            voxels: vec![
                mk(15, "minecraft:stone", &[]),
                mk(16, "minecraft:ice", &[]),
            ],
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
        let mk = |x, name: &str, kind| SurfaceVoxel {
            x,
            y: 70,
            z: 0,
            span: 1,
            state: State { name: name.into(), properties: vec![] },
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: FACE_NEG_X | FACE_POS_X,
            kind,
        };
        let opaque = VoxelMaterial::Solid;
        let trans = VoxelMaterial::Transparent;
        let foliage = VoxelMaterial::Foliage;
        let liquid = VoxelMaterial::Liquid;
        let evaluate = |left_name, left_kind, right_name, right_kind| {
            let mut view = SurfaceView {
                x: 15, z: 0, y_min: 64, y_max: 111, width: 32, depth: 16,
                voxels: vec![mk(15, left_name, left_kind), mk(16, right_name, right_kind)],
                missing_columns: 0, source_chunks: 2, source_tiles: vec![],
            };
            reconcile_exact_seams(&mut view);
            (view.voxels[0].faces & FACE_POS_X != 0, view.voxels[1].faces & FACE_NEG_X != 0)
        };
        assert_eq!(evaluate("minecraft:glass", trans, "minecraft:glass", trans), (false, false));
        assert_eq!(evaluate("minecraft:glass", trans, "minecraft:ice", trans), (true, true));
        assert_eq!(evaluate("minecraft:oak_leaves", foliage, "minecraft:oak_leaves", foliage), (false, false));
        assert_eq!(evaluate("minecraft:stone", opaque, "minecraft:ice", trans), (true, false));
        assert_eq!(evaluate("minecraft:stone", opaque, "minecraft:glass", trans), (true, false));
        assert_eq!(evaluate("minecraft:stone", opaque, "minecraft:water", liquid), (true, false));
        assert_eq!(evaluate("minecraft:water", liquid, "minecraft:lava", liquid), (true, true));
        assert_eq!(evaluate("minecraft:water", liquid, "minecraft:water", liquid), (false, false));
    }
    #[test]
    fn streamed_water_seams_hide_only_verified_internal_faces() {
        use mcedit2_world::sections::State;
        let fluid = |x, name: &str, level: &str| SurfaceVoxel {
            x,
            y: 70,
            z: 0,
            span: 1,
            state: State {
                name: name.into(),
                properties: vec![("level".into(), level.into())],
            },
            biome: None,
            original_block_entity: false,
            staged: false,
            faces: FACE_POS_X | FACE_NEG_X,
            kind: VoxelMaterial::Liquid,
        };
        let mut view = SurfaceView {
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
        view.voxels[1].state.name = "minecraft:lava".into();
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
        assert!(plan([0.0, 0.0, 0.0], 65).is_err());
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
}
