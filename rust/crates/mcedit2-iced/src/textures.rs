//! Optional non-mutating Java resource-pack block PNG reader.
//! Supports an extracted pack directory or a ZIP, never executes or extracts
//! pack data; missing/unknown textures retain the existing safe color guide.
use crate::{resource_ids, resource_models::ModelBank};
use image::ImageFormat;
use mcedit2_world::sections::State;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Seek};
use std::path::Path;
use std::sync::Arc;

const MISSING_TEXTURE_ID: &str = "mcedit2:missingno";
fn missing_texture() -> Tile {
    let mut rgba = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16 {
        for x in 0..16 {
            let purple = (x < 8) == (y < 8);
            rgba.extend_from_slice(if purple {
                &[248, 0, 248, 255]
            } else {
                &[0, 0, 0, 255]
            });
        }
    }
    Tile {
        width: 16,
        height: 16,
        rgba,
    }
}
// PNG count is constrained by GPU atlas capacity, not an arbitrary 4096.
const MAX_TEXTURE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PIXEL_EDGE: u32 = 4096;
const MAX_IMAGE_RGBA: u64 = 128 * 1024 * 1024;
const MAX_RGBA_MEMORY: usize = 512 * 1024 * 1024;
const MAX_ANIMATION_META: usize = 32 * 1024;
const MAX_ANIMATION_STEPS: usize = 4096;
#[cfg(test)]
const MIN_ATLAS_CELL: u32 = 16;
const MAX_ATLAS_CELL: u32 = 4096;
/// Rendering semantics, derived from actual PNG alpha instead of block ID
/// guesses. Liquid/foliage may override opaque PNGs in the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum AlphaMode {
    Opaque = 0,
    Cutout = 1,
    Blend = 2,
}
impl AlphaMode {
    fn from_pixels(pixels: &[u8]) -> Self {
        let mut zero = false;
        let mut partial = false;
        for alpha in pixels.iter().skip(3).step_by(4) {
            if *alpha == 0 {
                zero = true;
            } else if *alpha < 255 {
                partial = true;
            }
            if partial {
                return Self::Blend;
            }
        }
        if zero {
            Self::Cutout
        } else {
            Self::Opaque
        }
    }
}
pub(crate) fn classify_block_alpha(
    state: &State,
    kind: mcedit2_world::surface::VoxelMaterial,
    png: AlphaMode,
) -> (AlphaMode, f32) {
    use mcedit2_world::surface::VoxelMaterial;
    match kind {
        VoxelMaterial::Liquid => (AlphaMode::Blend, 0.76),
        VoxelMaterial::Transparent => {
            let name = state.name.as_str();
            // Original Java glass with a binary-alpha PNG is a cutout:
            // the dark border is solid, only actual transparent texels
            // reveal the scene. True partial-alpha stained sprites blend.
            // Ice has its own translucency independently of sprite alpha.
            if name.contains("glass") {
                match png {
                    AlphaMode::Cutout => (AlphaMode::Cutout, 1.0),
                    AlphaMode::Blend => (AlphaMode::Blend, 1.0),
                    AlphaMode::Opaque => (AlphaMode::Blend, 0.72),
                }
            } else if name.ends_with(":ice") || name.ends_with("_ice") {
                (AlphaMode::Blend, 0.72)
            } else {
                (png, 1.0)
            }
        }
        VoxelMaterial::Foliage | VoxelMaterial::Solid => (png, 1.0),
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Tile {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl Tile {
    pub fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::from_pixels(&self.rgba)
    }
    pub fn pixel(&self, u: f32, v: f32) -> [u8; 4] {
        let x = (u.clamp(0.0, 0.999_999) * self.width as f32) as usize;
        let y = (v.clamp(0.0, 0.999_999) * self.height as f32) as usize;
        let at = (y * self.width as usize + x) * 4;
        [
            self.rgba[at],
            self.rgba[at + 1],
            self.rgba[at + 2],
            self.rgba[at + 3],
        ]
    }
}
#[derive(Clone, Debug)]
struct SpriteAnimation {
    frames: Vec<Tile>,
    schedule: Vec<(usize, u16)>,
    cycle: u32,
}
impl SpriteAnimation {
    fn from_metadata(frames: Vec<Tile>, meta: &[u8]) -> Result<Self, String> {
        if frames.len() < 2 {
            return Err("Animated PNG needs two or more frames".into());
        }
        if meta.len() > MAX_ANIMATION_META {
            return Err("Animation metadata exceeds 32KiB".into());
        }
        let root: serde_json::Value =
            serde_json::from_slice(meta).map_err(|e| format!("Invalid animation metadata: {e}"))?;
        let animation = root
            .get("animation")
            .and_then(|v| v.as_object())
            .ok_or("Missing animation metadata object")?;
        let frame_time = animation
            .get("frametime")
            .map(|v| v.as_u64())
            .unwrap_or(Some(1))
            .ok_or("Animation frametime must be positive")?;
        if !(1..=1200).contains(&frame_time) {
            return Err("Animation frametime must be between 1 and 1200 ticks".into());
        }
        let schedule = if let Some(entries) = animation.get("frames") {
            let entries = entries
                .as_array()
                .ok_or("Animation frames must be an array")?;
            if entries.is_empty() || entries.len() > MAX_ANIMATION_STEPS {
                return Err("Animation frame sequence is empty or too long".into());
            }
            let mut sequence = Vec::with_capacity(entries.len());
            for item in entries {
                let (index, duration) = if let Some(index) = item.as_u64() {
                    (index, frame_time)
                } else {
                    (
                        item.get("index")
                            .and_then(|v| v.as_u64())
                            .ok_or("Animation frame requires an index")?,
                        item.get("time")
                            .map(|v| v.as_u64())
                            .unwrap_or(Some(frame_time))
                            .ok_or("Animation frame time must be positive")?,
                    )
                };
                if index >= frames.len() as u64 || !(1..=1200).contains(&duration) {
                    return Err("Animation step index or duration out of bounds".into());
                }
                sequence.push((index as usize, duration as u16));
            }
            sequence
        } else {
            frames
                .iter()
                .enumerate()
                .map(|(i, _)| (i, frame_time as u16))
                .collect()
        };
        let cycle = schedule.iter().map(|(_, time)| *time as u32).sum();
        if cycle == 0 || cycle > 1_000_000 {
            return Err("Animation cycle exceeds bounded tick budget".into());
        }
        Ok(Self {
            frames,
            schedule,
            cycle,
        })
    }
    fn index_at(&self, ticks: u64) -> usize {
        let mut offset = (ticks % self.cycle as u64) as u32;
        for (index, time) in &self.schedule {
            if offset < *time as u32 {
                return *index;
            }
            offset -= *time as u32;
        }
        self.schedule.last().map(|x| x.0).unwrap_or(0)
    }
    fn tile_at(&self, ticks: u64) -> &Tile {
        &self.frames[self.index_at(ticks)]
    }
    fn alpha_mode(&self) -> AlphaMode {
        // The editor intentionally freezes animated textures at tick zero.
        // Classify the *displayed* frame, not every unseen animation frame.
        self.tile_at(0).alpha_mode()
    }
}
/// UVs refer to the center of each 32px atlas texel; slots have a
/// 1-pixel duplicated gutter on all sides to prevent texture bleeding.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AtlasUv {
    pub start: [f32; 2],
    pub end: [f32; 2],
    pub page: u32,
}
fn orient_uv(corners: [[f32; 2]; 4], permutation: [u8; 4]) -> [[f32; 2]; 4] {
    permutation.map(|index| corners[index as usize])
}
impl AtlasUv {
    pub fn corners_for(&self, uv: [f32; 4], rotation: u16) -> [[f32; 2]; 4] {
        let mut corners = [
            self.sample(uv[0] / 16.0, uv[1] / 16.0),
            self.sample(uv[2] / 16.0, uv[1] / 16.0),
            self.sample(uv[2] / 16.0, uv[3] / 16.0),
            self.sample(uv[0] / 16.0, uv[3] / 16.0),
        ];
        for _ in 0..rotation / 90 {
            corners.rotate_right(1);
        }
        corners
    }
    pub fn sample(&self, u: f32, v: f32) -> [f32; 2] {
        let u = u.clamp(0.0, 1.0);
        let v = v.clamp(0.0, 1.0);
        [
            self.start[0] + (self.end[0] - self.start[0]) * u,
            self.start[1] + (self.end[1] - self.start[1]) * v,
        ]
    }
}
#[derive(Clone, Debug)]
pub(crate) struct TextureAtlas {
    pub pages: Vec<Arc<AtlasPage>>,
    pub entries: BTreeMap<String, AtlasUv>,
    #[cfg(test)]
    slots: BTreeMap<String, SlotPlacement>,
    cell: u32,
    pub white: AtlasUv,
}
/// Every atlas page uses the same GPU allocation size, allowing one
/// texture_2d_array binding and a per-vertex layer index on wgpu.
#[derive(Clone, Debug)]
pub(crate) struct AtlasPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
const MAX_PAGE_LAYERS: u32 = 256;
const MAX_ATLAS_GPU_BYTES: u64 = 768 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SlotPlacement {
    page: usize,
    x: u32,
    y: u32,
    cell: u32,
}
struct MixedLayout {
    edge: u32,
    slots: Vec<SlotPlacement>,
    pages: usize,
    max_cell: u32,
}
/// Deterministically shelf-pack mixed source sprite resolutions into an
/// array of equal-sized GPU pages, without promoting every 16px tile to HD.
fn mixed_layout(tiles: &BTreeMap<String, Tile>) -> Result<MixedLayout, String> {
    // Stable sprite IDs, but pack large sprites first to avoid fragmentation.
    let cells = std::iter::once(16u32)
        .chain(tiles.values().map(|t| {
            t.width
                .max(t.height)
                .max(16)
                .next_power_of_two()
                .min(MAX_ATLAS_CELL)
        }))
        .collect::<Vec<_>>();
    for factor in [1u32, 2, 4, 8, 16, 32, 64, 128, 256] {
        let sizes = cells
            .iter()
            .map(|c| (*c / factor).max(16))
            .collect::<Vec<_>>();
        let max_cell = *sizes.iter().max().unwrap_or(&16);
        let sum = sizes
            .iter()
            .try_fold(0u64, |acc, c| acc.checked_add((*c as u64 + 2).pow(2)))
            .ok_or("Atlas area overflow")?;
        let max_edge = if max_cell > 2048 { 8192 } else { 2048 };
        let edge = if max_cell > 2048 {
            8192
        } else {
            (sum as f64).sqrt().ceil().max(64.0) as u32
        }
        .next_power_of_two()
        .clamp(64, max_edge);
        let mut order = (0..sizes.len()).collect::<Vec<_>>();
        order.sort_by_key(|&i| (std::cmp::Reverse(sizes[i]), i));
        let mut shelves: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new()];
        let mut next_y = vec![0u32];
        let mut slots = vec![
            SlotPlacement {
                page: 0,
                x: 0,
                y: 0,
                cell: 16
            };
            sizes.len()
        ];
        let mut valid = true;
        for i in order {
            let cell = sizes[i];
            let stride = cell + 2;
            if stride > edge {
                valid = false;
                break;
            }
            let mut found = None;
            for (page, rows) in shelves.iter().enumerate() {
                for (row, &(used, _, height)) in rows.iter().enumerate() {
                    if height >= stride && edge - used >= stride {
                        let waste = height - stride;
                        if found.is_none_or(|(_, _, best)| waste < best) {
                            found = Some((page, row, waste));
                        }
                    }
                }
            }
            if let Some((page, row, _)) = found {
                let (used, y, _) = &mut shelves[page][row];
                slots[i] = SlotPlacement {
                    page,
                    x: *used,
                    y: *y,
                    cell,
                };
                *used += stride;
                continue;
            }
            let mut page = next_y.iter().position(|&y| edge - y >= stride);
            if page.is_none() {
                if shelves.len() >= MAX_PAGE_LAYERS as usize {
                    valid = false;
                    break;
                }
                shelves.push(Vec::new());
                next_y.push(0);
                page = Some(shelves.len() - 1);
            }
            let page = page.unwrap();
            let y = next_y[page];
            shelves[page].push((stride, y, stride));
            next_y[page] += stride;
            slots[i] = SlotPlacement {
                page,
                x: 0,
                y,
                cell,
            };
        }
        let pages = shelves.len();
        if valid
            && pages <= MAX_PAGE_LAYERS as usize
            && edge as u64 * edge as u64 * 4 * pages as u64 <= MAX_ATLAS_GPU_BYTES
        {
            return Ok(MixedLayout {
                edge,
                slots,
                pages,
                max_cell,
            });
        }
    }
    Err("Mixed-resolution texture atlas exceeds GPU page/memory budget".into())
}
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
struct PagePlan {
    columns: u32,
    rows: u32,
    width: u32,
    height: u32,
    capacity: usize,
    pages: usize,
    cell: u32,
    stride: u32,
}
#[cfg(test)]
fn page_plan(slots: usize, cell: u32) -> Result<PagePlan, String> {
    if slots == 0 {
        return Err("Atlas requires a white tile".into());
    }
    if cell < MIN_ATLAS_CELL || cell > MAX_ATLAS_CELL || !cell.is_power_of_two() {
        return Err("Unsupported source sprite atlas resolution".into());
    }
    let stride = cell + 2; // 1px padding each side
    let max_page_edge = if cell > 1024 { 8192 } else { 2048 };
    let max_cols = max_page_edge / stride;
    let columns = ((slots as f64).sqrt().ceil() as u32).clamp(1, max_cols);
    let rows = (slots.div_ceil(columns as usize) as u32).clamp(1, max_cols);
    let capacity = columns as usize * rows as usize;
    let pages = slots.div_ceil(capacity);
    if pages > MAX_PAGE_LAYERS as usize {
        return Err("Resource atlas exceeds 256 portable GPU array layers".into());
    }
    let width = columns * stride;
    let height = rows * stride;
    let bytes = width as u64 * height as u64 * 4 * pages as u64;
    if bytes > MAX_ATLAS_GPU_BYTES {
        return Err("Resource atlas exceeds 768 MiB GPU memory budget".into());
    }
    Ok(PagePlan {
        columns,
        rows,
        width,
        height,
        capacity,
        pages,
        cell,
        stride,
    })
}
fn padded_tile(tile: &Tile, cell: u32) -> Vec<[u8; 4]> {
    let n = cell as usize;
    let mut data = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            data.push(tile.pixel((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32));
        }
    }
    // Preserve the source alpha while extending edge RGB into transparent
    // pixels, preventing halos around cutout glass/foliage at page edges.
    if tile.alpha_mode() == AlphaMode::Cutout {
        let original = data.clone();
        for y in 0..n {
            for x in 0..n {
                if original[y * n + x][3] != 0 {
                    continue;
                }
                'nearest: for radius in 1_i32..=3 {
                    for dy in -radius..=radius {
                        let dx = radius - dy.abs();
                        for nx in [x as i32 - dx, x as i32 + dx] {
                            let ny = y as i32 + dy;
                            if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                                continue;
                            }
                            let sample = original[ny as usize * n + nx as usize];
                            if sample[3] == 255 {
                                data[y * n + x][..3].copy_from_slice(&sample[..3]);
                                break 'nearest;
                            }
                        }
                    }
                }
            }
        }
    }
    data
}
impl TextureAtlas {
    #[cfg(test)]
    fn create(tiles: &BTreeMap<String, Tile>) -> Self {
        Self::create_at(tiles, &BTreeMap::new(), 0)
    }
    fn create_at(
        tiles: &BTreeMap<String, Tile>,
        animations: &BTreeMap<String, SpriteAnimation>,
        tick: u64,
    ) -> Self {
        let layout = mixed_layout(tiles).expect("Mixed atlas prevalidated");
        let mut pages = (0..layout.pages)
            .map(|_| AtlasPage {
                width: layout.edge,
                height: layout.edge,
                rgba: vec![0u8; layout.edge as usize * layout.edge as usize * 4],
            })
            .collect::<Vec<_>>();
        let uv = |slot: SlotPlacement| {
            let x = slot.x + 1;
            let y = slot.y + 1;
            AtlasUv {
                start: [
                    (x as f32 + 0.5) / layout.edge as f32,
                    (y as f32 + 0.5) / layout.edge as f32,
                ],
                end: [
                    (x as f32 + slot.cell as f32 - 0.5) / layout.edge as f32,
                    (y as f32 + slot.cell as f32 - 0.5) / layout.edge as f32,
                ],
                page: slot.page as u32,
            }
        };
        let white = Tile {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
        };
        write_placement(&mut pages[0], layout.slots[0], &white);
        let mut entries = BTreeMap::new();
        #[cfg(test)]
        let mut slots = BTreeMap::new();
        for (index, (name, tile)) in tiles.iter().enumerate() {
            let slot = layout.slots[index + 1];
            let source = animations
                .get(name)
                .map(|a| a.tile_at(tick))
                .unwrap_or(tile);
            write_placement(&mut pages[slot.page], slot, source);
            entries.insert(name.clone(), uv(slot));
            #[cfg(test)]
            slots.insert(name.clone(), slot);
        }
        Self {
            pages: pages.into_iter().map(Arc::new).collect(),
            entries,
            #[cfg(test)]
            slots,
            white: uv(layout.slots[0]),
            cell: layout.max_cell,
        }
    }
    #[cfg(test)]
    fn advance(
        &self,
        animations: &BTreeMap<String, SpriteAnimation>,
        previous: u64,
        next_tick: u64,
    ) -> Self {
        let mut next = self.clone();
        for (name, animation) in animations {
            if animation.index_at(previous) == animation.index_at(next_tick) {
                continue;
            }
            let Some(&slot) = self.slots.get(name) else {
                continue;
            };
            write_placement(
                Arc::make_mut(&mut next.pages[slot.page]),
                slot,
                animation.tile_at(next_tick),
            );
        }
        next
    }
    pub(crate) fn page_count(&self) -> usize {
        self.pages.len()
    }
    pub(crate) fn tile_resolution(&self) -> u32 {
        self.cell
    }
    pub(crate) fn page(&self, index: u32) -> Option<&AtlasPage> {
        self.pages.get(index as usize).map(Arc::as_ref)
    }
}
fn write_placement(page: &mut AtlasPage, slot: SlotPlacement, tile: &Tile) {
    let data = padded_tile(tile, slot.cell);
    for y in 0..slot.cell + 2 {
        for x in 0..slot.cell + 2 {
            let xx = (x as i32 - 1).clamp(0, slot.cell as i32 - 1) as usize;
            let yy = (y as i32 - 1).clamp(0, slot.cell as i32 - 1) as usize;
            let at = (((slot.y + y) * page.width + slot.x + x) * 4) as usize;
            page.rgba[at..at + 4].copy_from_slice(&data[yy * slot.cell as usize + xx]);
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StaticObjTriangle {
    pub xyz: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub page: u32,
    pub tint: [f32;4],
    /// Low two bits are opaque/cutout/blend; high bits encode static glTF
    /// sidedness, cutoff and OPAQUE alpha semantics.
    pub mode: u32,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct TextureBank {
    // Original PNG/JSON source is immutable and shared across animation.
    // Only changed RGBA pages are cloned on a sprite frame transition.
    tiles: Arc<BTreeMap<String, Tile>>,
    models: Arc<ModelBank>,
    // Decoded full animation strips exist only during resource-pack loading.
    // Once .mcmeta selects tick zero, only the frozen tile is retained.
    animations: Arc<BTreeMap<String, SpriteAnimation>>,
    static_animation_count: usize,
    metadata: BTreeMap<String, Vec<u8>>,
    grass_colormap: Option<Tile>,
    foliage_colormap: Option<Tile>,
    modern_textures: BTreeSet<String>,
    modern_metadata: BTreeSet<String>,
    // Track identities replaced in each overlay independently of entry order.
    layer_png: BTreeSet<String>,
    layer_metadata: BTreeSet<String>,
    atlas: Option<Arc<TextureAtlas>>,
    #[cfg(test)]
    animation_tick: u64,
    #[cfg(test)]
    pub source: String,
    pub warnings: Vec<String>,
}
/// Parse source texture locations from all Java block-model-capable
/// namespaces. Keep block/ and legacy blocks/ aliases backward compatible;
/// retain other folders in the identity so similarly-named sprites cannot
/// collide. The atlas loader still applies its existing memory budget.
fn texture_id(path: &str) -> Option<String> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() < 4
        || parts[0] != "assets"
        || !matches!(parts[2], "textures" | "models")
        || (parts[2] == "textures" && parts.len() < 5)
        || !path.ends_with(".png")
    {
        return None;
    }
    let namespace = parts[1];
    if !resource_ids::namespace(namespace) {
        return None;
    }
    let relative = parts[3..].join("/");
    let name = relative.strip_suffix(".png")?;
    if !resource_ids::path(name, 160) {
        return None;
    }
    if parts[2] == "models" {
        // glTF image URIs commonly live beside the .gltf itself, not in
        // Minecraft's textures/ folder. Give them a collision-free atlas ID.
        return Some(format!("{namespace}:__gltf/{name}"));
    }
    // Climate tables have an independent 256x256 source decoder.
    if namespace == "minecraft" && matches!(name, "colormap/grass" | "colormap/foliage") {
        return None;
    }
    let name = name
        .strip_prefix("block/")
        .or_else(|| name.strip_prefix("blocks/"))
        .unwrap_or(name);
    Some(format!("{namespace}:{name}"))
}
fn decode_png_frames(bytes: &[u8]) -> Result<Vec<Tile>, String> {
    if bytes.len() as u64 > MAX_TEXTURE_BYTES {
        return Err("PNG compressed source exceeds 32MiB".into());
    }
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("Invalid PNG header".into());
    }
    let source_width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let source_height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if source_width == 0
        || source_height == 0
        || source_width > MAX_PIXEL_EDGE
        || source_height > 8192
    {
        return Err("PNG header exceeds safe dimensions".into());
    }
    if source_width as u64 * source_height as u64 * 4 > MAX_IMAGE_RGBA {
        return Err("Single PNG exceeds 128MiB decoded safety budget".into());
    }
    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png)
        .map_err(|e| format!("Invalid PNG: {e}"))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || width > MAX_PIXEL_EDGE || height > 8192 {
        return Err("Texture PNG exceeds decoded frame safeguards".into());
    }
    let frame_h = if height >= width && height % width == 0 {
        width
    } else {
        height
    };
    if frame_h > MAX_PIXEL_EDGE {
        return Err("Animation frame exceeds 4096px".into());
    }
    let count = height / frame_h;
    if count as usize > MAX_ANIMATION_STEPS {
        return Err("Animated source contains too many frames".into());
    }
    let pixels = image.into_raw();
    let row_bytes = width as usize * frame_h as usize * 4;
    Ok((0..count)
        .map(|index| Tile {
            width,
            height: frame_h,
            rgba: pixels[index as usize * row_bytes..(index as usize + 1) * row_bytes].to_vec(),
        })
        .collect())
}
fn decode_png(bytes: &[u8]) -> Result<Tile, String> {
    decode_png_frames(bytes)?
        .into_iter()
        .next()
        .ok_or("Empty PNG".into())
}
/// The source biome palette contains registry IDs, not the world's
/// datapack-defined climate. Use vanilla values where known; modded IDs
/// deliberately fall back rather than claim an invented exact biome color.
fn biome_climate(biome: &str) -> Option<(f32, f32)> {
    Some(match biome {
        "minecraft:plains" | "minecraft:sunflower_plains" => (0.8, 0.4),
        "minecraft:forest" | "minecraft:flower_forest" => (0.7, 0.8),
        "minecraft:birch_forest" | "minecraft:old_growth_birch_forest" => (0.6, 0.6),
        "minecraft:dark_forest" => (0.7, 0.8),
        "minecraft:jungle" | "minecraft:bamboo_jungle" => (0.95, 0.9),
        "minecraft:sparse_jungle" => (0.95, 0.8),
        "minecraft:savanna" | "minecraft:savanna_plateau" => (1.2, 0.0),
        "minecraft:desert" => (2.0, 0.0),
        "minecraft:badlands" | "minecraft:eroded_badlands" | "minecraft:wooded_badlands" => {
            (2.0, 0.0)
        }
        "minecraft:swamp" | "minecraft:mangrove_swamp" => (0.8, 0.9),
        "minecraft:taiga"
        | "minecraft:old_growth_pine_taiga"
        | "minecraft:old_growth_spruce_taiga" => (0.25, 0.8),
        "minecraft:snowy_taiga" => (-0.5, 0.4),
        "minecraft:snowy_plains" | "minecraft:ice_spikes" => (0.0, 0.5),
        "minecraft:meadow" => (0.5, 0.8),
        "minecraft:cherry_grove" => (0.5, 0.8),
        "minecraft:river" => (0.5, 0.5),
        "minecraft:beach" => (0.8, 0.4),
        _ => return None,
    })
}
/// Only documented vanilla vegetation receives fallback tint when the
/// actual source JSON parent is unavailable. Unsupported namespaces never
/// inherit another mod's tint, and grass-block dirt sides remain untinted.
fn known_vanilla_biome_tint(state: &State, face: crate::isometric::Face) -> bool {
    let Some(name) = state.name.strip_prefix("minecraft:") else {
        return false;
    };
    if name == "grass_block" {
        return face == crate::isometric::Face::Top;
    }
    matches!(
        name,
        "oak_leaves"
            | "spruce_leaves"
            | "birch_leaves"
            | "jungle_leaves"
            | "acacia_leaves"
            | "dark_oak_leaves"
            | "mangrove_leaves"
            | "vine"
            | "short_grass"
            | "tall_grass"
            | "grass"
            | "fern"
            | "large_fern"
    )
}
fn biome_color(biome: Option<&str>, name: &str) -> [f32; 3] {
    let rgb = match (biome.unwrap_or(""), name) {
        ("minecraft:swamp" | "minecraft:mangrove_swamp", "water") => 0x617B64u32,
        ("minecraft:warm_ocean" | "minecraft:lukewarm_ocean", "water") => 0x43D5EE,
        ("minecraft:cold_ocean" | "minecraft:deep_cold_ocean", "water") => 0x3D57D6,
        ("minecraft:frozen_ocean" | "minecraft:deep_frozen_ocean", "water") => 0x3938C9,
        (_, "water") => 0x3F76E4,
        ("minecraft:swamp", "grass") => 0x6A7039,
        ("minecraft:dark_forest", "grass") => 0x507A32,
        (
            "minecraft:badlands" | "minecraft:eroded_badlands" | "minecraft:wooded_badlands",
            "grass",
        ) => 0x90814D,
        ("minecraft:jungle" | "minecraft:sparse_jungle" | "minecraft:bamboo_jungle", "grass") => {
            0x59C93C
        }
        ("minecraft:desert" | "minecraft:savanna" | "minecraft:savanna_plateau", "grass") => {
            0xBFB755
        }
        ("minecraft:snowy_plains" | "minecraft:snowy_taiga", "grass") => 0x80B497,
        ("minecraft:swamp", "foliage") => 0x6A7039,
        (
            "minecraft:badlands" | "minecraft:eroded_badlands" | "minecraft:wooded_badlands",
            "foliage",
        ) => 0x9E814D,
        ("minecraft:jungle" | "minecraft:sparse_jungle" | "minecraft:bamboo_jungle", "foliage") => {
            0x30BB0B
        }
        (_, "foliage") => 0x48B518,
        (_, "grass") => 0x91BD59,
        _ => 0xFFFFFF,
    };
    [
        ((rgb >> 16) & 255) as f32 / 255.0,
        ((rgb >> 8) & 255) as f32 / 255.0,
        (rgb & 255) as f32 / 255.0,
    ]
}

/// The source archive and directory walkers share one canonical resource
/// classifier. In particular, a high-priority texture's identity is only
/// committed after its bytes decode, regardless of ZIP entry order.
enum PackAsset {
    Colormap(&'static str),
    Model,
    Obj,
    Gltf,
    GltfBuffer,
    Mtl,
    Metadata(String),
    Texture(String),
}
impl PackAsset {
    fn classify(name: &str) -> Option<Self> {
        if name == "assets/minecraft/textures/colormap/grass.png" {
            return Some(Self::Colormap("grass"));
        }
        if name == "assets/minecraft/textures/colormap/foliage.png" {
            return Some(Self::Colormap("foliage"));
        }
        if let Some(name) = name.strip_suffix(".mcmeta") {
            return texture_id(name).map(Self::Metadata);
        }
        if let Some(id) = texture_id(name) {
            return Some(Self::Texture(id));
        }
        if name.ends_with(".bin") && name.contains("/models/") {
            return Some(Self::GltfBuffer);
        }
        if (name.ends_with(".gltf") || name.ends_with(".glb")) && name.contains("/models/") {
            return Some(Self::Gltf);
        }
        if name.ends_with(".mtl") && name.contains("/models/") {
            return Some(Self::Mtl);
        }
        if name.ends_with(".obj") && name.contains("/models/") {
            return Some(Self::Obj);
        }
        (name.ends_with(".json")
            && (name.contains("/models/") || name.contains("/blockstates/")))
        .then_some(Self::Model)
    }
    fn max_bytes(&self) -> u64 {
        match self {
            Self::Model | Self::Obj | Self::Mtl => 2 * 1024 * 1024,
            Self::Gltf | Self::GltfBuffer => 8 * 1024 * 1024,
            Self::Metadata(_) => MAX_ANIMATION_META as u64,
            Self::Colormap(_) | Self::Texture(_) => MAX_TEXTURE_BYTES,
        }
    }
}

/// Bake static Wavefront map_d into RGBA *before* the shared atlas is
/// allocated. A separate second atlas sample per fragment is unnecessary,
/// and textured cutout opacity is preserved at pixel resolution.
fn combine_opacity(diffuse:&Tile,mask:&Tile)->Tile{
    let mut rgba=diffuse.rgba.clone();
    let width=diffuse.width as usize;
    let height=diffuse.height as usize;
    for y in 0..height{
        for x in 0..width{
            let m=mask.pixel((x as f32+0.5)/width as f32,(y as f32+0.5)/height as f32);
            let luma=(0.2126*m[0] as f32+0.7152*m[1] as f32
                +0.0722*m[2] as f32).round() as u16;
            // Grayscale maps use luminance; PNG alpha masks use Alpha.
            let opacity=if m[3]<255 {m[3] as u16} else {luma};
            let i=(y*width+x)*4+3;
            rgba[i]=((rgba[i] as u16*opacity+127)/255) as u8;
        }
    }
    Tile{width:diffuse.width,height:diffuse.height,rgba}
}
fn material_texture_id(model:&str,raw:&str)->Option<String>{
    let ns=model.strip_prefix("model:")?.split_once(':')?.0;
    let id=resource_ids::qualified(raw,ns)?;
    let (ns,path)=id.split_once(':')?;
    let path=path.strip_prefix("block/")
        .or_else(||path.strip_prefix("blocks/"))
        .unwrap_or(path);
    Some(format!("{ns}:{path}"))
}
impl TextureBank {
    fn ingest_asset(&mut self, name: &str, asset: PackAsset, bytes: Vec<u8>, memory: &mut usize) {
        match asset {
            PackAsset::Colormap(which) => {
                if let Err(e) = self.insert_colormap(which, &bytes) {
                    self.warnings.push(format!("Skipped {name}: {e}"));
                }
            }
            PackAsset::Model => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert(name, &bytes) {
                    self.warnings.push(format!("Skipped model {name}: {e}"));
                }
            }
            PackAsset::Obj => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert_obj(name, &bytes) {
                    self.warnings.push(format!("Skipped OBJ {name}: {e}"));
                }
            }
            PackAsset::Gltf => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert_gltf(name, &bytes) {
                    self.warnings.push(format!("Skipped glTF {name}: {e}"));
                }
            }
            PackAsset::GltfBuffer => {
                if let Err(e)=Arc::make_mut(&mut self.models).insert_gltf_buffer(name,&bytes) {
                    self.warnings.push(format!("Skipped glTF buffer {name}: {e}"));
                }
            }
            PackAsset::Mtl => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert_mtl(name, &bytes) {
                    self.warnings.push(format!("Skipped MTL {name}: {e}"));
                }
            }
            PackAsset::Metadata(id) => {
                if self.accept_metadata_path(&id, name) && self.metadata.len() < 65536 {
                    self.layer_metadata.insert(id.clone());
                    self.metadata.insert(id, bytes);
                }
            }
            PackAsset::Texture(id) => {
                if self.accept_texture_path(&id, name) {
                    if let Err(e) = self.insert_texture_path(id.clone(), name, &bytes, memory) {
                        self.warnings.push(format!("Skipped {id}: {e}"));
                    }
                }
            }
        }
    }
    fn insert(&mut self, id: String, data: &[u8], memory: &mut usize) -> Result<(), String> {
        // Source limits are validated against mixed GPU page layout after loading.
        let frames = decode_png_frames(data)?;
        let bytes = frames.iter().map(|tile| tile.rgba.len()).sum::<usize>();
        // A replaced animation may contain dozens of frames, not merely
        // the first visible tile. Account for its entire old allocation
        // when canonical block/ supersedes legacy blocks/ within one pack.
        let prior = self.animations.get(&id).map_or_else(
            || self.tiles.get(&id).map_or(0, |tile| tile.rgba.len()),
            |animation| animation.frames.iter().map(|frame| frame.rgba.len()).sum(),
        );
        let next = memory
            .saturating_sub(prior)
            .checked_add(bytes)
            .ok_or("Texture memory overflow")?;
        if next > MAX_RGBA_MEMORY {
            return Err("Decoded texture memory exceeds the 512MiB safety budget".into());
        }
        *memory = next;
        self.layer_png.insert(id.clone());
        let tile = frames[0].clone();
        // A later replacement may be a static image. Remove any old
        // animation instead of playing frames from another source file.
        let animations = Arc::make_mut(&mut self.animations);
        animations.remove(&id);
        if frames.len() > 1 {
            animations.insert(
                id.clone(),
                SpriteAnimation {
                    frames,
                    schedule: vec![(0, 1)],
                    cycle: 1,
                },
            );
        }
        Arc::make_mut(&mut self.tiles).insert(id, tile);
        Ok(())
    }
    fn insert_colormap(&mut self, which: &str, data: &[u8]) -> Result<(), String> {
        let tile = decode_png(data)?;
        if tile.width != 256 || tile.height != 256 {
            return Err("Vanilla biome colormap must be 256x256".into());
        }
        match which {
            "grass" => self.grass_colormap = Some(tile),
            "foliage" => self.foliage_colormap = Some(tile),
            _ => return Err("Unknown biome colormap".into()),
        }
        Ok(())
    }
    fn sample_colormap(&self, biome: Option<&str>, which: &str) -> Option<[f32; 3]> {
        let (mut temperature, downfall) = biome_climate(biome?)?;
        temperature = temperature.clamp(0.0, 1.0);
        let humidity = (downfall * temperature).clamp(0.0, 1.0);
        let tile = if which == "grass" {
            self.grass_colormap.as_ref()?
        } else {
            self.foliage_colormap.as_ref()?
        };
        let rgba = tile.pixel(1.0 - temperature, 1.0 - humidity);
        Some([
            rgba[0] as f32 / 255.0,
            rgba[1] as f32 / 255.0,
            rgba[2] as f32 / 255.0,
        ])
    }
    /// Within one pack, a successfully decoded modern block/ PNG wins over
    /// the legacy blocks/ alias. Invalid PNG data must NOT reserve that
    /// identity: an intact legacy sprite (or a lower-priority pack) can still
    /// render the block instead of being silently replaced by missingno.
    fn accept_texture_path(&self, id: &str, path: &str) -> bool {
        path.contains("/textures/block/") || !self.modern_textures.contains(id)
    }
    fn insert_texture_path(
        &mut self,
        id: String,
        path: &str,
        data: &[u8],
        memory: &mut usize,
    ) -> Result<(), String> {
        self.insert(id.clone(), data, memory)?;
        if path.contains("/textures/block/") {
            self.modern_textures.insert(id);
        }
        Ok(())
    }
    fn accept_metadata_path(&mut self, id: &str, path: &str) -> bool {
        let modern = path.contains("/textures/block/");
        if modern {
            self.modern_metadata.insert(id.to_owned());
        }
        modern || !self.modern_metadata.contains(id)
    }
    fn walk(
        &mut self,
        root: &Path,
        folder: &Path,
        memory: &mut usize,
        depth: u8,
        entries: &mut usize,
    ) -> Result<(), String> {
        if depth > 20 {
            return Err("Resource-pack folder nesting over 20".into());
        }
        let metadata = match fs::symlink_metadata(folder) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        // The initial assets/ directory must obey the same rule as its
        // children; is_dir() alone would follow a linked outside tree.
        if metadata.file_type().is_symlink() {
            return Err("Linked resource-pack assets directory is refused".into());
        }
        if !metadata.is_dir() {
            return Ok(());
        }
        // Directory enumeration order is not portable. This also makes the
        // behavior of within-pack alias and model insertion deterministic.
        let mut items = fs::read_dir(folder)
            .map_err(|e| e.to_string())?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        items.sort_by_key(|item| item.file_name());
        for item in items {
            *entries += 1;
            if *entries > 250_000 {
                return Err("Resource pack exceeds 250,000 directory entries safety budget".into());
            }
            let ty = item.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                continue;
            }
            let path = item.path();
            if ty.is_dir() {
                self.walk(root, &path, memory, depth + 1, entries)?;
            } else if ty.is_file() {
                let Ok(relative) = path.strip_prefix(root) else {
                    continue;
                };
                let name = relative.to_string_lossy().replace('\\', "/");
                let Some(asset) = PackAsset::classify(&name) else {
                    continue;
                };
                let max_size = asset.max_bytes();
                let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
                if meta.len() > max_size {
                    self.warnings.push(format!("Skipped oversize {name}"));
                    continue;
                }
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                if bytes.len() as u64 > max_size {
                    self.warnings.push(format!("Skipped oversize {name}"));
                    continue;
                }
                self.ingest_asset(&name, asset, bytes, memory);
            }
        }
        Ok(())
    }
    fn read_zip<R: Read + Seek>(&mut self, r: R, memory: &mut usize) -> Result<(), String> {
        let mut archive = zip::ZipArchive::new(r).map_err(|e| e.to_string())?;
        if archive.len() > 1_000_000 {
            return Err("ZIP exceeds 1,000,000 entries safety budget".into());
        }
        for i in 0..archive.len() {
            let mut member = match archive.by_index(i) {
                Ok(member) => member,
                Err(error) => {
                    self.warnings.push(format!("Skipped unreadable ZIP entry {i}: {error}"));
                    continue;
                }
            };
            let name = member.name().to_string();
            let Some(asset) = PackAsset::classify(&name) else {
                continue;
            };
            let max_size = asset.max_bytes();
            if member.size() > max_size {
                self.warnings.push(format!("Skipped oversize {name}"));
                continue;
            }
            let mut bytes = Vec::new();
            if let Err(error) = member
                .by_ref()
                .take(max_size + 1)
                .read_to_end(&mut bytes) {
                self.warnings.push(format!("Skipped unreadable asset {name}: {error}"));
                continue;
            }
            if bytes.len() as u64 > max_size {
                self.warnings.push(format!("Skipped oversize {name}"));
                continue;
            }
            self.ingest_asset(&name, asset, bytes, memory);
        }
        Ok(())
    }
    /// Resource packs may place metadata before or after the PNG in ZIPs;
    /// resolve order-independently only after all namespaced assets load.
    fn bake_mtl_masks(&mut self){
        let declared=self.models.opacity_maps();
        if declared.len()>8192{
            self.warnings.push("Too many MTL opacity materials".into());
            return;
        }
        let mut memory:usize=self.tiles.values().map(|t|t.rgba.len()).sum();
        for (model,material,diffuse,mask) in declared{
            let (Some(diffuse_id),Some(mask_id))=(
                material_texture_id(&model,&diffuse),
                material_texture_id(&model,&mask)
            )else{
                self.warnings.push(format!("Invalid MTL texture for {material}"));
                continue;
            };
            let (Some(base),Some(alpha))=(
                self.tiles.get(&diffuse_id),self.tiles.get(&mask_id)
            )else{
                self.warnings.push(format!("Missing map_Kd/map_d for {material}"));
                continue;
            };
            let bytes=base.rgba.len();
            if memory.checked_add(bytes).is_none_or(|v|v>MAX_RGBA_MEMORY){
                self.warnings.push("MTL opacity maps exceed atlas memory budget".into());
                break;
            }
            let tile=combine_opacity(base,alpha);
            let synthetic=format!("mcedit2:mtl_opacity_{}",self.tiles.len());
            Arc::make_mut(&mut self.tiles).insert(synthetic.clone(),tile);
            Arc::make_mut(&mut self.models)
                .set_opacity_overlay(model,material,synthetic);
            memory+=bytes;
        }
    }
    fn finalize_animations(&mut self) {
        let source = std::mem::take(&mut self.animations);
        let candidates = Arc::try_unwrap(source).unwrap_or_else(|shared| (*shared).clone());
        self.static_animation_count = 0;
        for (id, candidate) in candidates {
            let Some(meta) = self.metadata.get(&id) else {
                continue;
            };
            match SpriteAnimation::from_metadata(candidate.frames, meta) {
                Ok(animation) => {
                    // Render only the first *scheduled* mcmeta frame; it may
                    // not be the PNG strip's first image. Free all later
                    // frames once the static tile has been copied.
                    let first = animation.tile_at(0).clone();
                    Arc::make_mut(&mut self.tiles).insert(id, first);
                    self.static_animation_count += 1;
                }
                Err(e) => self.warnings.push(format!("Skipped animation {id}: {e}")),
            }
        }
        self.metadata.clear();
        // No animation tick can ever rebuild the atlas in static mode.
        // Dropping the decoded strips is important for large Java client
        // archives and modded high-resolution resource packs.
        self.animations = Arc::new(BTreeMap::new());
    }
    #[cfg(test)]
    pub fn has_animation(&self) -> bool {
        !self.animations.is_empty()
    }
    pub fn animation_count(&self) -> usize {
        // Report how many animated source textures were frozen, not how
        // many transient animation frames are still retained in RAM.
        self.static_animation_count
    }
    /// Advance only if a frame actually changes. The source atlas UV slots,
    /// model JSON and texture data stay immutable between scene frames.
    #[cfg(test)]
    pub fn advance_animation(&self, ticks: u64) -> Option<Self> {
        if !self.animations.iter().any(|(name, animation)| {
            self.tiles.contains_key(name)
                && animation.index_at(self.animation_tick) != animation.index_at(ticks)
        }) {
            return None;
        }
        let mut next = self.clone();
        next.animation_tick = ticks;
        next.atlas = self
            .atlas
            .as_ref()
            .map(|atlas| Arc::new(atlas.advance(&self.animations, self.animation_tick, ticks)));
        Some(next)
    }
    /// A single pack uses the same path as ordered overlays.
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_stack(&[path.to_path_buf()])
    }

    /// Merge client and resource packs in increasing priority. Only source
    /// identity maps are overlaid; the GPU atlas is allocated once at the end.
    pub fn open_stack(paths: &[std::path::PathBuf]) -> Result<Self, String> {
        if paths.is_empty() || paths.iter().any(|p| p.as_os_str().is_empty()) {
            return Err("Select at least one resource-pack directory or ZIP".into());
        }
        // Large Prism/NeoForge modpacks may contain hundreds of active mod
        // archives. The atlas has separate decoded-RGBA and GPU memory caps;
        // the former 32-pack ceiling incorrectly excluded most mod assets.
        if paths.len() > 1024 {
            return Err("Resource-pack stack exceeds 1024 layers".into());
        }
        let mut out = Self {
            #[cfg(test)]
            source: paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(" -> "),
            ..Self::default()
        };
        let mut memory = 0;
        for path in paths {
            // Canonical block/ beats legacy blocks/ within one pack only;
            // a higher pack's legacy sprite must still replace base sprites.
            out.modern_textures.clear();
            out.modern_metadata.clear();
            out.layer_png.clear();
            out.layer_metadata.clear();
            if path.is_dir() {
                let root = path.canonicalize().map_err(|e| e.to_string())?;
                let mut entries = 0;
                out.walk(&root, &root.join("assets"), &mut memory, 0, &mut entries)?;
            } else if path.is_file() {
                if !path.extension().and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("zip")
                        || ext.eq_ignore_ascii_case("jar"))
                {
                    return Err(
                        "Only directories, .zip packs and .jar client assets are supported".into(),
                    );
                }
                out.read_zip(File::open(path).map_err(|e| e.to_string())?, &mut memory)?;
            } else {
                return Err(format!("Resource-pack layer does not exist: {}", path.display()));
            }
            // A new static image must never inherit lower-priority animated
            // PNG metadata. Metadata-only overlays are deliberately allowed.
            for id in &out.layer_png {
                if !out.layer_metadata.contains(id) {
                    out.metadata.remove(id);
                }
            }
        }
        if out.tiles.is_empty() {
            return Err("Packs contain no supported assets/*/textures/block/*.png".into());
        }
        Arc::make_mut(&mut out.tiles).insert(MISSING_TEXTURE_ID.into(), missing_texture());
        // Resolve pack-local external .bin references only after all ZIP/JAR
        // entries and overlay layers have been added, independent of order.
        out.warnings.extend(Arc::make_mut(&mut out.models).finish_gltf());
        out.finalize_animations();
        out.bake_mtl_masks();
        let chosen = mixed_layout(&out.tiles)?;
        let source_max = out
            .tiles
            .values()
            .map(|tile| tile.width.max(tile.height))
            .max()
            .unwrap_or(16);
        if source_max > chosen.max_cell {
            out.warnings.push(format!(
                "GPU atlas downsampled block sprites from {source_max}px to {}px to fit memory",
                chosen.max_cell
            ));
        }
        out.atlas = Some(Arc::new(TextureAtlas::create_at(
            &out.tiles,
            &out.animations,
            0,
        )));
        Ok(out)
    }
    pub fn count(&self) -> usize {
        self.tiles.len()
    }
    pub fn atlas(&self) -> Option<&std::sync::Arc<TextureAtlas>> {
        self.atlas.as_ref()
    }
    /// Sample the same packed PNG alpha bytes used by the GPU. The hit test
    /// only uses it as a conservative pixel mask; no world data is mutated.
    pub fn atlas_pixel(&self, uv: [f32; 2], page: u32) -> Option<[u8; 4]> {
        let page = self.atlas.as_ref()?.page(page)?;
        if !uv[0].is_finite() || !uv[1].is_finite() {
            return None;
        }
        let x = (uv[0].clamp(0.0, 0.999_999) * page.width as f32) as usize;
        let y = (uv[1].clamp(0.0, 0.999_999) * page.height as f32) as usize;
        let at = (y * page.width as usize + x) * 4;
        let rgba = page.rgba.get(at..at + 4)?;
        Some([rgba[0], rgba[1], rgba[2], rgba[3]])
    }
    pub fn atlas_size(&self) -> Option<(u32, u32)> {
        self.atlas
            .as_ref()?
            .pages
            .first()
            .map(|a| (a.width, a.height))
    }
    /// Resolve GPU UV and material alpha once per source state/face/element.
    /// The fallback may use old per-block named textures, but never another
    /// namespace with a coincidentally matching resource path.
    pub fn element_unshaded_at(
        &self,
        state: &State,
        element_index: usize,
        pos: (i32, i32, i32),
    ) -> Option<bool> {
        self.models.element_unshaded_at(state, element_index, pos)
    }
    pub fn face_cull_direction_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element_index: usize,
        pos: (i32, i32, i32),
    ) -> Option<Option<crate::isometric::Face>> {
        self.models
            .face_cull_direction_at(state, face, element_index, pos)
    }
    pub fn face_defined_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element_index: usize,
        pos: (i32, i32, i32),
    ) -> Option<bool> {
        self.models.face_defined_at(state, face, element_index, pos)
    }
    #[cfg(test)]
    pub fn atlas_face_style(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element_index: usize,
    ) -> Option<([[f32; 2]; 4], AlphaMode, u32)> {
        self.atlas_face_style_at(state, face, element_index, (0, 0, 0))
    }
    pub fn atlas_face_style_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element_index: usize,
        pos: (i32, i32, i32),
    ) -> Option<([[f32; 2]; 4], AlphaMode, u32)> {
        let spec = self.models.element_face_at(state, face, element_index, pos);
        let (id, uv, rot, perm) = match spec {
            Some(spec) => {
                let id = if self.tiles.contains_key(&spec.texture) {
                    spec.texture
                } else {
                    MISSING_TEXTURE_ID.to_string()
                };
                (id, spec.uv, spec.rotation, spec.uvlock)
            }
            None => {
                // Missing JSON faces are geometry, not missing textures.
                if self.models.face_defined_at(state, face, element_index, pos) == Some(false) {
                    return None;
                }
                (
                    self.resolve_id_at(state, face, pos)
                        .unwrap_or_else(|| MISSING_TEXTURE_ID.into()),
                    [0.0, 0.0, 16.0, 16.0],
                    0,
                    [0, 1, 2, 3],
                )
            }
        };
        let region = self.atlas.as_ref()?.entries.get(&id)?;
        let alpha = self
            .animations
            .get(&id)
            .map(SpriteAnimation::alpha_mode)
            .unwrap_or(self.tiles.get(&id)?.alpha_mode());
        Some((
            orient_uv(region.corners_for(uv, rot), perm),
            alpha,
            region.page,
        ))
    }
    /// Raw source UVs for arbitrary JSON models, before atlas clipping.
    /// Generic static tiling is independent of individual block identities.
    pub fn atlas_repeating_face_at(
        &self, state: &State, face: crate::isometric::Face,
        element_index: usize, pos: (i32, i32, i32),
    ) -> Option<([[f32; 2]; 4], AtlasUv)> {
        let spec = self.models.element_face_at(state, face, element_index, pos)?;
        if spec.uv.iter().all(|v| (0.0..=16.0).contains(v)) {
            return None;
        }
        let id = if self.tiles.contains_key(&spec.texture) {
            spec.texture.as_str()
        } else {
            MISSING_TEXTURE_ID
        };
        let atlas = self.atlas.as_ref()?.entries.get(id)?.clone();
        let [u0, v0, u1, v1] = spec.uv;
        let mut corners = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
        for _ in 0..(spec.rotation / 90) {
            corners.rotate_right(1);
        }
        Some((orient_uv(corners, spec.uvlock), atlas))
    }
    /// Deterministic vanilla biome color approximations for the verified source
    /// biome ID. This remains high-gamma/fullbright: RGB tint never adds shadows.
    /// Custom/modded biomes use neutral vanilla colors until their registry
    /// climate/colormap definitions are available.
    /// Vanilla block tint is an independent multiplier over the texture.
    /// Until original biome sampling is wired in, use documented neutral
    /// grass/foliage defaults; un-tinted faces remain at full brightness.
    #[cfg(test)]
    pub fn face_tint_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element: usize,
        pos: (i32, i32, i32),
    ) -> [f32; 3] {
        self.face_tint_with_biome_at(state, face, element, pos, None)
    }
    pub fn face_tint_with_biome_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element: usize,
        pos: (i32, i32, i32),
        biome: Option<&str>,
    ) -> [f32; 3] {
        // Vanilla fluids are rendered by a built-in fluid renderer and do
        // not necessarily have block-model tintindex entries. Respect the
        // resource sprite itself: missing textures must remain purple/black.
        if state.name == "minecraft:water" {
            return if self.resolve_id_at(state, face, pos).is_some() {
                biome_color(biome, "water")
            } else {
                [1.0; 3]
            };
        }
        let spec = self.models.element_face_at(state, face, element, pos);
        if let Some(spec) = spec {
            if spec.tint_index.is_none() || !self.tiles.contains_key(&spec.texture) {
                return [1.0; 3];
            }
        } else {
            // The original game can load block textures while a lightweight
            // override pack omits their JSON parents. Only known vanilla
            // biome-tinted blocks get a conservative tint in this case.
            // Explicit JSON faces without tintindex stay untinted, and the
            // purple/black missing-texture placeholder is NEVER recolored.
            if self
                .models
                .face_defined_at(state, face, element, pos)
                .is_some()
            {
                return [1.0; 3];
            }
            let Some(id) = self.resolve_id_at(state, face, pos) else {
                return [1.0; 3];
            };
            if !self.tiles.contains_key(&id) || !known_vanilla_biome_tint(state, face) {
                return [1.0; 3];
            }
        }
        let name = state.name.as_str();
        if name.contains("spruce_leaves") {
            [0.38, 0.60, 0.38]
        } else if name.contains("birch_leaves") {
            [0.50, 0.65, 0.33]
        } else if name.contains("leaves") || name.contains("vine") {
            self.sample_colormap(biome, "foliage")
                .unwrap_or_else(|| biome_color(biome, "foliage"))
        } else if name.contains("water") {
            biome_color(biome, "water")
        } else {
            self.sample_colormap(biome, "grass")
                .unwrap_or_else(|| biome_color(biome, "grass"))
        }
    }
    #[cfg(test)]
    pub fn alpha_mode(&self, state: &State, face: crate::isometric::Face) -> Option<AlphaMode> {
        let id = self.resolve_id(state, face)?;
        self.animations
            .get(&id)
            .map(SpriteAnimation::alpha_mode)
            .or_else(|| self.tiles.get(&id).map(Tile::alpha_mode))
    }
    #[cfg(test)]
    pub fn atlas_face_corners(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element_index: usize,
    ) -> Option<[[f32; 2]; 4]> {
        self.atlas_face_style_at(state, face, element_index, (0, 0, 0))
            .map(|(corners, _, _)| corners)
    }
    #[cfg(test)]
    pub fn atlas_uv(&self, state: &State, face: crate::isometric::Face) -> Option<AtlasUv> {
        let id = self.resolve_id(state, face)?;
        self.atlas.as_ref()?.entries.get(&id).cloned()
    }
    /// Every declared OBJ material maps through the same namespace-safe
    /// atlas lookup as JSON faces, without any block ID special cases.
    pub fn static_obj_at(
        &self,
        state: &State,
        pos: (i32, i32, i32),
    ) -> Option<Vec<StaticObjTriangle>> {
        let chosen = self.models.obj_at(state, pos)?;
        let atlas = self.atlas.as_ref()?;
        let mut out = Vec::new();
        for tri in chosen {
            let id = if self.tiles.contains_key(&tri.texture) {
                tri.texture.as_str()
            } else {
                MISSING_TEXTURE_ID
            };
            let slot = atlas.entries.get(id)?;
            let alpha = if tri.tint[3] < 0.999 {
                AlphaMode::Blend
            } else {
                self.tiles.get(id)?.alpha_mode()
            };
            let mode = crate::material_mode::encode(
                alpha as u32, tri.alpha_mode.as_deref(), tri.alpha_cutoff, tri.double_sided,
            );
            if !crate::static_triangle_uv::tile(tri.xyz, tri.uv, slot, |xyz, uv| {
                if out.len() < 4096 {
                    out.push(StaticObjTriangle {
                        xyz,
                        uv,
                        page: slot.page,
                        mode,
                        tint: tri.tint,
                    });
                }
            }) {
                return None;
            }
        }
        Some(out)
    }
    /// Stable model/tile identity across animated RGBA atlas frame updates.
    pub fn geometry_id(&self) -> (usize, usize) {
        (
            Arc::as_ptr(&self.models) as usize,
            Arc::as_ptr(&self.tiles) as usize,
        )
    }
    pub fn model_count(&self) -> usize {
        self.models.count()
    }
    pub fn geometry(&self, state: &State) -> Option<Vec<crate::models::Part>> {
        self.geometry_at(state, (0, 0, 0))
    }
    pub fn geometry_at(
        &self,
        state: &State,
        pos: (i32, i32, i32),
    ) -> Option<Vec<crate::models::Part>> {
        self.models.parts_at(state, pos)
    }
    pub fn model_variant_key(&self, state: &State, pos: (i32, i32, i32)) -> u64 {
        self.models.weighted_selection_key(state, pos)
    }
    #[cfg(test)]
    pub fn get(&self, id: &str) -> Option<&Tile> {
        self.tiles.get(id)
    }
    #[cfg(test)]
    pub fn resolve_id(&self, state: &State, face: crate::isometric::Face) -> Option<String> {
        self.resolve_id_at(state, face, (0, 0, 0))
    }
    pub fn resolve_id_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        pos: (i32, i32, i32),
    ) -> Option<String> {
        let (ns, name) = state.name.split_once(':')?;
        if ns == "numeric" {
            return None;
        }
        if let Some(key) = self.models.texture_at(state, face, pos) {
            if self.tiles.contains_key(&key) {
                return Some(key);
            }
        }
        let top = face == crate::isometric::Face::Top;
        let bottom = face == crate::isometric::Face::Bottom;
        let mut candidates = Vec::with_capacity(6);
        if name == "grass_block" {
            candidates.push(
                if top {
                    "grass_block_top"
                } else if bottom {
                    "dirt"
                } else {
                    "grass_block_side"
                }
                .to_string(),
            );
        } else if name == "mycelium" {
            candidates.push(
                if top {
                    "mycelium_top"
                } else if bottom {
                    "dirt"
                } else {
                    "mycelium_side"
                }
                .into(),
            );
        } else if name == "podzol" {
            candidates.push(
                if top {
                    "podzol_top"
                } else if bottom {
                    "dirt"
                } else {
                    "podzol_side"
                }
                .into(),
            );
        } else if name.ends_with("_log") || name.ends_with("_stem") || name.ends_with("_wood") {
            let axis = state
                .properties
                .iter()
                .find(|(k, _)| k == "axis")
                .map(|(_, v)| v.as_str())
                .unwrap_or("y");
            let end = match axis {
                "x" => matches!(
                    face,
                    crate::isometric::Face::PosX | crate::isometric::Face::NegX
                ),
                "z" => matches!(
                    face,
                    crate::isometric::Face::PosZ | crate::isometric::Face::NegZ
                ),
                _ => top || bottom,
            };
            if end {
                candidates.push(format!("{name}_top"));
            }
            candidates.push(name.to_owned());
        } else if name == "water" || name == "lava" {
            if top || bottom {
                candidates.push(format!("{name}_still"));
                candidates.push(format!("{name}_flow"));
            } else {
                candidates.push(format!("{name}_flow"));
                candidates.push(format!("{name}_still"));
            }
        } else if name == "sandstone" {
            candidates.push(
                if top {
                    "sandstone_top"
                } else if bottom {
                    "sandstone_bottom"
                } else {
                    "sandstone"
                }
                .into(),
            );
        }
        candidates.push(name.to_owned());
        // Namespace is retained: never project one mod's asset onto another.
        for key in candidates {
            let id = format!("{ns}:{key}");
            if self.tiles.contains_key(&id) {
                return Some(id);
            }
        }
        None
    }
    #[cfg(test)]
    pub fn resolve<'a>(&'a self, state: &State, face: crate::isometric::Face) -> Option<&'a Tile> {
        self.get(&self.resolve_id(state, face)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn texture_bank_retains_clone_debug_and_default_contracts() {
        fn require_traits<T: Clone + std::fmt::Debug + Default>() {}
        require_traits::<TextureBank>();
        let bank = TextureBank::default();
        let copy = bank.clone();
        assert_eq!(copy.source, "");
        assert!(copy.atlas.is_none());
        assert!(format!("{copy:?}").contains("TextureBank"));
    }
    use crate::isometric::Face;
    fn state(name: &str, properties: Vec<(String, String)>) -> State {
        State {
            name: name.into(),
            properties,
        }
    }
    fn solid(color: [u8; 4]) -> Tile {
        Tile {
            width: 1,
            height: 1,
            rgba: color.to_vec(),
        }
    }
    #[test]
    fn vanilla_colormap_samples_registered_climate_and_skips_unknown_mods() {
        let mut bank = TextureBank::default();
        bank.grass_colormap = Some(Tile {
            width: 256,
            height: 256,
            rgba: vec![12, 84, 192, 255].repeat(256 * 256),
        });
        assert_eq!(
            bank.sample_colormap(Some("minecraft:plains"), "grass"),
            Some([12.0 / 255.0, 84.0 / 255.0, 192.0 / 255.0])
        );
        assert_eq!(bank.sample_colormap(Some("mod:unknown"), "grass"), None);
        assert_eq!(
            bank.sample_colormap(Some("minecraft:plains"), "foliage"),
            None
        );
    }
    #[test]
    fn grass_and_water_tint_use_original_biome_name() {
        let swamp = biome_color(Some("minecraft:swamp"), "water");
        let ocean = biome_color(Some("minecraft:deep_cold_ocean"), "water");
        let unknown = biome_color(Some("other:unknown"), "water");
        assert_ne!(swamp, ocean);
        assert_eq!(unknown, biome_color(None, "water"));
        assert_ne!(
            biome_color(Some("minecraft:badlands"), "grass"),
            biome_color(Some("minecraft:jungle"), "grass")
        );
    }
    #[test]
    fn vanilla_vegetation_without_json_parent_uses_only_real_sprite_tints() {
        let mut bank = TextureBank::default();
        let top = state("minecraft:grass_block", vec![]);
        let fern = state("minecraft:fern", vec![]);
        let leaf = state("minecraft:oak_leaves", vec![]);
        assert_eq!(
            bank.face_tint_with_biome_at(&leaf, Face::Top, 0, (1, 70, 1), Some("minecraft:swamp")),
            [1.0; 3]
        ); // missing sprite stays purple
        for name in ["grass_block_top", "grass_block_side", "fern", "oak_leaves"] {
            Arc::make_mut(&mut bank.tiles)
                .insert(format!("minecraft:{name}"), solid([127, 127, 127, 255]));
        }
        assert_eq!(
            bank.face_tint_with_biome_at(&top, Face::Top, 0, (1, 70, 1), Some("minecraft:swamp")),
            biome_color(Some("minecraft:swamp"), "grass")
        );
        assert_eq!(
            bank.face_tint_with_biome_at(&top, Face::PosX, 0, (1, 70, 1), Some("minecraft:swamp")),
            [1.0; 3]
        );
        assert_eq!(
            bank.face_tint_with_biome_at(&fern, Face::Top, 0, (1, 70, 1), Some("minecraft:jungle")),
            biome_color(Some("minecraft:jungle"), "grass")
        );
        assert_eq!(
            bank.face_tint_with_biome_at(&leaf, Face::Top, 0, (1, 70, 1), Some("minecraft:swamp")),
            biome_color(Some("minecraft:swamp"), "foliage")
        );
        let cherry = state("minecraft:cherry_leaves", vec![]);
        Arc::make_mut(&mut bank.tiles).insert(
            "minecraft:cherry_leaves".into(),
            solid([127, 127, 127, 255]),
        );
        assert_eq!(
            bank.face_tint_with_biome_at(
                &cherry,
                Face::Top,
                0,
                (1, 70, 1),
                Some("minecraft:swamp")
            ),
            [1.0; 3]
        );
        let modded = state("custom:oak_leaves", vec![]);
        Arc::make_mut(&mut bank.tiles)
            .insert("custom:oak_leaves".into(), solid([127, 127, 127, 255]));
        assert_eq!(
            bank.face_tint_with_biome_at(
                &modded,
                Face::Top,
                0,
                (1, 70, 1),
                Some("minecraft:swamp")
            ),
            [1.0; 3]
        );
    }
    #[test]
    fn water_tint_respects_available_original_sprite_and_missing_checker() {
        let mut bank = TextureBank::default();
        let water = state("minecraft:water", vec![]);
        assert_eq!(
            bank.face_tint_at(&water, Face::Top, 0, (0, 60, 0)),
            [1.0; 3]
        );
        Arc::make_mut(&mut bank.tiles)
            .insert("minecraft:water_still".into(), solid([140, 140, 140, 255]));
        assert_eq!(
            bank.face_tint_at(&water, Face::Top, 0, (0, 60, 0)),
            biome_color(None, "water")
        );
        let lava = state("minecraft:lava", vec![]);
        assert_eq!(bank.face_tint_at(&lava, Face::Top, 0, (0, 60, 0)), [1.0; 3]);
    }
    #[test]
    fn fluid_top_and_side_use_distinct_original_still_and_flow_sprites() {
        let mut bank = TextureBank::default();
        let tiles = Arc::make_mut(&mut bank.tiles);
        tiles.insert("minecraft:water_still".into(), solid([0, 90, 210, 255]));
        tiles.insert("minecraft:water_flow".into(), solid([0, 130, 240, 255]));
        let water = state("minecraft:water", vec![("level".into(), "4".into())]);
        assert_eq!(
            bank.resolve_id(&water, Face::Top),
            Some("minecraft:water_still".into())
        );
        assert_eq!(
            bank.resolve_id(&water, Face::PosX),
            Some("minecraft:water_flow".into())
        );
    }
    #[test]
    fn minecraft_animation_metadata_uses_custom_order_and_tick_lengths() {
        let frames = vec![
            solid([255, 0, 0, 255]),
            solid([0, 255, 0, 255]),
            solid([0, 0, 255, 255]),
        ];
        let animation = SpriteAnimation::from_metadata(
            frames,
            br#"{"animation":{"frametime":2,"frames":[{"index":2,"time":3},0,1]}}"#,
        )
        .unwrap();
        assert_eq!(animation.cycle, 7);
        let chosen = (0..9)
            .map(|tick| animation.index_at(tick))
            .collect::<Vec<_>>();
        assert_eq!(chosen, vec![2, 2, 2, 0, 0, 1, 1, 2, 2]);
        assert!(SpriteAnimation::from_metadata(
            vec![solid([0, 0, 0, 255]); 2],
            br#"{"animation":{"frames":[5]}}"#
        )
        .is_err());
        assert!(SpriteAnimation::from_metadata(
            vec![solid([0, 0, 0, 255]); 2],
            br#"{"animation":{"frametime":0}}"#
        )
        .is_err());
    }
    #[test]
    fn animated_atlas_updates_gpu_and_canvas_source_from_one_tick() {
        let mut bank = TextureBank::default();
        let a = solid([3, 14, 159, 255]);
        let b = solid([230, 100, 6, 128]);
        Arc::make_mut(&mut bank.tiles).insert("minecraft:water_still".into(), a.clone());
        Arc::make_mut(&mut bank.animations).insert(
            "minecraft:water_still".into(),
            SpriteAnimation {
                frames: vec![a, b],
                schedule: vec![(0, 2), (1, 1)],
                cycle: 3,
            },
        );
        bank.atlas = Some(Arc::new(TextureAtlas::create_at(
            &bank.tiles,
            &bank.animations,
            0,
        )));
        let uv = bank.atlas.as_ref().unwrap().entries["minecraft:water_still"].sample(0.5, 0.5);
        assert_eq!(bank.atlas_pixel(uv, 0), Some([3, 14, 159, 255]));
        assert!(bank.advance_animation(1).is_none());
        let next = bank.advance_animation(2).unwrap();
        assert_eq!(next.atlas_pixel(uv, 0), Some([230, 100, 6, 128]));
        assert_eq!(bank.atlas_pixel(uv, 0), Some([3, 14, 159, 255]));
        assert!(Arc::ptr_eq(&bank.tiles, &next.tiles));
        assert!(Arc::ptr_eq(&bank.models, &next.models));
        assert!(Arc::ptr_eq(&bank.animations, &next.animations));
    }
    #[test]
    fn meta_file_order_is_independent_of_image_loading() {
        let mut bank = TextureBank::default();
        let id = "minecraft:lava_still".to_string();
        bank.metadata
            .insert(id.clone(), br#"{"animation":{"frametime":3}}"#.to_vec());
        let source = vec![solid([6, 12, 18, 255]), solid([24, 30, 36, 255])];
        Arc::make_mut(&mut bank.tiles).insert(id.clone(), source[0].clone());
        Arc::make_mut(&mut bank.animations).insert(
            id.clone(),
            SpriteAnimation {
                frames: source,
                schedule: vec![(0, 1)],
                cycle: 1,
            },
        );
        bank.finalize_animations();
        assert_eq!(bank.animation_count(), 1);
        assert!(bank.metadata.is_empty());
        assert!(bank.animations.is_empty());
        assert!(!bank.has_animation());
        assert!(bank.advance_animation(3).is_none());
        assert_eq!(bank.tiles[&id].rgba, [6, 12, 18, 255]);
    }
    #[test]
    fn static_source_frame_obeys_mcmeta_nonzero_first_index() {
        let mut bank = TextureBank::default();
        let id = "minecraft:water_still".to_owned();
        let frames = vec![
            solid([20, 40, 60, 255]),
            solid([200, 100, 50, 125]),
            solid([80, 120, 210, 255]),
        ];
        Arc::make_mut(&mut bank.tiles).insert(id.clone(), frames[0].clone());
        Arc::make_mut(&mut bank.animations).insert(
            id.clone(),
            SpriteAnimation {
                frames,
                schedule: vec![(0, 1)],
                cycle: 1,
            },
        );
        bank.metadata
            .insert(id.clone(), br#"{"animation":{"frames":[2,1,0]}}"#.to_vec());
        bank.finalize_animations();
        assert_eq!(bank.tiles[&id].rgba, [80, 120, 210, 255]);
        let atlas = TextureAtlas::create_at(&bank.tiles, &bank.animations, 0);
        let uv = atlas.entries[&id].sample(0.5, 0.5);
        let page = atlas.page(atlas.entries[&id].page).unwrap();
        let at = (((uv[1] * page.height as f32) as usize * page.width as usize)
            + (uv[0] * page.width as f32) as usize)
            * 4;
        assert_eq!(&page.rgba[at..at + 4], &[80, 120, 210, 255]);
        assert_eq!(bank.animation_count(), 1);
        assert!(bank.animations.is_empty());
        assert!(!bank.has_animation());
        assert_eq!(bank.tiles[&id].alpha_mode(), AlphaMode::Opaque);
        assert!(bank.advance_animation(1).is_none());
    }
    #[test]
    fn cutout_atlas_avoids_black_filter_fringe_without_changing_alpha() {
        let mut tiles = BTreeMap::new();
        tiles.insert(
            "test:leaves".into(),
            Tile {
                width: 2,
                height: 1,
                rgba: vec![83, 183, 61, 255, 0, 0, 0, 0],
            },
        );
        let atlas = TextureAtlas::create(&tiles);
        let uv = atlas.entries.get("test:leaves").unwrap();
        let page = atlas.page(uv.page).unwrap();
        let point = uv.sample(0.53, 0.5);
        let x = (point[0] * page.width as f32) as usize;
        let y = (point[1] * page.height as f32) as usize;
        let at = (y * page.width as usize + x) * 4;
        assert_eq!(&page.rgba[at..at + 4], [83, 183, 61, 0]);
        assert_eq!(&tiles["test:leaves"].rgba[4..8], [0, 0, 0, 0]);
        let far = uv.sample(0.95, 0.5);
        let fx = (far[0] * page.width as f32) as usize;
        let fy = (far[1] * page.height as f32) as usize;
        assert_eq!(page.rgba[(fy * page.width as usize + fx) * 4 + 3], 0);
    }
    #[test]
    fn model_face_without_texture_cannot_borrow_another_named_side() {
        let bank = TextureBank::default();
        let state = state("minecraft:stone", vec![]);
        // Without an atlas loaded, neither fallback texture nor alternate
        // source face can be sampled.
        assert!(bank
            .atlas_face_style_at(&state, crate::isometric::Face::Top, 0, (0, 0, 0))
            .is_none());
        assert!(bank
            .atlas_face_corners(&state, crate::isometric::Face::Top, 0)
            .is_none());
    }
    #[test]
    fn missing_named_block_renders_checker_atlas_and_missing_json_face_stays_absent() {
        use crate::isometric::Face;
        let mut bank = TextureBank::default();
        let mut tiles = BTreeMap::new();
        tiles.insert(MISSING_TEXTURE_ID.to_string(), missing_texture());
        bank.atlas = Some(Arc::new(TextureAtlas::create(&tiles)));
        bank.tiles = Arc::new(tiles);
        let state = state("minecraft:unknown_block", vec![]);
        let (_, mode, page) = bank
            .atlas_face_style_at(&state, Face::Top, 0, (0, 0, 0))
            .expect("unknown block should use missing texture");
        assert_eq!(mode, AlphaMode::Opaque);
        assert!(bank.atlas().unwrap().page(page).is_some());
    }
    #[test]
    fn missing_texture_is_opaque_minecraft_purple_black_checker() {
        let tile = missing_texture();
        assert_eq!(tile.alpha_mode(), AlphaMode::Opaque);
        assert_eq!(tile.pixel(0.1, 0.1), [248, 0, 248, 255]);
        assert_eq!(tile.pixel(0.9, 0.1), [0, 0, 0, 255]);
        assert_eq!(tile.pixel(0.1, 0.9), [0, 0, 0, 255]);
        assert_eq!(tile.pixel(0.9, 0.9), [248, 0, 248, 255]);
    }
    #[test]
    fn original_glass_cutout_and_translucent_stained_glass_remain_distinct() {
        use mcedit2_world::surface::VoxelMaterial;
        let clear = state("minecraft:glass", vec![]);
        let stained = state("minecraft:blue_stained_glass", vec![]);
        assert_eq!(
            classify_block_alpha(&clear, VoxelMaterial::Transparent, AlphaMode::Cutout),
            (AlphaMode::Cutout, 1.0)
        );
        assert_eq!(
            classify_block_alpha(&stained, VoxelMaterial::Transparent, AlphaMode::Blend),
            (AlphaMode::Blend, 1.0)
        );
        let ice = state("minecraft:ice", vec![]);
        assert_eq!(
            classify_block_alpha(&ice, VoxelMaterial::Transparent, AlphaMode::Opaque),
            (AlphaMode::Blend, 0.72)
        );
    }
    #[test]
    fn leaves_cutout_is_not_forced_into_transparent_blending() {
        use mcedit2_world::surface::VoxelMaterial;
        let state = State {
            name: "minecraft:oak_leaves".into(),
            properties: vec![],
        };
        assert_eq!(
            classify_block_alpha(&state, VoxelMaterial::Foliage, AlphaMode::Cutout),
            (AlphaMode::Cutout, 1.0)
        );
    }
    #[test]
    fn modern_texture_and_metadata_paths_take_priority_regardless_of_order() {
        let mut bank = TextureBank::default();
        let id = "minecraft:stone";
        assert!(bank.accept_texture_path(id, "assets/minecraft/textures/blocks/stone.png"));
        assert!(bank.accept_texture_path(id, "assets/minecraft/textures/block/stone.png"));
        // Classification alone cannot reserve the modern identity: the
        // candidate's PNG may be malformed. A successful insert marks it.
        assert!(bank.accept_texture_path(id, "assets/minecraft/textures/blocks/stone.png"));
        bank.modern_textures.insert(id.into());
        assert!(!bank.accept_texture_path(id, "assets/minecraft/textures/blocks/stone.png"));
        assert!(bank.accept_metadata_path(id, "assets/minecraft/textures/blocks/stone.png.mcmeta"));
        assert!(bank.accept_metadata_path(id, "assets/minecraft/textures/block/stone.png.mcmeta"));
        assert!(!bank.accept_metadata_path(id, "assets/minecraft/textures/blocks/stone.png.mcmeta"));
    }
    #[test]
    fn modern_and_pre_flattening_vanilla_block_paths_share_identity() {
        assert_eq!(
            texture_id("assets/minecraft/textures/block/stone.png"),
            Some("minecraft:stone".into())
        );
        assert_eq!(
            texture_id("assets/minecraft/textures/blocks/stone.png"),
            Some("minecraft:stone".into())
        );
        assert_eq!(
            texture_id("assets/minecraft/textures/item/stone.png"),
            Some("minecraft:item/stone".into())
        );
    }
    #[test]
    fn atlas_packs_sorted_ids_without_cross_tile_color_bleed() {
        let mut tiles = BTreeMap::new();
        tiles.insert("a:first".into(), solid([255, 0, 0, 255]));
        tiles.insert("b:second".into(), solid([0, 0, 255, 128]));
        let atlas = TextureAtlas::create(&tiles);
        assert!(atlas.pages[0].width >= MIN_ATLAS_CELL + 2);
        assert!(atlas.pages[0].height >= MIN_ATLAS_CELL + 2);
        assert!(atlas
            .pages
            .iter()
            .all(|p| p.width == atlas.pages[0].width && p.height == atlas.pages[0].height));
        let one = &atlas.entries["a:first"];
        let two = &atlas.entries["b:second"];
        let sample = |entry: &AtlasUv| {
            let page = atlas.page(entry.page).unwrap();
            let uv = entry.sample(0.5, 0.5);
            let x = (uv[0] * page.width as f32) as usize;
            let y = (uv[1] * page.height as f32) as usize;
            page.rgba[(y * page.width as usize + x) * 4..][..4].to_vec()
        };
        assert_eq!(sample(one), vec![255, 0, 0, 255]);
        assert_eq!(sample(two), vec![0, 0, 255, 128]);
        assert_eq!(sample(&atlas.white), vec![255, 255, 255, 255]);
    }
    #[test]
    fn atlas_alpha_sampling_retains_transparent_holes() {
        let mut bank = TextureBank::default();
        Arc::make_mut(&mut bank.tiles).insert(
            "test:bars".into(),
            Tile {
                width: 2,
                height: 1,
                rgba: vec![255, 255, 255, 255, 0, 0, 0, 0],
            },
        );
        bank.atlas = Some(std::sync::Arc::new(TextureAtlas::create(&bank.tiles)));
        let rect = bank
            .atlas
            .as_ref()
            .unwrap()
            .entries
            .get("test:bars")
            .unwrap();
        let solid = rect.sample(0.2, 0.5);
        let hole = rect.sample(0.8, 0.5);
        assert_eq!(bank.atlas_pixel(solid, rect.page).unwrap()[3], 255);
        assert_eq!(bank.atlas_pixel(hole, rect.page).unwrap()[3], 0);
        assert_eq!(bank.atlas_pixel([f32::NAN, 0.2], 0), None);
    }
    #[test]
    fn alpha_modes_distinguish_cutout_from_true_translucency() {
        assert_eq!(
            AlphaMode::from_pixels(&[1, 2, 3, 255, 0, 0, 0, 255]),
            AlphaMode::Opaque
        );
        assert_eq!(
            AlphaMode::from_pixels(&[0, 0, 0, 0, 1, 2, 3, 255]),
            AlphaMode::Cutout
        );
        assert_eq!(
            AlphaMode::from_pixels(&[0, 0, 0, 0, 1, 2, 3, 128]),
            AlphaMode::Blend
        );
        assert_eq!(AlphaMode::from_pixels(&[0, 0, 0, 0]), AlphaMode::Cutout);
    }
    #[test]
    fn one_pass_gpu_face_style_preserves_cutout_and_uv_coordinates() {
        let mut bank = TextureBank::default();
        Arc::make_mut(&mut bank.tiles).insert(
            "test:casing".into(),
            Tile {
                width: 2,
                height: 1,
                rgba: vec![10, 20, 30, 255, 20, 30, 40, 0],
            },
        );
        bank.atlas = Some(std::sync::Arc::new(TextureAtlas::create(&bank.tiles)));
        let state = state("test:casing", vec![]);
        let (uv, alpha, page) = bank.atlas_face_style(&state, Face::Top, 0).unwrap();
        assert_eq!(page, 0);
        assert_eq!(alpha, AlphaMode::Cutout);
        let first = bank.atlas_uv(&state, Face::Top).unwrap();
        assert_eq!(uv[0], first.start);
    }
    #[test]
    fn uvlock_handles_corner_reflection_as_well_as_rotation() {
        let slot = AtlasUv {
            start: [0.2, 0.3],
            end: [0.4, 0.5],
            page: 0,
        };
        let uv = slot.corners_for([4.0, 0.0, 12.0, 16.0], 0);
        let mirrored = orient_uv(uv, [1, 0, 3, 2]);
        assert_eq!(mirrored[0], uv[1]);
        assert_eq!(mirrored[1], uv[0]);
        assert_eq!(mirrored[3], uv[2]);
        assert!(mirrored.iter().flatten().all(|x| x.is_finite()));
    }
    #[test]
    fn per_face_uv_crop_and_rotation_stay_inside_atlas_slot() {
        let slot = AtlasUv {
            start: [0.2, 0.3],
            end: [0.4, 0.5],
            page: 0,
        };
        let straight = slot.corners_for([4.0, 0.0, 12.0, 16.0], 0);
        let turned = slot.corners_for([4.0, 0.0, 12.0, 16.0], 90);
        assert_eq!(straight[0], [0.25, 0.3]);
        assert_eq!(turned[1], straight[0]);
        assert!((straight[2][0] - 0.35).abs() < 1e-6);
    }
    #[test]
    fn variable_resolution_paging_preserves_hd_source_tiles() {
        let small = page_plan(100, 16).unwrap();
        let hd = page_plan(100, 128).unwrap();
        assert!(small.width < hd.width);
        assert_eq!(hd.cell, 128);
        assert!(page_plan(1_000_000, 16).is_err());
        let mut tiles = BTreeMap::new();
        for i in 0..15_000 {
            tiles.insert(
                format!("mod:block_{i}"),
                Tile {
                    width: 1,
                    height: 1,
                    rgba: vec![i as u8, 7, 19, 255],
                },
            );
        }
        tiles.insert("zzz:tail".into(), solid([199, 7, 19, 255]));
        let atlas = TextureAtlas::create(&tiles);
        assert_eq!(atlas.tile_resolution(), 16);
        assert!(atlas.page_count() > 1);
        let first = &atlas.entries["mod:block_0"];
        let last = &atlas.entries["zzz:tail"];
        assert_ne!(first.page, last.page);
        let color = |region: &AtlasUv| {
            let page = atlas.page(region.page).unwrap();
            let uv = region.sample(0.5, 0.5);
            let x = (uv[0] * page.width as f32) as usize;
            let y = (uv[1] * page.height as f32) as usize;
            page.rgba[(y * page.width as usize + x) * 4..][..4].to_vec()
        };
        assert_eq!(color(first), vec![0, 7, 19, 255]);
        assert_eq!(color(last), vec![199, 7, 19, 255]);
    }
    #[test]
    fn animation_updates_only_its_layer_and_preserves_uv() {
        let mut tiles = BTreeMap::new();
        for i in 0..15_000 {
            tiles.insert(format!("mod:tile_{i}"), solid([10, 30, 60, 255]));
        }
        let key = "zzz:animated".to_string();
        tiles.insert(key.clone(), solid([10, 30, 60, 255]));
        let mut frames = BTreeMap::new();
        frames.insert(
            key.clone(),
            SpriteAnimation {
                frames: vec![solid([10, 30, 60, 255]), solid([230, 11, 24, 255])],
                schedule: vec![(0, 1), (1, 1)],
                cycle: 2,
            },
        );
        let atlas = TextureAtlas::create_at(&tiles, &frames, 0);
        let updated = atlas.advance(&frames, 0, 1);
        assert_eq!(atlas.entries[&key], updated.entries[&key]);
        let changed = atlas.entries[&key].page as usize;
        assert!(changed > 0);
        assert!(Arc::ptr_eq(&atlas.pages[0], &updated.pages[0]));
        assert!(!Arc::ptr_eq(&atlas.pages[changed], &updated.pages[changed]));
    }
    #[test]
    fn mixed_resolution_atlas_preserves_hd_and_small_sprites() {
        let mut sprites = BTreeMap::new();
        for i in 0..1000 {
            sprites.insert(format!("mod:small_{i:04}"), solid([14, 25, 36, 255]));
        }
        sprites.insert(
            "zzz:hd".into(),
            Tile {
                width: 512,
                height: 512,
                rgba: vec![207; 512 * 512 * 4],
            },
        );
        let mixed = mixed_layout(&sprites).unwrap();
        assert_eq!(mixed.max_cell, 512);
        // Unlike the old all-512-pixel-cell atlas, ordinary small sprites
        // must retain their own 16px allocations.
        assert_eq!(mixed.slots[1].cell, 16);
        assert_eq!(mixed.slots[1001].cell, 512);
        assert!(mixed.pages <= 2);
        let atlas = TextureAtlas::create(&sprites);
        let small = atlas.entries.get("mod:small_0000").unwrap();
        let hd = atlas.entries.get("zzz:hd").unwrap();
        let sample = |region: &AtlasUv| {
            let page = atlas.page(region.page).unwrap();
            let at = region.sample(0.5, 0.5);
            let off = ((at[1] * page.height as f32) as usize * page.width as usize
                + (at[0] * page.width as f32) as usize)
                * 4;
            page.rgba[off]
        };
        assert_eq!(sample(small), 14);
        assert_eq!(sample(hd), 207);
    }
    #[test]
    fn sorted_large_sprite_keeps_stable_texture_id_mapping() {
        let mut tiles = BTreeMap::new();
        for i in 0..256 {
            tiles.insert(format!("a:small_{i:03}"), solid([i as u8, 0, 0, 255]));
        }
        tiles.insert(
            "z:large".into(),
            Tile {
                width: 256,
                height: 256,
                rgba: vec![212; 256 * 256 * 4],
            },
        );
        let layout = mixed_layout(&tiles).unwrap();
        assert_eq!(layout.slots.len(), 258);
        assert_eq!(layout.slots[257].cell, 256);
        let atlas = TextureAtlas::create(&tiles);
        for (name, uv) in &atlas.entries {
            let page = atlas.page(uv.page).unwrap();
            let [u, v] = uv.sample(0.5, 0.5);
            let offset = (((v * page.height as f32) as usize) * page.width as usize
                + (u * page.width as f32) as usize)
                * 4;
            let expected = if name == "z:large" {
                212
            } else {
                name.strip_prefix("a:small_")
                    .unwrap()
                    .parse::<u16>()
                    .unwrap() as u8
            };
            assert_eq!(page.rgba[offset], expected, "{name}");
        }
    }
    #[test]
    fn hd_resolution_falls_back_only_when_page_memory_budget_requires() {
        assert!(page_plan(100, 4096).is_err());
        assert!(page_plan(100, 1024).is_ok());
    }
    #[test]
    fn sparse_hd_pack_chooses_source_resolution() {
        let mut tiles = BTreeMap::new();
        tiles.insert(
            "mod:hd".into(),
            Tile {
                width: 128,
                height: 128,
                rgba: vec![150; 128 * 128 * 4],
            },
        );
        let atlas = TextureAtlas::create(&tiles);
        assert_eq!(atlas.tile_resolution(), 128);
        assert_eq!(atlas.page_count(), 1);
    }
    #[test]
    fn invalid_png_header_rejected_before_allocating_pixels() {
        assert!(decode_png(b"not a png").is_err());
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[16..20].copy_from_slice(&10000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes());
        assert!(decode_png(&bytes).unwrap_err().contains("dimensions"));
    }
    #[test]
    fn material_map_d_combines_luminance_and_png_alpha_per_pixel(){
        let diffuse=Tile{
            width:2,height:1,
            rgba:vec![120,180,240,255, 30,60,90,128]
        };
        let alpha=Tile{
            width:2,height:1,
            rgba:vec![0,0,0,255, 255,255,255,64]
        };
        let output=combine_opacity(&diffuse,&alpha);
        assert_eq!(&output.rgba[0..3],&[120,180,240]);
        assert_eq!(output.rgba[3],0);
        assert_eq!(&output.rgba[4..7],&[30,60,90]);
        assert_eq!(output.rgba[7],32);
    }
    #[test]
    fn opacity_material_texture_ids_cannot_escape_pack_namespace(){
        assert_eq!(material_texture_id("model:example:block/widget","example:item/mask"),
            Some("example:item/mask".into()));
        assert_eq!(material_texture_id("model:example:block/widget","item/opacity"),
            Some("example:item/opacity".into()));
        assert_eq!(material_texture_id("model:example:block/widget","../secret"),None);
    }
    #[test]
    fn gltf_images_in_model_root_have_stable_noncolliding_atlas_id() {
        assert_eq!(
            texture_id("assets/create.addon/models/panel.png"),
            Some("create.addon:__gltf/panel".into())
        );
        assert_eq!(
            texture_id("assets/create.addon/models/block/panel.png"),
            Some("create.addon:__gltf/block/panel".into())
        );
        assert_eq!(
            texture_id("assets/create.addon/textures/block/panel.png"),
            Some("create.addon:panel".into())
        );
        assert_eq!(texture_id("assets/create.addon/textures/panel.png"), None);
    }
    #[test]
    fn all_model_texture_families_are_namespaced_and_collision_free() {
        assert_eq!(
            texture_id("assets/create.addon/textures/item/machine/gizmo.png"),
            Some("create.addon:item/machine/gizmo".into())
        );
        assert_eq!(
            texture_id("assets/create.addon/textures/entity/machine/gizmo.png"),
            Some("create.addon:entity/machine/gizmo".into())
        );
        assert_eq!(
            texture_id("assets/create.addon/textures/block/machine/gizmo.png"),
            Some("create.addon:machine/gizmo".into())
        );
        assert_eq!(
            texture_id("assets/create.addon/textures/blocks/machine/gizmo.png"),
            Some("create.addon:machine/gizmo".into())
        );
        assert_eq!(
            texture_id("assets/minecraft/textures/colormap/grass.png"), None
        );
        for invalid in [
            "assets/mod/textures/entity/../escape.png",
            "assets/mod/textures/item//empty.png",
            "assets/mod/textures/entity/UPPER.png",
            "assets/mod/textures/item/./secret.png",
        ] {
            assert_eq!(texture_id(invalid), None);
        }
    }
    #[test]
    fn dotted_modded_namespace_and_texture_filename_load_without_loss() {
        assert_eq!(
            texture_id("assets/create.addon/textures/block/machine/gear.v2.png"),
            Some("create.addon:machine/gear.v2".into())
        );
        assert_eq!(
            texture_id("assets/create.addon/textures/blocks/machine/gear.v2.png"),
            Some("create.addon:machine/gear.v2".into())
        );
        for filename in [
            "assets/create.addon/textures/block/../secret.png",
            "assets/create.addon/textures/block/./gear.png",
            "assets/create.addon/textures/block/a//gear.png",
            "assets/create.addon/textures/block/A.png",
            "assets/../textures/block/stone.png",
        ] {
            assert_eq!(texture_id(filename), None, "{filename}");
        }
    }
    #[cfg(unix)]
    #[test]
    fn external_assets_directory_symlink_is_not_traversed() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "mcedit2-linked-assets-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pack")).unwrap();
        std::fs::create_dir_all(root.join("external")).unwrap();
        symlink(root.join("external"), root.join("pack/assets")).unwrap();
        let error = TextureBank::open(&root.join("pack")).unwrap_err();
        assert!(error.contains("Linked resource-pack assets directory"));
        let _ = std::fs::remove_dir_all(&root);
    }
    #[test]
    fn path_parser_is_namespaced_and_does_not_accept_traversal() {
        assert_eq!(
            texture_id("assets/minecraft/textures/block/stone.png"),
            Some("minecraft:stone".into())
        );
        assert_eq!(
            texture_id("assets/example/textures/block/machine/casing.png"),
            Some("example:machine/casing".into())
        );
        assert_eq!(texture_id("assets/../textures/block/test.png"), None);
        assert_eq!(
            texture_id("assets/minecraft/textures/item/stone.png"),
            Some("minecraft:item/stone".into())
        );
    }
    #[test]
    fn exact_face_rule_and_mod_namespace() {
        let mut bank = TextureBank::default();
        Arc::make_mut(&mut bank.tiles).insert("minecraft:oak_log".into(), solid([17, 0, 0, 255]));
        Arc::make_mut(&mut bank.tiles)
            .insert("minecraft:oak_log_top".into(), solid([81, 0, 0, 255]));
        let log = state("minecraft:oak_log", vec![("axis".into(), "x".into())]);
        assert_eq!(
            bank.resolve(&log, Face::PosX).unwrap().pixel(0.4, 0.4)[0],
            81
        );
        assert_eq!(
            bank.resolve(&log, Face::Top).unwrap().pixel(0.4, 0.4)[0],
            17
        );
        assert!(bank
            .resolve(&state("mod:oak_log", vec![]), Face::Top)
            .is_none());
    }
    #[test]
    fn nearest_sampler_clamps_and_preserves_alpha() {
        let tile = Tile {
            width: 2,
            height: 2,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 160, 7, 8, 9, 255, 10, 11, 12, 0],
        };
        assert_eq!(tile.pixel(0.75, 0.25), [4, 5, 6, 160]);
        assert_eq!(tile.pixel(9.0, 9.0), [10, 11, 12, 0]);
    }
}

#[cfg(test)]
#[path = "texture_stack_tests.rs"]
mod stack_tests;
