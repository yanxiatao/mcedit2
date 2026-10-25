//! Continuous Minecraft spectator terrain streaming: complete chunk rings,
//! progressive source LOD, bounded reuse, no guessed absent source chunks.
//! This is display-only: sampled voxels must never enter edit/selection APIs.
use crate::Message;
use iced::Task;
use mcedit2_world::{
    editing::Session,
    far_view::{self, FarChunkPatch, FarChunkRequest},
    surface::{
        SurfaceView, SurfaceVoxel, VoxelMaterial, FACE_BOTTOM, FACE_NEG_X, FACE_NEG_Z, FACE_POS_X,
        FACE_POS_Z, FACE_TOP,
    },
    WorldError,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub(crate) const DEFAULT_RADIUS: u16 = 32;
pub(crate) const MAX_RADIUS: u16 = 64;
const CACHE_LIMIT: usize = 18_000;
const CACHE_VOXELS: usize = 96_000;
const VISIBLE_VOXELS: usize = 96_000;
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
    if radius == 0 || radius > MAX_RADIUS {
        return Err("Spectator streaming radius must be 1..64 chunks".into());
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
/// Keep the planned visible tile order stable, but issue the first inexpensive
/// LOD batch before expensive complete near-chunk reads.
fn prioritized_pending(
    requests: &[FarChunkRequest],
    cached: &BTreeMap<Key, Arc<FarChunkPatch>>,
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
/// A single background read may still be in the OS when the spectator flies
/// elsewhere. Inspect cancellation between *original* chunk reads so stale
/// jobs cannot keep scanning hundreds of now-irrelevant source chunks.
fn not_cancelled(flag: &AtomicBool) -> bool {
    !flag.load(Ordering::Acquire)
}
/// Cache trim performs one distance ordering per completed batch rather than
/// rescanning every retained chunk for each individual eviction.
fn evict_farthest(
    cache: &mut BTreeMap<Key, Arc<FarChunkPatch>>,
    cached_voxels: &mut usize,
    center: (i32, i32),
    max_entries: usize,
    max_voxels: usize,
) {
    if cache.len() <= max_entries && *cached_voxels <= max_voxels {
        return;
    }
    let mut candidates = cache.keys().copied().collect::<Vec<_>>();
    candidates.sort_unstable_by_key(|&(cx, cz, span)| {
        (
            (i64::from(cx) - i64::from(center.0))
                .abs()
                .max((i64::from(cz) - i64::from(center.1)).abs()),
            span,
            cx,
            cz,
        )
    });
    for id in candidates.into_iter().rev() {
        if cache.len() <= max_entries && *cached_voxels <= max_voxels {
            break;
        }
        if let Some(patch) = cache.remove(&id) {
            *cached_voxels = cached_voxels.saturating_sub(patch.voxels.len());
        }
    }
}
/// Only faces between verified full vanilla opaque cubes can be removed
/// after independently-read chunks are merged. Keep all other boundaries:
/// missing cells, modded models, partial blocks, liquids and LOD are not
/// evidence of a solid occluder.
fn full_opaque_cube(voxel: &SurfaceVoxel) -> bool {
    if voxel.span != 1
        || voxel.kind != VoxelMaterial::Solid
        || !voxel.state.name.starts_with("minecraft:")
    {
        return false;
    }
    let name = voxel.state.name.strip_prefix("minecraft:").unwrap_or("");
    // Positive allowlist instead of guessing that an unfamiliar vanilla
    // state is a full cube. Unknown future blocks must remain visible.
    matches!(
        name,
        "stone"
            | "granite"
            | "diorite"
            | "andesite"
            | "deepslate"
            | "cobbled_deepslate"
            | "cobblestone"
            | "mossy_cobblestone"
            | "dirt"
            | "coarse_dirt"
            | "rooted_dirt"
            | "grass_block"
            | "sand"
            | "red_sand"
            | "gravel"
            | "bedrock"
            | "netherrack"
            | "end_stone"
            | "obsidian"
            | "crying_obsidian"
            | "clay"
            | "bricks"
            | "stone_bricks"
            | "mossy_stone_bricks"
            | "cracked_stone_bricks"
            | "chiseled_stone_bricks"
            | "tuff"
            | "calcite"
            | "dripstone_block"
            | "mud"
            | "packed_mud"
            | "mud_bricks"
            | "snow_block"
            | "ice"
            | "packed_ice"
            | "blue_ice"
    ) || name.ends_with("_planks")
        || name.ends_with("_log")
        || name.ends_with("_wood")
        || name.ends_with("_concrete")
        || name.ends_with("_terracotta")
        || name.ends_with("_wool")
        || name.ends_with("_ore")
        || name.ends_with("_metal_block")
        || name.ends_with("_concrete_powder")
}
/// Verified classes: 0 is an opaque full cube, 1 and 2 are distinct
/// original vanilla fluids. Non-exact LOD cells and unknown model shapes
/// must never be used to infer neighboring source geometry.
fn seam_material(v: &SurfaceVoxel) -> Option<u8> {
    if full_opaque_cube(v) {
        return Some(0);
    }
    if v.span != 1 || v.kind != VoxelMaterial::Liquid {
        return None;
    }
    match v.state.name.as_str() {
        "minecraft:water" => Some(1),
        "minecraft:lava" => Some(2),
        _ => None,
    }
}
fn reconcile_exact_seams(view: &mut SurfaceView) {
    let occupied: BTreeMap<(i32, i32, i32), u8> = view
        .voxels
        .iter()
        .filter_map(|v| seam_material(v).map(|class| ((v.x, v.y, v.z), class)))
        .collect();
    for v in &mut view.voxels {
        let Some(kind) = seam_material(v) else {
            continue;
        };
        for (dx, dy, dz, face) in [
            (0, 1, 0, FACE_TOP),
            (0, -1, 0, FACE_BOTTOM),
            (1, 0, 0, FACE_POS_X),
            (-1, 0, 0, FACE_NEG_X),
            (0, 0, 1, FACE_POS_Z),
            (0, 0, -1, FACE_NEG_Z),
        ] {
            if let (Some(x), Some(y), Some(z)) = (
                v.x.checked_add(dx),
                v.y.checked_add(dy),
                v.z.checked_add(dz),
            ) {
                // Opaque/opaque: eliminate hidden inner faces.
                // Fluid/fluid: hide only equal fluids, even at different
                // levels, to preserve continuously interpolated surfaces.
                // Fluid/opaque: no fluid wall is visible through stone;
                // still retain the opaque face visible through the water.
                let occluded = occupied.get(&(x, y, z)).is_some_and(|&other| {
                    (kind == 0 && other == 0) || (kind != 0 && (other == kind || other == 0))
                });
                if occluded {
                    v.faces &= !face;
                }
            }
        }
    }
}
pub(crate) struct Stream {
    pub key: (usize, usize),
    pub source: Arc<Session>,
    pub center: (i32, i32),
    pub radius: u16,
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
        nonce: u64,
    ) -> Result<Self, String> {
        let exact_y_min = Self::height_window(&source, location);
        let mut state = Self {
            key,
            source,
            center: center_chunk(location),
            radius,
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
        state.replan(location, radius, nonce)?;
        Ok(state)
    }
    /// Camera navigation never mutates world state. A pending old batch
    /// may finish; the UI nonce check discards it before publishing.
    pub fn replan(&mut self, location: [f64; 3], radius: u16, nonce: u64) -> Result<(), String> {
        let requests = plan(location, radius)?;
        let exact_y_min = Self::height_window(&self.source, location);
        let changed_height = self.exact_y_min != exact_y_min;
        let needed: BTreeSet<_> = requests.iter().copied().map(key).collect();
        // Exact voxel chunks depend on the observer's vertical view window.
        // Distant sampled source-height columns do not.
        self.cache
            .retain(|id, _| needed.contains(id) && !(changed_height && id.2 == 1));
        self.exact_y_min = exact_y_min;
        self.cached_voxels = self.cache.values().map(|p| p.voxels.len()).sum();
        self.complete = requests
            .iter()
            .filter(|r| self.cache.contains_key(&key(**r)))
            .count();
        self.total = requests.len();
        self.published = self.complete;
        self.pending = prioritized_pending(&requests, &self.cache);
        self.center = center_chunk(location);
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
                    if n <= CACHE_VOXELS {
                        self.cached_voxels += n;
                        self.cache.insert(id, Arc::new(patch));
                    }
                }
                Err(_) => self.errors += 1,
            }
        }
        // Prune farthest source snapshots in one ordered pass. The cache
        // is display-only and must never exceed its bounded voxel budget.
        evict_farthest(
            &mut self.cache,
            &mut self.cached_voxels,
            self.center,
            CACHE_LIMIT,
            CACHE_VOXELS,
        );
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
    pub fn combined(&self, near: &SurfaceView) -> SurfaceView {
        let mut result = near.clone();
        // Source near-field window may not align with chunk boundaries.
        // Do not overlay sparse LOD onto any chunk touching exact 3D volume.
        let overlaps = |req: FarChunkRequest| {
            let x = req.cx as i64 * 16;
            let z = req.cz as i64 * 16;
            x < near.x as i64 + near.width as i64
                && x + 16 > near.x as i64
                && z < near.z as i64 + near.depth as i64
                && z + 16 > near.z as i64
        };
        // Source-aware budget: a dense exact ring must not use all visible
        // geometry memory before there is room for verified distant samples.
        // Keep complete patches (never truncate a real chunk midway).
        let has_distant = self
            .wanted
            .iter()
            .any(|r| r.span > 1 && self.cache.contains_key(&key(*r)));
        let near_limit = if has_distant {
            VISIBLE_VOXELS * 3 / 4
        } else {
            VISIBLE_VOXELS
        };
        for req in &self.wanted {
            if overlaps(*req) {
                continue;
            }
            let Some(patch) = self.cache.get(&key(*req)) else {
                continue;
            };
            let limit = if req.span == 1 {
                near_limit
            } else {
                VISIBLE_VOXELS
            };
            if result.voxels.len().saturating_add(patch.voxels.len()) > limit {
                continue;
            }
            result.voxels.extend(patch.voxels.iter().cloned());
            result.source_tiles.push(patch.tile.clone());
            result.source_chunks += usize::from(patch.tile.source_present);
            result.missing_columns += patch.tile.unknown_columns;
        }
        reconcile_exact_seams(&mut result);
        result
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
    #[test]
    fn first_batch_previews_horizon_then_restores_nearby_detail() {
        let requests = plan([0.0, 80.0, 0.0], 16).unwrap();
        let pending = prioritized_pending(&requests, &BTreeMap::new());
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
    fn cancelled_stale_batch_stops_at_next_chunk_boundary() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker = Arc::clone(&cancelled);
        assert!(not_cancelled(&worker));
        cancelled.store(true, Ordering::Release);
        assert!(!not_cancelled(&worker));
    }
    #[test]
    fn cache_eviction_is_bounded_and_keeps_nearer_chunk() {
        use mcedit2_world::surface::SourceChunkTile;
        let mut cache = BTreeMap::new();
        let empty = |cx| {
            Arc::new(FarChunkPatch {
                tile: SourceChunkTile {
                    chunk_x: cx,
                    chunk_z: 0,
                    source_present: true,
                    known_columns: 0,
                    unknown_columns: 0,
                    exposed_voxels: 0,
                },
                voxels: Vec::new(),
            })
        };
        cache.insert((0, 0, 4), empty(0));
        cache.insert((10, 0, 4), empty(10));
        cache.insert((30, 0, 4), empty(30));
        let mut count = 0;
        evict_farthest(&mut cache, &mut count, (0, 0), 2, 10);
        assert_eq!(cache.len(), 2);
        assert!(cache.contains_key(&(0, 0, 4)));
        assert!(!cache.contains_key(&(30, 0, 4)));
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
