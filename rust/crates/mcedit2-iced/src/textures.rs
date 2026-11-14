//! Optional non-mutating Java resource-pack block PNG reader.
//! Supports an extracted pack directory or a ZIP, never executes or extracts
//! pack data; missing/unknown models and textures use the purple/black missing material.
#[cfg(test)]
#[path = "texture_animation_tests.rs"]
pub(crate) mod animation_tests;
#[path = "atlas_animation.rs"]
pub(crate) mod atlas_animation;
#[path = "runtime_atlas.rs"]
pub(crate) mod runtime_atlas;
#[path = "texture_animation.rs"]
pub(crate) mod texture_animation;
#[path = "texture_sampler.rs"]
pub(crate) mod texture_sampler;
use crate::{resource_ids, resource_models::ModelBank};
#[cfg(test)]
use image::ImageFormat;
use mcedit2_world::sections::State;
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
#[cfg(test)]
use std::io::Seek;
use std::path::Path;
use std::sync::{Arc, Weak};
#[path = "biome_colors.rs"]
mod biome_colors;
#[path = "numeric_biome_colors.rs"]
mod numeric_biome_colors;
pub(crate) use biome_colors::Context as TintContext;
pub(crate) use numeric_biome_colors::Choice as NumericBiomeChoice;
#[path = "java_assets.rs"]
mod java_assets;
#[path = "texture_alpha.rs"]
mod texture_alpha;
pub(crate) use java_assets::LoadControl;
#[path = "atlas_compatibility.rs"]
mod atlas_compatibility;
pub(crate) use atlas_compatibility::atlas_compatible;

pub(crate) const MISSING_TEXTURE_ID: &str = "mcedit2:missingno";
// 内部采样身份不属于资源注册名，不能被资源包的同名 PNG 占用。
pub(crate) fn simple_key(id: &str) -> String {
    format!("@simple/{id}")
}
pub(crate) fn binding_bytes(id: &str) -> u64 {
    (std::mem::size_of::<(String, AtlasUv)>() + 64 + id.len()) as u64
}
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
#[cfg(test)]
const MAX_TEXTURE_BYTES: u64 = 32 * 1024 * 1024;
#[cfg(test)]
const MAX_PIXEL_EDGE: u32 = 4096;
pub(crate) const MAX_IMAGE_RGBA: u64 = 128 * 1024 * 1024;
const MAX_ANIMATION_STEPS: usize = 4096;
#[cfg(test)]
const MIN_ATLAS_CELL: u32 = 16;
/// 模型声明、已核实的原版通道及 PNG 推断分别保留来源，按该优先级分类。
/// 显式 / 原版通道有来源标记，不与像素推断混淆，也不额外衰减 PNG Alpha。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum AlphaMode {
    Opaque = 0,
    Cutout = 1,
    Blend = 2,
    AuthoredOpaque = 8,
    // Java 原始阈值为 0.1；单独标记，避免量化为 26/255 后错误拒绝边界像素。
    CutoutLow = 1 | 32 | crate::material_mode::NO_MIPMAP,
    SimpleCutout = 1 | 32 | crate::material_mode::NO_MIPMAP | crate::material_mode::SIMPLE_TEXTURE,
    // 原版 / 显式 tripwire 保留混合、0.1 阈值和深度写入，不能并入普通玻璃通道。
    BlendCutoutLow = 2 | 32 | crate::material_mode::TRIPWIRE,
    DeclaredCutoutMipped = 1 | 64,
    DeclaredBlend = 2 | 64,
}
impl AlphaMode {
    /// 来源声明只参与材质选择，不能泄漏为 GPU 的 VIEW_OFFSET_Z 等几何标记。
    pub(crate) const fn gpu_bits(self) -> u32 {
        match self {
            Self::DeclaredCutoutMipped => 1,
            Self::DeclaredBlend => 2,
            _ => self as u32,
        }
    }
    pub(crate) fn from_render_type(value: &str) -> Result<Self, String> {
        let id = resource_ids::qualified(value, "minecraft").ok_or("render_type 资源 ID 无效")?;
        match id.as_str() {
            "minecraft:solid" => Ok(Self::AuthoredOpaque),
            "minecraft:cutout" => Ok(Self::CutoutLow),
            "minecraft:cutout_mipped" => Ok(Self::DeclaredCutoutMipped),
            "minecraft:translucent" => Ok(Self::DeclaredBlend),
            "minecraft:tripwire" => Ok(Self::BlendCutoutLow),
            "mcedit2:simple_cutout" => Ok(Self::SimpleCutout),
            _ => Err(format!("尚未支持的 render_type：{value}")),
        }
    }
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
    if matches!(
        png,
        AlphaMode::AuthoredOpaque
            | AlphaMode::SimpleCutout
            | AlphaMode::CutoutLow
            | AlphaMode::BlendCutoutLow
            | AlphaMode::DeclaredCutoutMipped
            | AlphaMode::DeclaredBlend
    ) {
        return (png, 1.0);
    }
    let fluid_layer = (kind == VoxelMaterial::Liquid)
        .then(|| mcedit2_world::native_material::fluid_layer(state))
        .flatten();
    if let Some(layer) = fluid_layer.or_else(|| mcedit2_world::native_material::layer(state)) {
        use mcedit2_world::native_material::Layer;
        return (
            match layer {
                Layer::Solid => AlphaMode::AuthoredOpaque,
                Layer::Cutout => AlphaMode::CutoutLow,
                Layer::CutoutMipped => AlphaMode::DeclaredCutoutMipped,
                Layer::Translucent => AlphaMode::DeclaredBlend,
                Layer::Tripwire => AlphaMode::BlendCutoutLow,
            },
            1.0,
        );
    }
    (png, 1.0)
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
    #[cfg(test)]
    fn from_metadata(frames: Vec<Tile>, meta: &[u8]) -> Result<Self, String> {
        if frames.len() < 2 {
            return Err("Animated PNG needs two or more frames".into());
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
/// UV 指向实际矩形的首尾像素中心；四周各保留一像素重复边缘，避免跨材质串色。
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
    pub(crate) animation: Option<Arc<atlas_animation::Set>>,
    pub(crate) animation_frame: Option<Arc<atlas_animation::Frame>>,
    pub pages: Vec<Arc<AtlasPage>>,
    /// 与原页一一对应；旧页 Arc 在运行时追加时保持不变。
    pub(crate) mip_pages: Vec<Arc<crate::material_mips::MipPage>>,
    /// 帧快照共享 UV 索引，播放动画不能每帧复制数万个资源条目。
    pub entries: Arc<BTreeMap<String, AtlasUv>>,
    pub(crate) runtime_sources: Arc<BTreeMap<String, Arc<RuntimeTexture>>>,
    simple_errors: Arc<BTreeMap<String, String>>,
    #[cfg(test)]
    slots: BTreeMap<String, SlotPlacement>,
    cell: u32,
    pub white: AtlasUv,
    allocated_bytes: u64,
    budget_bytes: u64,
}
/// 图集页独立分配 GPU 纹理，由绘制范围切换绑定，不受总数组层数限制。
#[derive(Clone, Debug)]
pub(crate) struct AtlasPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
/// 运行时图片使用独立页追加，不能覆盖资源包的已有条目。
#[derive(Clone, Debug)]
pub(crate) struct RuntimeTexture {
    pub sampler: Option<texture_sampler::Sampler>,
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl RuntimeTexture {
    pub fn same(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.rgba == other.rgba
            && self.sampler == other.sampler
    }
}
impl TextureAtlas {
    pub(crate) fn resource_budget(&self) -> crate::resource_budget::ResourceBudget {
        crate::resource_budget::ResourceBudget {
            bytes: self.budget_bytes,
        }
    }
    /// 动态合成先检查已有资源预算，避免在图集拒绝前分配无界临时图像。
    pub(crate) fn remaining_bytes(&self) -> u64 {
        self.budget_bytes.saturating_sub(self.allocated_bytes)
    }
    /// 取消只丢弃未完成的新图集，所有旧页与旧画面继续有效。
    pub(crate) fn append_runtime_cancellable(
        &self,
        textures: impl IntoIterator<Item = RuntimeTexture>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Arc<Self>, String> {
        if cancelled() {
            return Err("运行时图集合成已取消".into());
        }
        let mut sources = BTreeMap::new();
        for texture in textures {
            if cancelled() {
                return Err("运行时图集合成已取消".into());
            }
            if sources
                .insert(texture.id.clone(), Arc::new(texture))
                .is_some()
            {
                return Err("运行时贴图 ID 重复".into());
            }
        }
        Ok(
            runtime_atlas::reconcile(Arc::new(self.clone()), sources, None, cancelled)?
                .0
                .atlas
                .clone(),
        )
    }
    pub(crate) fn extends_exactly(&self, base: &Self) -> bool {
        std::ptr::eq(self, base)
            || self.pages.len() >= base.pages.len()
                && match (&self.animation, &base.animation) {
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    (_, None) => true,
                    (None, Some(_)) => false,
                }
                && self.mip_pages.len() >= base.mip_pages.len()
                && self
                    .pages
                    .iter()
                    .zip(&base.pages)
                    .all(|(a, b)| Arc::ptr_eq(a, b))
                && self
                    .mip_pages
                    .iter()
                    .zip(&base.mip_pages)
                    .all(|(a, b)| Arc::ptr_eq(a, b))
                && self.white == base.white
                && base
                    .entries
                    .iter()
                    .all(|(id, uv)| self.entries.get(id) == Some(uv))
    }
}
// Atlas pages are uploaded as independent GPU textures; a hardware texture
// array's layer count no longer imposes a limit on the complete resource pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SlotPlacement {
    page: usize,
    x: u32,
    y: u32,
    cell: u32,
    width: u32,
    height: u32,
}
struct MixedLayout {
    page_sizes: Vec<[u32; 2]>,
    slots: Vec<SlotPlacement>,
    pages: usize,
    max_cell: u32,
}
/// 矩形纹理按原始比例排布；页只覆盖实际占用范围，不按最大纹理扩展其他资源。
#[cfg(test)]
fn mixed_layout(tiles: &BTreeMap<String, Tile>) -> Result<MixedLayout, String> {
    mixed_layout_checked(tiles, &mut || Ok(()))
}
fn mixed_layout_checked<T: Borrow<Tile>>(
    tiles: &BTreeMap<String, T>,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<MixedLayout, String> {
    check()?;
    let dimensions = std::iter::once((16u32, 16u32))
        .chain(tiles.values().map(|tile| {
            let tile: &Tile = tile.borrow();
            (tile.width, tile.height)
        }))
        .map(|(w, h)| {
            if w == 0 || h == 0 {
                return Err("图集纹理尺寸不能为零".to_owned());
            }
            let maximum = w.max(h);
            // 小纹理沿用至少 16px；大纹理保留源分辨率，上传由实际设备检查尺寸。
            let target = maximum.max(16);
            let scale = |value: u32| -> u32 {
                ((u64::from(value) * u64::from(target) + u64::from(maximum) / 2)
                    / u64::from(maximum))
                .max(1) as u32
            };
            Ok((
                scale(w),
                scale(h),
                target
                    .checked_next_power_of_two()
                    .ok_or("纹理分辨率超出图集地址范围")?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let footprint = |w: u32, h: u32| ((w + 2).div_ceil(2) * 2, (h + 2).div_ceil(2) * 2);
    // 分辨率只决定排布批次，不限制资源条目；常用小材质不散落在大型图片的空隙中。
    let class = |cell: u32| cell.ilog2().saturating_sub(8);
    let mut classes = BTreeMap::<u32, Vec<usize>>::new();
    for (i, &(_, _, cell)) in dimensions.iter().enumerate() {
        classes.entry(class(cell)).or_default().push(i);
    }
    let empty = SlotPlacement {
        page: 0,
        x: 0,
        y: 0,
        cell: 16,
        width: 16,
        height: 16,
    };
    let mut slots = vec![empty; dimensions.len()];
    #[derive(Clone, Copy)]
    struct Free {
        page: usize,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    }
    let mut page_count = 0usize;
    let mut done = 0usize;
    for (_, mut order) in classes {
        let mut area = 0u64;
        let mut largest = 0u32;
        for &i in &order {
            let (w, h, _) = dimensions[i];
            let (pw, ph) = footprint(w, h);
            largest = largest.max(pw).max(ph);
            area = area
                .checked_add(u64::from(pw) * u64::from(ph))
                .ok_or("图集面积溢出")?;
        }
        let maximum_edge = if largest > 2048 {
            8192.max(largest)
        } else {
            2048
        };
        let edge = ((area as f64).sqrt().ceil() as u32)
            .max(largest)
            .max(64)
            .min(maximum_edge)
            .div_ceil(64)
            * 64;
        if largest > edge {
            return Err("纹理矩形超过图集页几何范围".into());
        }
        order.sort_unstable_by_key(|&i| {
            let (w, h, _) = dimensions[i];
            (std::cmp::Reverse(h), std::cmp::Reverse(w), i)
        });
        let mut free = Vec::<Free>::new();
        for i in order.into_iter() {
            if done.is_multiple_of(128) {
                check()?;
            }
            done += 1;
            let (width, height, cell) = dimensions[i];
            let (pw, ph) = footprint(width, height);
            let mut found = None;
            for (index, rect) in free.iter().enumerate() {
                if index != 0 && index.is_multiple_of(4096) {
                    check()?;
                }
                if rect.w >= pw && rect.h >= ph {
                    let score = (
                        u64::from(rect.w) * u64::from(rect.h) - u64::from(pw) * u64::from(ph),
                        (rect.w - pw).min(rect.h - ph),
                        rect.page,
                        rect.y,
                        rect.x,
                    );
                    if found.is_none_or(|(_, old)| score < old) {
                        found = Some((index, score));
                    }
                }
            }
            let rect = if let Some((index, _)) = found {
                free.swap_remove(index)
            } else {
                let page = page_count;
                page_count = page_count.checked_add(1).ok_or("图集页数溢出")?;
                Free {
                    page,
                    x: 0,
                    y: 0,
                    w: edge,
                    h: edge,
                }
            };
            slots[i] = SlotPlacement {
                page: rect.page,
                x: rect.x,
                y: rect.y,
                cell,
                width,
                height,
            };
            let dw = rect.w - pw;
            let dh = rect.h - ph;
            // 二分剩余空间，不丢掉长条纹理旁边的可用整列；子矩形互不重叠。
            if dw > dh {
                if dw > 0 {
                    free.push(Free {
                        page: rect.page,
                        x: rect.x + pw,
                        y: rect.y,
                        w: dw,
                        h: rect.h,
                    });
                }
                if dh > 0 {
                    free.push(Free {
                        page: rect.page,
                        x: rect.x,
                        y: rect.y + ph,
                        w: pw,
                        h: dh,
                    });
                }
            } else {
                if dw > 0 {
                    free.push(Free {
                        page: rect.page,
                        x: rect.x + pw,
                        y: rect.y,
                        w: dw,
                        h: ph,
                    });
                }
                if dh > 0 {
                    free.push(Free {
                        page: rect.page,
                        x: rect.x,
                        y: rect.y + ph,
                        w: rect.w,
                        h: dh,
                    });
                }
            }
        }
    }
    let mut page_sizes = vec![[0u32; 2]; page_count];
    for slot in &slots {
        let (pw, ph) = footprint(slot.width, slot.height);
        let size = &mut page_sizes[slot.page];
        size[0] = size[0].max(slot.x + pw);
        size[1] = size[1].max(slot.y + ph);
    }
    for size in &mut page_sizes {
        for n in size {
            *n = (*n).max(64).div_ceil(64) * 64;
        }
    }
    check()?;
    Ok(MixedLayout {
        pages: page_sizes.len(),
        page_sizes,
        max_cell: slots.iter().map(|s| s.cell).max().unwrap_or(16),
        slots,
    })
}
fn padded_tile(
    tile: &Tile,
    width: u32,
    height: u32,
    extend_rgb: bool,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<Vec<[u8; 4]>, String> {
    let w = width as usize;
    let h = height as usize;
    let count = w.checked_mul(h).ok_or("图集矩形大小溢出")?;
    let mut data = Vec::new();
    data.try_reserve_exact(count)
        .map_err(|e| format!("图集矩形工作内存不足：{e}"))?;
    if tile.width == width && tile.height == height {
        let pixels = bytemuck::try_cast_slice::<u8, [u8; 4]>(&tile.rgba)
            .map_err(|e| format!("纹理 RGBA 布局无效：{e}"))?;
        if pixels.len() != count {
            return Err("纹理 RGBA 数量与尺寸不一致".into());
        }
        for (y, row) in pixels.chunks_exact(w).enumerate() {
            if y.is_multiple_of(32) {
                check()?;
            }
            data.extend_from_slice(row);
        }
    } else {
        for y in 0..h {
            if y.is_multiple_of(32) {
                check()?;
            }
            for x in 0..w {
                data.push(tile.pixel((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32));
            }
        }
    }
    // Preserve the source alpha while extending edge RGB into transparent
    // pixels, preventing halos around cutout glass/foliage at page edges.
    if extend_rgb && tile.alpha_mode() == AlphaMode::Cutout {
        // 仅修改 Alpha=0 的 RGB，Alpha=255 的源像素始终不变，无需复制整张图。
        for y in 0..h {
            if y % 32 == 0 {
                check()?;
            }
            for x in 0..w {
                if data[y * w + x][3] != 0 {
                    continue;
                }
                'nearest: for radius in 1_i32..=3 {
                    for dy in -radius..=radius {
                        let dx = radius - dy.abs();
                        for nx in [x as i32 - dx, x as i32 + dx] {
                            let ny = y as i32 + dy;
                            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                                continue;
                            }
                            let sample = data[ny as usize * w + nx as usize];
                            if sample[3] == 255 {
                                data[y * w + x][..3].copy_from_slice(&sample[..3]);
                                break 'nearest;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(data)
}
impl TextureAtlas {
    pub(crate) fn material(&self, id: &str, mode: u32) -> Option<&AtlasUv> {
        if mode & crate::material_mode::SIMPLE_TEXTURE != 0 {
            self.simple(id)
        } else {
            self.entries.get(id)
        }
    }
    pub(crate) fn simple(&self, id: &str) -> Option<&AtlasUv> {
        if self.simple_errors.contains_key(id) {
            return None;
        }
        self.entries.get(&simple_key(id))
    }
    pub(crate) fn native_sampler(
        &self,
        uv: &AtlasUv,
        mode: u32,
    ) -> Result<Option<texture_sampler::Sampler>, String> {
        let mip = self.mip_page(uv.page)?;
        let Some(owner) = mip.owner_at(uv.sample(0.5, 0.5))? else {
            if mode & crate::material_mode::SIMPLE_TEXTURE != 0 {
                return Err("整图槽没有材质归属".into());
            }
            return Ok(None);
        };
        let flags = mip.materials.get(owner).ok_or("整图材质归属无效")?.padding[0];
        Ok((mode & crate::material_mode::SIMPLE_TEXTURE != 0
            || flags & texture_sampler::Sampler::FORCE != 0)
            .then(|| texture_sampler::Sampler::from_flags(flags)))
    }
    /// 图集动画与整图采样分别绑定；只写入尚未发布的新图集。
    fn configure_simple_samplers(
        &mut self,
        samplers: &BTreeMap<String, Result<texture_sampler::Sampler, String>>,
    ) -> Result<(), String> {
        for (id, sampler) in samplers {
            let Some(uv) = self.entries.get(&simple_key(id)) else {
                continue;
            };
            match sampler {
                Ok(sampler) => {
                    let mip = self
                        .mip_pages
                        .get_mut(uv.page as usize)
                        .ok_or("整图材质页缺失")?;
                    let mip = Arc::get_mut(mip).expect("新图集 mip 尚未共享");
                    let owner = mip
                        .owner_at(uv.sample(0.5, 0.5))?
                        .ok_or("整图材质没有 owner")?;
                    mip.materials
                        .get_mut(owner)
                        .ok_or("整图材质 owner 无效")?
                        .padding[0] = sampler.flags(false);
                }
                Err(error) => {
                    self.allocated_bytes = self
                        .allocated_bytes
                        .checked_add(
                            (id.len() + error.len() + std::mem::size_of::<(String, String)>() + 64)
                                as u64,
                        )
                        .ok_or("整图错误索引预算溢出")?;
                    if self.allocated_bytes > self.budget_bytes {
                        return Err("整图错误索引超出图集预算".into());
                    }
                    Arc::make_mut(&mut self.simple_errors).insert(id.clone(), error.clone());
                }
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn empty_test_atlas() -> Self {
        Self::create(&BTreeMap::new())
    }
    /// 几何/混合夹具使用未分槽的原始小图片，不伪造资源包材质矩形。
    #[cfg(test)]
    pub(crate) fn rebuild_raw_test_mips(&mut self) {
        let old = std::mem::take(&mut self.mip_pages);
        self.mip_pages = self
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| {
                if let Some(mip) = old.get(index).filter(|mip| Arc::ptr_eq(&mip.base, page)) {
                    return Arc::clone(mip);
                }
                assert!(page.width > 0 && page.height > 0);
                let (owner_width, owner_height) = (page.width.div_ceil(2), page.height.div_ceil(2));
                let owner_scale = if owner_width == page.width && owner_height == page.height {
                    1
                } else {
                    2
                };
                Arc::new(crate::material_mips::MipPage {
                    base: Arc::clone(page),
                    width: 1,
                    height: 1,
                    rgba: vec![0; 4],
                    owner_width,
                    owner_height,
                    owner_scale,
                    owner: vec![0; (owner_width as usize) * (owner_height as usize)],
                    materials: Vec::new(),
                    owner_rects: Vec::new(),
                })
            })
            .collect();
        self.allocated_bytes = self
            .pages
            .iter()
            .map(|page| page.rgba.len() as u64)
            .sum::<u64>()
            + self
                .mip_pages
                .iter()
                .map(|mip| {
                    (mip.rgba.len()
                        + mip.owner.len() * 4
                        + mip.materials.len()
                            * std::mem::size_of::<crate::material_mips::GpuMaterial>()
                        + mip.owner_rects.len() * std::mem::size_of::<[u32; 4]>())
                        as u64
                })
                .sum::<u64>();
    }
    #[cfg(test)]
    fn create(tiles: &BTreeMap<String, Tile>) -> Self {
        Self::create_at(tiles, &BTreeMap::new(), 0)
    }
    #[cfg(test)]
    fn create_at(
        tiles: &BTreeMap<String, Tile>,
        animations: &BTreeMap<String, SpriteAnimation>,
        tick: u64,
    ) -> Self {
        Self::create_at_checked(tiles, animations, tick, u64::MAX, |_, _| Ok(()))
            .expect("测试图集必须满足源数据约束")
    }
    fn create_at_checked<T: Borrow<Tile>>(
        tiles: &BTreeMap<String, T>,
        animations: &BTreeMap<String, SpriteAnimation>,
        tick: u64,
        atlas_budget: u64,
        mut check: impl FnMut(usize, usize) -> Result<(), String>,
    ) -> Result<Self, String> {
        check(0, tiles.len())?;
        let layout = mixed_layout_checked(tiles, &mut || check(0, tiles.len()))?;
        if layout.pages > u32::MAX as usize {
            return Err("图集页索引超出 u32 范围".into());
        }
        let work_total = tiles
            .len()
            .checked_add(layout.pages)
            .ok_or("图集进度溢出")?;
        let mut bindings = 0u64;
        for name in tiles.keys() {
            bindings = bindings
                .checked_add(binding_bytes(name))
                .ok_or("图集采样身份预算溢出")?;
            if !name.starts_with("@simple/") {
                let key = simple_key(name);
                if !tiles.contains_key(&key) {
                    bindings = bindings
                        .checked_add(binding_bytes(&key))
                        .ok_or("整图采样身份预算溢出")?;
                }
            }
        }
        let allocated = layout
            .page_sizes
            .iter()
            .try_fold(0u64, |sum, [w, h]| {
                sum.checked_add(u64::from(*w) * u64::from(*h) * 4)
            })
            .and_then(|bytes| bytes.checked_add(bindings))
            .ok_or("图集页预算溢出")?;
        if allocated > atlas_budget {
            return Err(format!("图集需要 {} MiB，当前资源预算剩余可用于图集 {} MiB；请提高资源内存预算后重新加载。", allocated.div_ceil(1024*1024), atlas_budget/(1024*1024)));
        }
        let mut pages = layout
            .page_sizes
            .iter()
            .map(|&[width, height]| {
                let size = (u64::from(width) * u64::from(height) * 4)
                    .try_into()
                    .map_err(|_| "图集页大小无法表示")?;
                let mut rgba = Vec::new();
                rgba.try_reserve_exact(size)
                    .map_err(|e| format!("图集页内存不足：{e}"))?;
                rgba.resize(size, 0);
                Ok::<_, String>(AtlasPage {
                    width,
                    height,
                    rgba,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let uv = |slot: SlotPlacement| {
            let [width, height] = layout.page_sizes[slot.page];
            let x = slot.x + 1;
            let y = slot.y + 1;
            AtlasUv {
                start: [
                    (x as f32 + 0.5) / width as f32,
                    (y as f32 + 0.5) / height as f32,
                ],
                end: [
                    (x as f32 + slot.width as f32 - 0.5) / width as f32,
                    (y as f32 + slot.height as f32 - 0.5) / height as f32,
                ],
                page: slot.page as u32,
            }
        };
        let white = Tile {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
        };
        write_placement_checked(
            &mut pages[layout.slots[0].page],
            layout.slots[0],
            &white,
            false,
            &mut || check(0, work_total),
        )?;
        let mut entries = BTreeMap::new();
        #[cfg(test)]
        let mut slots = BTreeMap::new();
        for (index, (name, tile)) in tiles.iter().enumerate() {
            let tile: &Tile = tile.borrow();
            let slot = layout.slots[index + 1];
            let source = animations
                .get(name)
                .map(|a| a.tile_at(tick))
                .unwrap_or(tile);
            write_placement_checked(
                &mut pages[slot.page],
                slot,
                source,
                name.starts_with("@simple/"),
                &mut || check(index + 1, work_total),
            )?;
            entries.insert(name.clone(), uv(slot));
            #[cfg(test)]
            slots.insert(name.clone(), slot);
        }
        // 普通整图复用原槽；有精灵裁切的整图已作为独立内部采样身份参与实际排布。
        for name in tiles.keys().filter(|name| !name.starts_with("@simple/")) {
            let key = simple_key(name);
            if !entries.contains_key(&key) {
                entries.insert(key, entries[name].clone());
            }
        }
        let pages: Vec<_> = pages.into_iter().map(Arc::new).collect();
        let mut rectangles = vec![Vec::new(); pages.len()];
        for slot in &layout.slots {
            rectangles[slot.page].push(crate::material_mips::SpriteRect {
                x: slot.x + 1,
                y: slot.y + 1,
                width: slot.width,
                height: slot.height,
            });
        }
        let mut mip_pages = Vec::with_capacity(pages.len());
        let mut total = allocated;
        for (index, page) in pages.iter().enumerate() {
            let done = tiles.len() + index;
            check(done, work_total)?;
            let remaining = atlas_budget.checked_sub(total).ok_or("图集超出预算")?;
            let (mip, cost) = crate::material_mips::build(
                Arc::clone(page),
                &rectangles[index],
                remaining,
                &mut || check(done, work_total),
            )?;
            total = total.checked_add(cost).ok_or("图集与 mip 总预算溢出")?;
            mip_pages.push(mip);
        }
        check(work_total, work_total)?;
        Ok(Self {
            pages,
            runtime_sources: Arc::new(BTreeMap::new()),
            simple_errors: Default::default(),
            animation: None,
            animation_frame: None,
            mip_pages,
            entries: Arc::new(entries),
            #[cfg(test)]
            slots,
            white: uv(layout.slots[0]),
            cell: layout.max_cell,
            allocated_bytes: total,
            budget_bytes: atlas_budget,
        })
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
        for (index, page) in next.pages.iter().enumerate() {
            if Arc::ptr_eq(page, &self.pages[index]) {
                continue;
            }
            let old = &self.mip_pages[index];
            let rectangles = old
                .materials
                .iter()
                .map(|material| {
                    let uv = material.base_uv;
                    crate::material_mips::SpriteRect {
                        x: (uv[0] * page.width as f32 - 0.5).round() as u32,
                        y: (uv[1] * page.height as f32 - 0.5).round() as u32,
                        width: ((uv[2] - uv[0]) * page.width as f32 + 1.0).round() as u32,
                        height: ((uv[3] - uv[1]) * page.height as f32 + 1.0).round() as u32,
                    }
                })
                .collect::<Vec<_>>();
            let old_bytes = (old.rgba.len()
                + old.owner.len() * 4
                + old.materials.len() * std::mem::size_of::<crate::material_mips::GpuMaterial>()
                + old.owner_rects.len() * std::mem::size_of::<[u32; 4]>())
                as u64;
            let base_cost = next
                .allocated_bytes
                .checked_sub(old_bytes)
                .expect("旧 mip 预算");
            let (mip, cost) = crate::material_mips::build(
                Arc::clone(page),
                &rectangles,
                next.budget_bytes
                    .checked_sub(base_cost)
                    .expect("动画 mip 可用预算"),
                &mut || Ok(()),
            )
            .expect("动画 mip 重建");
            next.allocated_bytes = base_cost.checked_add(cost).expect("动画 mip 总预算");
            next.mip_pages[index] = mip;
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
    pub(crate) fn mip_page(
        &self,
        index: u32,
    ) -> Result<&Arc<crate::material_mips::MipPage>, String> {
        let base = self.pages.get(index as usize).ok_or("图集页索引无效")?;
        let mip = self
            .mip_pages
            .get(index as usize)
            .ok_or("图集页缺少材质 mip")?;
        if !Arc::ptr_eq(base, &mip.base) {
            return Err("材质 mip 与原图集页身份不一致".into());
        }
        Ok(mip)
    }
    pub(crate) fn allocated_bytes(&self) -> u64 {
        self.allocated_bytes
    }
    /// CPU Canvas 与拾取传入屏幕梯度获得同一材质级别的过滤结果。
    pub(crate) fn sample_lod(
        &self,
        uv: [f32; 2],
        page: u32,
        lod: f32,
        linear: bool,
        mode: u32,
    ) -> Result<[f32; 4], String> {
        if let Some(pixel) = self.animation_sample(uv, page, lod, linear, mode)? {
            return Ok(pixel);
        }
        self.mip_page(page)?.sample_lod(uv, lod, linear, mode)
    }
}
#[cfg(test)]
fn write_placement(page: &mut AtlasPage, slot: SlotPlacement, tile: &Tile) {
    write_placement_checked(page, slot, tile, false, &mut || Ok(())).expect("测试贴图写入失败");
}
fn write_placement_checked(
    page: &mut AtlasPage,
    slot: SlotPlacement,
    tile: &Tile,
    simple: bool,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    if slot.width == 0
        || slot.height == 0
        || slot
            .x
            .checked_add(slot.width)
            .and_then(|x| x.checked_add(2))
            .is_none_or(|x| x > page.width)
        || slot
            .y
            .checked_add(slot.height)
            .and_then(|y| y.checked_add(2))
            .is_none_or(|y| y > page.height)
        || page.rgba.len() as u64 != u64::from(page.width) * u64::from(page.height) * 4
    {
        return Err("纹理矩形超出图集页边界".into());
    }
    let data = padded_tile(tile, slot.width, slot.height, !simple, check)?;
    let width = slot.width as usize;
    for y in 0..slot.height + 2 {
        if y.is_multiple_of(32) {
            check()?;
        }
        let yy = (y as i32 - 1).clamp(0, slot.height as i32 - 1) as usize;
        let row = &data[yy * width..(yy + 1) * width];
        let at = (((slot.y + y) * page.width + slot.x) * 4) as usize;
        page.rgba[at..at + 4].copy_from_slice(&row[0]);
        page.rgba[at + 4..at + 4 + width * 4].copy_from_slice(bytemuck::cast_slice(row));
        page.rgba[at + 4 + width * 4..at + 8 + width * 4].copy_from_slice(&row[width - 1]);
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct StaticObjTriangle {
    pub xyz: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub page: u32,
    pub rgba: [[f32; 4]; 3],
    pub cull: Option<crate::isometric::Face>,
    pub tint_index: Option<u32>,
    /// Low two bits are opaque/cutout/blend; high bits encode static glTF
    /// sidedness, cutoff and OPAQUE alpha semantics.
    pub mode: u32,
}
/// 资源身份只锚定控制块，避免地址复用误命中，不额外保活旧图片和模型载荷。
#[derive(Clone, Debug)]
pub(crate) struct VisibilityIdentity {
    block_layers: crate::block_visibility::Layers,
    world_data: Weak<mcedit2_world::world_data::Snapshot>,
    fonts: Weak<crate::font_resources::Resources>,
    models: Weak<ModelBank>,
    tiles: Weak<BTreeMap<String, Tile>>,
    simple_tiles: Weak<BTreeMap<String, Tile>>,
    simple_samplers: Weak<BTreeMap<String, Result<texture_sampler::Sampler, String>>>,
    png_layouts: Weak<BTreeMap<String, [u8; 2]>>,
    live_animations: Weak<BTreeMap<String, Arc<texture_animation::Sprite>>>,
    animations: Weak<BTreeMap<String, SpriteAnimation>>,
    tints: Weak<biome_colors::Resources>,
}
impl PartialEq for VisibilityIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.block_layers == other.block_layers
            && self.world_data.ptr_eq(&other.world_data)
            && self.fonts.ptr_eq(&other.fonts)
            && self.models.ptr_eq(&other.models)
            && self.tiles.ptr_eq(&other.tiles)
            && self.simple_tiles.ptr_eq(&other.simple_tiles)
            && self.simple_samplers.ptr_eq(&other.simple_samplers)
            && self.png_layouts.ptr_eq(&other.png_layouts)
            && self.live_animations.ptr_eq(&other.live_animations)
            && self.animations.ptr_eq(&other.animations)
            && self.tints.ptr_eq(&other.tints)
    }
}
impl Eq for VisibilityIdentity {}
#[derive(Clone, Debug, Default)]
pub(crate) struct TextureBank {
    block_layers: crate::block_visibility::Layers,
    available_textures: Arc<Vec<String>>,
    /// 最终生效 PNG 的实际资源包 / 目录 / 资产索引；不借 mcmeta 来源覆盖图片归属。
    texture_origins: Arc<BTreeMap<String, Arc<std::path::PathBuf>>>,
    definition_source_paths: Arc<Vec<std::path::PathBuf>>,
    generated_items: Arc<crate::item_generated::Cache>,
    world_data: Arc<mcedit2_world::world_data::Snapshot>,
    /// 注册表按已绑定的实际世界解释；图像客户端版本不能覆盖存档的定义合同。
    world_version: Option<i32>,
    item_version: Option<i32>,
    live_animations: Arc<BTreeMap<String, Arc<texture_animation::Sprite>>>,
    /// 图集元数据无效或只选择了静态子帧时，PNG 整图仍有独立的 SimpleTexture 合同。
    simple_tiles: Arc<BTreeMap<String, Tile>>,
    simple_samplers: Arc<BTreeMap<String, Result<texture_sampler::Sampler, String>>>,
    /// 原 PNG 的位深与颜色类型；旧图案遮罩的读取合同需要实际格式。
    png_layouts: Arc<BTreeMap<String, [u8; 2]>>,
    fonts: Arc<crate::font_resources::Resources>,
    languages: Arc<crate::block_names::Names>,
    alpha_cache: Arc<texture_alpha::Cache>,
    /// 完成后的统计及诊断由运行时图集视图共享；加载期间按需独占写入。
    pub statistics: Arc<LoadStatistics>,
    // Original PNG/JSON source is immutable and shared across animation.
    // Only changed RGBA pages are cloned on a sprite frame transition.
    tiles: Arc<BTreeMap<String, Tile>>,
    models: Arc<ModelBank>,
    // 旧实验导入器的数据结构；生产 Java 动画由 live_animations 保留源和完整时序。
    animations: Arc<BTreeMap<String, SpriteAnimation>>,
    static_animation_count: usize,
    metadata: BTreeMap<String, Vec<u8>>,
    villager_metadata: Arc<BTreeMap<String, Vec<u8>>>,
    tints: Arc<biome_colors::Resources>,
    modern_textures: BTreeSet<String>,
    modern_metadata: BTreeSet<String>,
    // Track identities replaced in each overlay independently of entry order.
    layer_png: BTreeSet<String>,
    layer_metadata: BTreeSet<String>,
    atlas: Option<Arc<TextureAtlas>>,
    #[cfg(test)]
    animation_tick: u64,
    pub warnings: Arc<Vec<String>>,
}
impl TextureBank {
    pub(crate) fn geometry_budget(&self) -> Option<crate::resource_budget::ResourceBudget> {
        self.atlas().map(|atlas| atlas.resource_budget())
    }
    /// 只改变世界显示策略，资源、图集及原始模型保持共享。
    pub(crate) fn with_block_layers(mut self, layers: crate::block_visibility::Layers) -> Self {
        self.block_layers = layers;
        self
    }

    pub(crate) fn block_layers(&self) -> crate::block_visibility::Layers {
        self.block_layers
    }

    pub(crate) fn block_entity_replaces_base(&self, state: &State) -> bool {
        self.declarative_block_replaces(state) == Some(true)
            || self.models.vanilla_banner_supported(state)
            || self.models.vanilla_skull_supported(state)
            || self.models.vanilla_sign_supported(state)
            || self.models.vanilla_piston_supported(state)
    }

    pub(crate) fn world_block_visible(&self, state: &State) -> bool {
        self.block_layers.visible(state, Some(self))
    }

    pub(crate) fn is_block_entity_model(&self, state: &State) -> bool {
        self.models.is_static_block_entity(state) || self.block_entity_replaces_base(state)
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct LoadStatistics {
    pub indexed_png: usize,
    pub requested_png: usize,
    pub decoded_png: usize,
    pub failed_png: usize,
    pub missing_references: Vec<String>,
    pub decoded_bytes: usize,
    pub budget_bytes: u64,
    pub atlas_bytes: u64,
    pub animation_reserved_bytes: u64,
    pub atlas_png: usize,
    pub indexed_models: usize,
    pub failed_models: usize,
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
    if !resource_ids::path(name) {
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
#[cfg(test)]
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
#[cfg(test)]
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
    Language,
    NumericBiomes,
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
        if name == numeric_biome_colors::ASSET {
            return Some(Self::NumericBiomes);
        }
        if crate::block_names::locale_asset(name).is_some() {
            return Some(Self::Language);
        }
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
        if crate::resource_models::obj_source_suffix(name).is_some() && name.contains("/models/") {
            return Some(Self::Obj);
        }
        (name.ends_with(".json") && (name.contains("/models/") || name.contains("/blockstates/")))
            .then_some(Self::Model)
    }
    /// 内建 Java 资源包含 JSON/PNG 和声明式 Forge / NeoForge OBJ/MTL。
    /// glTF、二进制缓冲和自定义可执行渲染器仍需独立扩展。
    fn is_java_resource(&self, path: &str) -> bool {
        matches!(
            self,
            Self::Language
                | Self::NumericBiomes
                | Self::Model
                | Self::Obj
                | Self::Mtl
                | Self::Metadata(_)
                | Self::Colormap(_)
        ) || (matches!(self, Self::Texture(_)) && path.split('/').nth(2) == Some("textures"))
    }
    fn max_bytes(&self, budget: crate::resource_budget::ResourceBudget) -> u64 {
        match self {
            Self::Colormap(_) | Self::Texture(_) => budget.bytes,
            _ => budget.bytes / 8,
        }
    }
}

/// Bake static Wavefront map_d into RGBA *before* the shared atlas is
/// allocated. A separate second atlas sample per fragment is unnecessary,
/// and textured cutout opacity is preserved at pixel resolution.
#[cfg(test)]
fn combine_opacity(diffuse: &Tile, mask: &Tile) -> Tile {
    let mut rgba = diffuse.rgba.clone();
    let width = diffuse.width as usize;
    let height = diffuse.height as usize;
    for y in 0..height {
        for x in 0..width {
            let m = mask.pixel(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let luma =
                (0.2126 * m[0] as f32 + 0.7152 * m[1] as f32 + 0.0722 * m[2] as f32).round() as u16;
            // Grayscale maps use luminance; PNG alpha masks use Alpha.
            let opacity = if m[3] < 255 { m[3] as u16 } else { luma };
            let i = (y * width + x) * 4 + 3;
            rgba[i] = ((rgba[i] as u16 * opacity + 127) / 255) as u8;
        }
    }
    Tile {
        width: diffuse.width,
        height: diffuse.height,
        rgba,
    }
}
#[cfg(test)]
fn material_texture_id(model: &str, raw: &str) -> Option<String> {
    let ns = model.strip_prefix("model:")?.split_once(':')?.0;
    let id = resource_ids::qualified(raw, ns)?;
    let (ns, path) = id.split_once(':')?;
    let path = path
        .strip_prefix("block/")
        .or_else(|| path.strip_prefix("blocks/"))
        .unwrap_or(path);
    Some(format!("{ns}:{path}"))
}
impl TextureBank {
    pub(crate) fn block_model_catalog(&self) -> &[String] {
        self.models.available_models()
    }
    pub(crate) fn definition_texture_slots(
        &self,
        model: &str,
        overrides: &BTreeMap<String, String>,
    ) -> Result<Vec<crate::resource_models::TextureSlot>, String> {
        self.models.definition_texture_slots(model, overrides)
    }
    pub(crate) fn definition_model_identity(&self) -> Weak<ModelBank> {
        Arc::downgrade(&self.models)
    }
    pub(crate) fn definition_sources(
        &self,
        world: &mcedit2_world::WorldSummary,
    ) -> Option<&[std::path::PathBuf]> {
        Arc::ptr_eq(&self.world_data, &world.world_data)
            .then_some(self.definition_source_paths.as_slice())
    }
    pub(crate) fn texture_catalog(&self) -> &[String] {
        &self.available_textures
    }
    pub(crate) fn texture_origin(&self, id: &str) -> Option<&Path> {
        self.texture_origins.get(id).map(|path| path.as_path())
    }
    pub(crate) fn texture_pack_paths(&self) -> impl Iterator<Item = &Arc<std::path::PathBuf>> {
        self.texture_origins.values()
    }
    pub(crate) fn definition_texture_preview(
        &self,
        id: &str,
    ) -> Option<iced::widget::image::Handle> {
        let tile = self.tiles.get(id)?;
        Some(iced::widget::image::Handle::from_rgba(
            tile.width,
            tile.height,
            tile.rgba.clone(),
        ))
    }
    pub(crate) fn definition_legacy_registry(&self) -> Option<Arc<crate::legacy_render::Registry>> {
        self.models.legacy.clone()
    }
    pub(crate) fn simple_sampler(&self, id: &str) -> Result<texture_sampler::Sampler, String> {
        self.simple_samplers
            .get(id)
            .cloned()
            .unwrap_or(Ok(texture_sampler::Sampler::default()))
    }
    pub(crate) fn png_layout(&self, id: &str) -> Option<[u8; 2]> {
        self.png_layouts.get(id).copied()
    }
    fn source_image(&self, id: &str, mode: u32) -> Option<&Tile> {
        if mode & crate::material_mode::SIMPLE_TEXTURE != 0 {
            self.simple_image(id)
        } else {
            self.get(id)
        }
    }
    fn source_alpha(&self, id: &str, mode: u32) -> Option<AlphaMode> {
        if mode & crate::material_mode::SIMPLE_TEXTURE != 0 {
            self.alpha_cache.simple(self, id)
        } else {
            self.sprite_alpha(id)
        }
    }
    /// 与 atlas 精灵独立的 PNG 源；该合同不解释 animation 节点。
    pub(crate) fn simple_image(&self, id: &str) -> Option<&Tile> {
        self.simple_tiles
            .get(id)
            .or_else(|| self.live_animations.get(id).map(|sprite| &sprite.source))
            .or_else(|| self.get(id))
    }
    /// 世界注册表在客户端内置定义之上应用；普通资源包的 data/ 仍不参与覆盖。
    pub(crate) fn apply_world_data(
        &mut self,
        world: &mcedit2_world::WorldSummary,
        control: &LoadControl,
    ) -> Result<(), String> {
        control.check()?;
        let current = mcedit2_world::world_data::Snapshot::read(&world.root, world.data_version)
            .map_err(|error| format!("世界数据包来源读取失败：{error}"))?;
        if current.origin != world.world_data.origin {
            return Err(
                "世界元数据或启用数据包在打开后已变化，请重新打开存档以加载一致来源".into(),
            );
        }
        self.apply_registry_data(Arc::clone(&world.world_data), world.data_version, control)
    }
    /// 显式注册表快照可来自真实世界或独立结构所选数据包，不附带伪造世界属性。
    pub(crate) fn apply_registry_data(
        &mut self,
        data: Arc<mcedit2_world::world_data::Snapshot>,
        version: Option<i32>,
        control: &LoadControl,
    ) -> Result<(), String> {
        control.check()?;
        let budget = crate::resource_budget::ResourceBudget {
            bytes: self.statistics.budget_bytes,
        };
        budget.check(
            data.allocated_bytes(),
            self.statistics.decoded_bytes,
            self.statistics.atlas_bytes,
        )?;
        let tints = Arc::make_mut(&mut self.tints);
        for layer in &data.biome_layers {
            tints.apply_world_layer(layer, control)?;
        }
        for diagnostic in &data.diagnostics {
            log::warn!("[world-data] {diagnostic}");
            Arc::make_mut(&mut self.warnings).push(diagnostic.clone());
        }
        self.world_data = data;
        self.world_version = version;
        Ok(())
    }
    pub(crate) fn painting_catalog(
        &self,
        version: i32,
    ) -> Result<Arc<mcedit2_world::painting::Catalog>, String> {
        self.world_data.painting_catalog(version)
    }
    pub(crate) fn piglin_tags(
        &self,
        version: i32,
    ) -> Result<Arc<mcedit2_world::piglin_appearance::Tags>, String> {
        self.world_data.piglin_tags(version)
    }
    pub(crate) fn wolf_variant_catalog(
        &self,
    ) -> Result<Arc<mcedit2_world::wolf_variant::Catalog>, String> {
        self.world_data.wolf_variant_catalog()
    }
    pub(crate) fn banner_catalog(&self) -> Result<Arc<mcedit2_world::banner::Catalog>, String> {
        let version = if self.world_data.origin.is_empty() {
            self.item_data_version()
                .ok_or("旗帜资源缺少准确客户端版本")?
        } else {
            self.world_version
                .ok_or("旗帜图案缺少实际世界 DataVersion")?
        };
        self.world_data.banner_catalog(version)
    }
    pub(crate) fn has_declarative_entity(&self, id: &str) -> bool {
        self.models.has_declarative_entity(id)
    }
    pub(crate) fn declarative_block_states(&self) -> std::collections::BTreeSet<String> {
        self.models.declarative_block_states()
    }
    pub(crate) fn declarative_block_replaces(&self, state: &State) -> Option<bool> {
        self.models.declarative_block_replaces(state)
    }
    pub(crate) fn declarative_block_quads(
        &self,
        state: &State,
        visual: Option<&mcedit2_world::entity_catalog::EntityVisual>,
        textures: bool,
    ) -> Option<(bool, Result<Vec<crate::world_mesh::WorldQuad>, String>)> {
        self.models
            .declarative_block_layers(state, visual)
            .map(|(replaces, result)| {
                (
                    replaces,
                    result.and_then(|faces| {
                        crate::layer_renderer::world_quads(
                            &faces,
                            self.atlas.as_deref().ok_or("方块实体外观缺少已完成图集")?,
                            textures,
                        )
                    }),
                )
            })
    }
    pub(crate) fn declarative_entity_quads(
        &self,
        id: &str,
        visual: &mcedit2_world::entity_catalog::EntityVisual,
    ) -> Option<Result<Vec<crate::world_mesh::WorldQuad>, String>> {
        self.models
            .declarative_entity_layers(id, visual)
            .map(|result| {
                result.and_then(|triangles| {
                    crate::layer_renderer::world_quads(
                        &triangles,
                        self.atlas.as_deref().ok_or("声明式实体缺少已完成图集")?,
                        true,
                    )
                })
            })
    }
    pub(crate) fn font_resources(&self) -> &crate::font_resources::Resources {
        &self.fonts
    }

    pub(crate) fn item_texture_animated(&self, id: &str) -> bool {
        self.live_animations.contains_key(id)
    }
    pub(crate) fn item_animation_handle(&self, id: &str) -> Option<Arc<texture_animation::Sprite>> {
        self.live_animations.get(id).map(Arc::clone)
    }
    pub(crate) fn item_animation(&self, id: &str) -> Option<&texture_animation::Sprite> {
        self.live_animations.get(id).map(Arc::as_ref)
    }
    pub(crate) fn generated_items(&self) -> &crate::item_generated::Cache {
        &self.generated_items
    }
    pub(crate) fn item_tile(&self, id: &str) -> Option<std::borrow::Cow<'_, Tile>> {
        if let Some(sprite) = self.live_animations.get(id) {
            let tick = self
                .atlas
                .as_ref()
                .and_then(|a| a.animation_tick())
                .unwrap_or(0);
            return Some(std::borrow::Cow::Owned(
                sprite.tile(sprite.sequence.stamp(tick)),
            ));
        }
        self.get(id).map(std::borrow::Cow::Borrowed)
    }
    pub(crate) fn item_data_version(&self) -> Option<i32> {
        self.item_version
    }
    pub(crate) fn item_scene(
        &self,
        id: &str,
        properties: &BTreeMap<String, f32>,
    ) -> Result<crate::resource_models::items::Scene, String> {
        self.models.item_scene(id, properties)
    }
    pub(crate) fn item_scene_context(
        &self,
        id: &str,
        properties: &BTreeMap<String, f32>,
        context: &str,
    ) -> Result<crate::resource_models::items::Scene, String> {
        self.models.item_scene_context(id, properties, context)
    }
    pub(crate) fn frame_scene(
        &self,
        glow: bool,
        map: bool,
    ) -> Result<crate::resource_models::items::Scene, String> {
        self.models.scene_model(
            format!(
                "model:minecraft:{}item_frame{}",
                if glow { "glow_" } else { "" },
                if map { "_map" } else { "" }
            ),
            &BTreeMap::new(),
            "fixed",
        )
    }
    pub(crate) fn translation(
        &self,
        key: &str,
        language: crate::language::Language,
    ) -> Option<&str> {
        self.languages.translation(key, language)
    }
    pub(crate) fn retain_villager_metadata(&mut self) {
        self.villager_metadata = Arc::new(
            self.metadata
                .iter()
                .filter(|(id, _)| crate::entity_geometry::villager_texture_dependency(id))
                .map(|(id, data)| (id.clone(), data.clone()))
                .collect(),
        );
    }
    pub(crate) fn villager_hat(&self, id: &str) -> Result<&str, String> {
        let Some(data) = self.villager_metadata.get(id) else {
            return Ok("none");
        };
        let value = mcedit2_world::client_json::resource_object(data)
            .map_err(|e| format!("{id} 帽元数据无效：{e}"))?
            .value;
        let Some(section) = value.get("villager") else {
            return Ok("none");
        };
        let section = section
            .as_object()
            .ok_or_else(|| format!("{id} villager 元数据不是对象"))?;
        match section.get("hat") {
            None => Ok("none"),
            Some(value) => match value.as_str() {
                Some("none") => Ok("none"),
                Some("partial") => Ok("partial"),
                Some("full") => Ok("full"),
                _ => Err(format!("{id} 帽元数据值无效")),
            },
        }
    }
    pub(crate) fn block_name(&self, id: &str, language: crate::language::Language) -> Option<&str> {
        self.languages.name(id, language)
    }
    pub(crate) fn render_state<'a>(&'a self, state: &'a State) -> &'a State {
        self.models.render_state(state)
    }
    pub(crate) fn render_material(
        &self,
        state: &State,
        source: mcedit2_world::surface::VoxelMaterial,
    ) -> mcedit2_world::surface::VoxelMaterial {
        if let Some(binding) = self.models.legacy.as_ref().and_then(|r| r.resolve(state)) {
            mcedit2_world::surface::material(&binding.state)
        } else {
            source
        }
    }
    /// 证明当前显示的完整不透明边界，包含严格核实的棋盘立方体表示。
    /// 部分模型、旋转 / 根变换及透明材质均不从缺失数据猜测完整边界。
    pub(crate) fn opaque_boundary_mask(
        &self,
        state: &State,
        pos: (i32, i32, i32),
        source_kind: mcedit2_world::surface::VoxelMaterial,
        show_textures: bool,
    ) -> u8 {
        if !self.world_block_visible(state) {
            return 0;
        }
        if self.declarative_block_replaces(state) == Some(true) {
            return 0;
        }
        use crate::isometric::Face;
        use mcedit2_world::surface::{VoxelMaterial, ALL_FACES};
        const FACES: [Face; 6] = [
            Face::Top,
            Face::Bottom,
            Face::PosX,
            Face::NegX,
            Face::PosZ,
            Face::NegZ,
        ];
        let display = self.render_state(state);
        let kind = self.render_material(state, source_kind);
        if kind == VoxelMaterial::Liquid && mcedit2_world::fluid_surface::is_named_fluid(display) {
            // 实际高度由源邻域生成，不能把同名 JSON 立方体当成完整遮挡证明。
            return 0;
        }
        if !show_textures {
            // 与 spectator / isometric 的纯色发射一致：原始状态决定调色板 Alpha，
            // 实际显示状态决定默认材质通道；关闭 PNG 后不再使用作者的贴图 Alpha。
            if [false, true].into_iter().any(|top| {
                crate::palette::tint(state, source_kind, top)
                    .unwrap_or_else(|| crate::palette::unknown_tint(state, source_kind))
                    .a
                    != 1.0
            }) {
                return 0;
            }
            let (mode, alpha) = classify_block_alpha(display, kind, AlphaMode::Opaque);
            if alpha != 1.0
                || !matches!(
                    mode,
                    AlphaMode::Opaque
                        | AlphaMode::AuthoredOpaque
                        | AlphaMode::Cutout
                        | AlphaMode::CutoutLow
                        | AlphaMode::DeclaredCutoutMipped
                )
            {
                return 0;
            }
        }
        // 纯色 Alpha 已按当前模式核实；贴图模式还需核实真实棋盘和默认材质通道。
        // 逐面作者通道仍在下面单独分类，不由方块的名称或粗略类别替代。
        let opaque_checker = || {
            let (mode, alpha) = classify_block_alpha(display, kind, AlphaMode::Opaque);
            !show_textures
                || (alpha == 1.0
                    && matches!(
                        mode,
                        AlphaMode::Opaque
                            | AlphaMode::AuthoredOpaque
                            | AlphaMode::Cutout
                            | AlphaMode::CutoutLow
                            | AlphaMode::DeclaredCutoutMipped
                    )
                    && self
                        .atlas
                        .as_ref()
                        .is_some_and(|atlas| atlas.entries.contains_key(MISSING_TEXTURE_ID))
                    && self.sprite_alpha(MISSING_TEXTURE_ID) == Some(AlphaMode::Opaque))
        };
        let Some(parts) = self.models.parts_at(state, pos) else {
            // geometry_at 对声明但不可解析的模型使用完整棋盘盒；无模型的外部命名空间
            // 也只使用该盒。数字旧版和内置近似几何仍沿原合同处理。
            let foreign =
                !state.name.starts_with("minecraft:") && !state.name.starts_with("numeric:");
            let cube = !state.name.starts_with("numeric:")
                && (self.models.declares_state(state) || foreign)
                && self
                    .models
                    .obj_at(state, pos)
                    .is_none_or(|triangles| triangles.is_empty())
                && FACES.into_iter().all(|face| {
                    self.models.face_defined_at(state, face, 0, pos).is_none()
                        && self
                            .models
                            .face_cull_direction_at(state, face, 0, pos)
                            .is_none()
                        && self.models.element_face_at(state, face, 0, pos).is_none()
                });
            return if cube && opaque_checker() {
                ALL_FACES
            } else {
                0
            };
        };
        let mut checker = None;
        let mut mask = 0;
        for (element, part) in parts.iter().enumerate() {
            if part.bounds != crate::models::Box3::FULL
                || part.rotation.is_some()
                || part.root.is_some()
                || part.variant_x % 90 != 0
                || part.variant_y % 90 != 0
            {
                continue;
            }
            for face in FACES {
                let direction = match part.normal(face.normal()) {
                    [0., 1., 0.] => Face::Top,
                    [0., -1., 0.] => Face::Bottom,
                    [1., 0., 0.] => Face::PosX,
                    [-1., 0., 0.] => Face::NegX,
                    [0., 0., 1.] => Face::PosZ,
                    [0., 0., -1.] => Face::NegZ,
                    _ => continue,
                };
                match self
                    .models
                    .face_cull_direction_at(state, face, element, pos)
                {
                    Some(Some(cull)) if cull != direction => continue,
                    None => continue,
                    _ => {}
                }
                let Some(spec) = self.models.element_face_at(state, face, element, pos) else {
                    continue;
                };
                if !show_textures {
                    mask |= direction.flag();
                    continue;
                }
                // atlas_face_style_at 对这两种缺图情况都绘制同一棋盘，不把原图猜成不透明。
                let pixels = if spec.texture == MISSING_TEXTURE_ID
                    || self
                        .source_image(
                            &spec.texture,
                            spec.render_type.map_or(0, AlphaMode::gpu_bits),
                        )
                        .is_none()
                {
                    if !*checker.get_or_insert_with(opaque_checker) {
                        continue;
                    }
                    AlphaMode::Opaque
                } else {
                    let Some(pixels) = self.source_alpha(
                        &spec.texture,
                        spec.render_type.map_or(0, AlphaMode::gpu_bits),
                    ) else {
                        continue;
                    };
                    pixels
                };
                let (mode, alpha) =
                    classify_block_alpha(display, kind, spec.render_type.unwrap_or(pixels));
                let opaque = matches!(mode, AlphaMode::Opaque | AlphaMode::AuthoredOpaque)
                    || (pixels == AlphaMode::Opaque
                        && matches!(
                            mode,
                            AlphaMode::Cutout
                                | AlphaMode::CutoutLow
                                | AlphaMode::SimpleCutout
                                | AlphaMode::DeclaredCutoutMipped
                        ));
                if opaque && alpha == 1. {
                    mask |= direction.flag();
                }
            }
        }
        mask
    }
    #[cfg(test)]
    fn ingest_asset(
        &mut self,
        name: &str,
        asset: PackAsset,
        bytes: Vec<u8>,
        memory: &mut usize,
    ) -> Result<(), String> {
        match asset {
            PackAsset::NumericBiomes => Arc::make_mut(&mut self.tints).insert_numeric(&bytes)?,
            PackAsset::Language => match Arc::make_mut(&mut self.languages).insert(name, &bytes) {
                Ok(diagnostics) => Arc::make_mut(&mut self.warnings).extend(
                    diagnostics
                        .into_iter()
                        .map(|error| format!("语言资源 {name}：{error}")),
                ),
                Err(error) => {
                    Arc::make_mut(&mut self.warnings).push(format!("语言资源 {name} 无效：{error}"))
                }
            },
            PackAsset::Colormap(which) => {
                if let Err(e) = self.insert_colormap(which, &bytes) {
                    Arc::make_mut(&mut self.warnings).push(format!("Skipped {name}: {e}"));
                }
            }
            PackAsset::Model => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert(name, &bytes) {
                    Arc::make_mut(&mut self.warnings).push(format!("Skipped model {name}: {e}"));
                }
            }
            PackAsset::Obj => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert_obj(name, &bytes) {
                    Arc::make_mut(&mut self.warnings).push(format!("Skipped OBJ {name}: {e}"));
                }
            }
            PackAsset::Gltf => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert_gltf(name, &bytes) {
                    Arc::make_mut(&mut self.warnings).push(format!("Skipped glTF {name}: {e}"));
                }
            }
            PackAsset::GltfBuffer => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert_gltf_buffer(name, &bytes) {
                    Arc::make_mut(&mut self.warnings)
                        .push(format!("Skipped glTF buffer {name}: {e}"));
                }
            }
            PackAsset::Mtl => {
                if let Err(e) = Arc::make_mut(&mut self.models).insert_mtl(name, &bytes) {
                    Arc::make_mut(&mut self.warnings).push(format!("Skipped MTL {name}: {e}"));
                }
            }
            PackAsset::Metadata(id) => {
                if self.accept_metadata_path(&id, name) {
                    self.layer_metadata.insert(id.clone());
                    self.metadata.insert(id, bytes);
                }
            }
            PackAsset::Texture(id) => {
                if self.accept_texture_path(&id, name) {
                    if let Err(e) = self.insert_texture_path(id.clone(), name, &bytes, memory) {
                        Arc::make_mut(&mut self.warnings).push(format!("Skipped {id}: {e}"));
                    }
                }
            }
        }
        Ok(())
    }
    #[cfg(test)]
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
        crate::resource_budget::ResourceBudget::automatic()?.check(
            self.models.source_bytes(),
            next,
            0,
        )?;
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
    #[cfg(test)]
    fn insert_colormap(&mut self, which: &str, data: &[u8]) -> Result<(), String> {
        let tile = decode_png(data)?;
        if tile.width != 256 || tile.height != 256 {
            return Err("Vanilla biome colormap must be 256x256".into());
        }
        match which {
            "grass" => Arc::make_mut(&mut self.tints).grass_colormap = Some(tile),
            "foliage" => Arc::make_mut(&mut self.tints).foliage_colormap = Some(tile),
            _ => return Err("Unknown biome colormap".into()),
        }
        Ok(())
    }
    #[cfg(test)]
    fn sample_colormap(&self, biome: Option<&str>, which: &str) -> Option<[f32; 3]> {
        self.tints.sample(biome, which)
    }
    /// Within one pack, a successfully decoded modern block/ PNG wins over
    /// the legacy blocks/ alias. Invalid PNG data must NOT reserve that
    /// identity: an intact legacy sprite (or a lower-priority pack) can still
    /// render the block instead of being silently replaced by missingno.
    fn accept_texture_path(&self, id: &str, path: &str) -> bool {
        path.contains("/textures/block/") || !self.modern_textures.contains(id)
    }
    #[cfg(test)]
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
    #[cfg(test)]
    fn walk(
        &mut self,
        root: &Path,
        folder: &Path,
        memory: &mut usize,
        depth: u8,
        entries: &mut usize,
        java_only: bool,
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
                self.walk(root, &path, memory, depth + 1, entries, java_only)?;
            } else if ty.is_file() {
                let Ok(relative) = path.strip_prefix(root) else {
                    continue;
                };
                let name = relative.to_string_lossy().replace('\\', "/");
                let Some(asset) = PackAsset::classify(&name) else {
                    continue;
                };
                if java_only && !asset.is_java_resource(&name) {
                    continue;
                }
                let max_size =
                    asset.max_bytes(crate::resource_budget::ResourceBudget::automatic()?);
                let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
                if meta.len() > max_size {
                    if matches!(asset, PackAsset::NumericBiomes) {
                        return Err(format!("数字 biome 颜色表超过单项预算：{name}"));
                    }
                    Arc::make_mut(&mut self.warnings).push(format!("Skipped oversize {name}"));
                    continue;
                }
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                if bytes.len() as u64 > max_size {
                    if matches!(asset, PackAsset::NumericBiomes) {
                        return Err(format!("数字 biome 颜色表超过单项预算：{name}"));
                    }
                    Arc::make_mut(&mut self.warnings).push(format!("Skipped oversize {name}"));
                    continue;
                }
                self.ingest_asset(&name, asset, bytes, memory)?;
            }
        }
        Ok(())
    }
    #[cfg(test)]
    fn read_zip<R: Read + Seek>(
        &mut self,
        r: R,
        memory: &mut usize,
        java_only: bool,
    ) -> Result<(), String> {
        let mut archive = zip::ZipArchive::new(r).map_err(|e| e.to_string())?;
        if archive.len() > 1_000_000 {
            return Err("ZIP exceeds 1,000,000 entries safety budget".into());
        }
        for i in 0..archive.len() {
            let mut member = match archive.by_index(i) {
                Ok(member) => member,
                Err(error) => {
                    Arc::make_mut(&mut self.warnings)
                        .push(format!("Skipped unreadable ZIP entry {i}: {error}"));
                    continue;
                }
            };
            let name = member.name().to_string();
            let Some(asset) = PackAsset::classify(&name) else {
                continue;
            };
            if java_only && !asset.is_java_resource(&name) {
                continue;
            }
            let max_size = asset.max_bytes(crate::resource_budget::ResourceBudget::automatic()?);
            if member.size() > max_size {
                if matches!(asset, PackAsset::NumericBiomes) {
                    return Err(format!("数字 biome 颜色表超过单项预算：{name}"));
                }
                Arc::make_mut(&mut self.warnings).push(format!("Skipped oversize {name}"));
                continue;
            }
            let mut bytes = Vec::new();
            if let Err(error) = member.by_ref().take(max_size + 1).read_to_end(&mut bytes) {
                if matches!(asset, PackAsset::NumericBiomes) {
                    return Err(format!("数字 biome 颜色表读取失败：{error}"));
                }
                Arc::make_mut(&mut self.warnings)
                    .push(format!("Skipped unreadable asset {name}: {error}"));
                continue;
            }
            if bytes.len() as u64 > max_size {
                if matches!(asset, PackAsset::NumericBiomes) {
                    return Err(format!("数字 biome 颜色表超过单项预算：{name}"));
                }
                Arc::make_mut(&mut self.warnings).push(format!("Skipped oversize {name}"));
                continue;
            }
            self.ingest_asset(&name, asset, bytes, memory)?;
        }
        Ok(())
    }
    /// Resource packs may place metadata before or after the PNG in ZIPs;
    /// resolve order-independently only after all namespaced assets load.
    #[cfg(test)]
    fn bake_mtl_masks(&mut self) {
        let declared = self.models.opacity_maps();
        if declared.len() > 8192 {
            Arc::make_mut(&mut self.warnings).push("Too many MTL opacity materials".into());
            return;
        }
        let mut memory: usize = self.tiles.values().map(|t| t.rgba.len()).sum();
        for (model, material, diffuse, mask) in declared {
            let (Some(diffuse_id), Some(mask_id)) = (
                material_texture_id(&model, &diffuse),
                material_texture_id(&model, &mask),
            ) else {
                Arc::make_mut(&mut self.warnings)
                    .push(format!("Invalid MTL texture for {material}"));
                continue;
            };
            let (Some(base), Some(alpha)) = (self.tiles.get(&diffuse_id), self.tiles.get(&mask_id))
            else {
                Arc::make_mut(&mut self.warnings)
                    .push(format!("Missing map_Kd/map_d for {material}"));
                continue;
            };
            let bytes = base.rgba.len();
            if memory.checked_add(bytes).is_none_or(|v| {
                crate::resource_budget::ResourceBudget::automatic()
                    .and_then(|b| b.check(self.models.source_bytes(), v, 0))
                    .is_err()
            }) {
                Arc::make_mut(&mut self.warnings)
                    .push("MTL opacity maps exceed atlas memory budget".into());
                break;
            }
            let tile = combine_opacity(base, alpha);
            let synthetic = format!("mcedit2:mtl_opacity_{}", self.tiles.len());
            Arc::make_mut(&mut self.tiles).insert(synthetic.clone(), tile);
            Arc::make_mut(&mut self.models).set_opacity_overlay(model, material, synthetic);
            memory += bytes;
        }
    }
    #[cfg(test)]
    fn finalize_animations(&mut self) {
        let source = std::mem::take(&mut self.animations);
        let candidates = Arc::try_unwrap(source).unwrap_or_else(|shared| (*shared).clone());
        self.static_animation_count = 0;
        for (id, candidate) in candidates {
            let Some(meta) = self.metadata.get(&id) else {
                // 没有动画声明的竖向 PNG 是完整静态图片，不能丢弃后续行。
                let width = candidate.frames[0].width;
                let height = candidate.frames.iter().map(|frame| frame.height).sum();
                let rgba = candidate
                    .frames
                    .into_iter()
                    .flat_map(|frame| frame.rgba)
                    .collect();
                Arc::make_mut(&mut self.tiles).insert(
                    id,
                    Tile {
                        width,
                        height,
                        rgba,
                    },
                );
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
                Err(e) => {
                    Arc::make_mut(&mut self.warnings).push(format!("Skipped animation {id}: {e}"))
                }
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
        // 生产源按可播放时序计数；实验导入器仍独立报告其静态选择数量。
        self.static_animation_count + self.live_animations.len()
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
    /// Legacy experimental importer exists for regression tests only.
    /// Production renderer and later plugin host must not invoke it.
    #[cfg(test)]
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_stack(&[path.to_path_buf()])
    }

    /// Merge client and resource packs in increasing priority. Only source
    /// identity maps are overlaid; the GPU atlas is allocated once at the end.
    #[cfg(test)]
    pub fn open_stack(paths: &[std::path::PathBuf]) -> Result<Self, String> {
        Self::open_stack_with_policy(paths, false)
    }
    /// Default UI/automatic-import path. No OBJ, glTF, GLB, MTL or custom
    /// format is auto-imported, even if those files occur inside a Mod JAR.
    #[cfg(test)]
    pub fn open_stack_java(paths: &[std::path::PathBuf]) -> Result<Self, String> {
        Self::open_stack_java_controlled(paths, &LoadControl::default())
    }
    pub(crate) fn open_stack_java_controlled(
        paths: &[std::path::PathBuf],
        control: &LoadControl,
    ) -> Result<Self, String> {
        java_assets::open(paths, control)
    }
    #[cfg(test)]
    fn open_stack_with_policy(
        paths: &[std::path::PathBuf],
        java_only: bool,
    ) -> Result<Self, String> {
        if paths.is_empty() || paths.iter().any(|p| p.as_os_str().is_empty()) {
            return Err("Select at least one resource-pack directory or ZIP".into());
        }
        let mut out = Self::default();
        let mut memory = 0;
        for path in paths {
            let started = std::time::Instant::now();
            eprintln!("[assets] 开始读取 {}", path.display());
            if let Some(version) = crate::legacy_render::Version::from_client_path(path) {
                Arc::make_mut(&mut out.models).legacy = Some(version.registry());
                Arc::make_mut(&mut out.tints).begin_legacy();
                Arc::make_mut(&mut out.warnings).push(
                    "旧版数字 biome 默认使用 MCEdit2 静态气候表，显式资源包颜色表可整份替换默认表。"
                        .into(),
                );
            }
            // Canonical block/ beats legacy blocks/ within one pack only;
            // a higher pack's legacy sprite must still replace base sprites.
            out.modern_textures.clear();
            out.modern_metadata.clear();
            out.layer_png.clear();
            out.layer_metadata.clear();
            if path.is_dir() {
                let root = path.canonicalize().map_err(|e| e.to_string())?;
                let mut entries = 0;
                out.walk(
                    &root,
                    &root.join("assets"),
                    &mut memory,
                    0,
                    &mut entries,
                    java_only,
                )?;
            } else if path.is_file() {
                if !path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("zip") || ext.eq_ignore_ascii_case("jar")
                    })
                {
                    return Err(
                        "Only directories, .zip packs and .jar client assets are supported".into(),
                    );
                }
                out.read_zip(
                    File::open(path).map_err(|e| e.to_string())?,
                    &mut memory,
                    java_only,
                )?;
            } else {
                return Err(format!(
                    "Resource-pack layer does not exist: {}",
                    path.display()
                ));
            }
            // A new static image must never inherit lower-priority animated
            // PNG metadata. Metadata-only overlays are deliberately allowed.
            for id in &out.layer_png {
                if !out.layer_metadata.contains(id) {
                    out.metadata.remove(id);
                }
            }
            eprintln!(
                "[assets] 已读取 {}：累计 {} 个 PNG / {} 个模型 / {} MiB，耗时 {:?}",
                path.display(),
                out.tiles.len(),
                out.models.count(),
                memory / (1024 * 1024),
                started.elapsed()
            );
        }
        if out.tiles.is_empty() {
            return Err("Packs contain no supported assets/*/textures/block/*.png".into());
        }
        Arc::make_mut(&mut out.tiles).insert(MISSING_TEXTURE_ID.into(), missing_texture());
        // Resolve pack-local external .bin references only after all ZIP/JAR
        // entries and overlay layers have been added, independent of order.
        Arc::make_mut(&mut out.warnings).extend(Arc::make_mut(&mut out.models).finish_gltf());
        out.finalize_animations();
        out.bake_mtl_masks();
        eprintln!("[assets] 开始建立图集：{} 个纹理", out.tiles.len());
        let chosen = mixed_layout(&out.tiles)?;
        let source_max = out
            .tiles
            .values()
            .map(|tile| tile.width.max(tile.height))
            .max()
            .unwrap_or(16);
        if source_max > chosen.max_cell {
            Arc::make_mut(&mut out.warnings).push(format!(
                "GPU atlas downsampled block sprites from {source_max}px to {}px to fit memory",
                chosen.max_cell
            ));
        }
        out.atlas = Some(Arc::new(TextureAtlas::create_at(
            &out.tiles,
            &out.animations,
            0,
        )));
        eprintln!("[assets] 资源准备完成：{} 个图集页", chosen.pages);
        Ok(out)
    }
    pub fn count(&self) -> usize {
        self.tiles.len()
            + self
                .fonts
                .image_ids()
                .filter(|id| !self.tiles.contains_key(*id))
                .count()
    }
    pub fn atlas(&self) -> Option<&std::sync::Arc<TextureAtlas>> {
        self.atlas.as_ref()
    }
    pub(crate) fn render_snapshot(&self) -> Result<Self, String> {
        let mut bank = self.clone();
        bank.atlas = self
            .atlas
            .as_ref()
            .map(|atlas| atlas.snapshot())
            .transpose()?
            .map(Arc::new);
        Ok(bank)
    }
    pub(crate) fn with_runtime_atlas(&self, atlas: Arc<TextureAtlas>) -> Result<Self, String> {
        let base = self.atlas.as_ref().ok_or("基础图集尚未建立")?;
        // 运行时图集在后台编译时完成并缓存严格证明；界面交接只读已有结果。
        if !atlas_compatible(&atlas, base) {
            return Err("运行时图集与基础 UV 或页面不兼容".into());
        }
        let mut bank = self.clone();
        bank.atlas = Some(atlas);
        Ok(bank)
    }
    /// Sample the same packed PNG alpha bytes used by the GPU. The hit test
    /// only uses it as a conservative pixel mask; no world data is mutated.
    pub fn atlas_pixel(&self, uv: [f32; 2], page: u32) -> Option<[u8; 4]> {
        if !uv[0].is_finite() || !uv[1].is_finite() {
            return None;
        }
        match self.atlas.as_ref()?.sample_lod(uv, page, 0., false, 0) {
            Ok(pixel) => Some(pixel.map(|v| (v * 255.).round() as u8)),
            Err(error) => {
                log::error!("[textures] 像素采样失败：{error}");
                None
            }
        }
    }
    /// Canvas 着色与投影拾取共用相同过滤器和浮点 Alpha，避免边缘覆盖率分歧。
    pub(crate) fn atlas_sample(
        &self,
        uv: [f32; 2],
        page: u32,
        quality: crate::gpu_mesh::Quality,
    ) -> Option<[f32; 4]> {
        let atlas = self.atlas.as_ref()?;
        match crate::canvas_raster::sample(Some(atlas), uv, page, quality) {
            Ok(pixel) => Some(pixel),
            Err(error) => {
                log::error!("[textures] 图集采样失败：{error}");
                None
            }
        }
    }
    pub fn atlas_size(&self) -> Option<(u32, u32)> {
        self.atlas
            .as_ref()?
            .pages
            .first()
            .map(|a| (a.width, a.height))
    }
    pub fn face_cull_direction_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element_index: usize,
        pos: impl Into<crate::resource_models::ModelSample> + Copy,
    ) -> Option<Option<crate::isometric::Face>> {
        self.models
            .face_cull_direction_at(state, face, element_index, pos)
    }
    pub fn face_defined_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element_index: usize,
        pos: impl Into<crate::resource_models::ModelSample> + Copy,
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
        pos: impl Into<crate::resource_models::ModelSample> + Copy,
    ) -> Option<([[f32; 2]; 4], AlphaMode, u32)> {
        let spec = self.models.element_face_at(state, face, element_index, pos);
        let authored = spec.as_ref().and_then(|s| s.render_type);
        let (id, uv, rot, perm) = match spec {
            Some(spec) => {
                let id = if self
                    .source_image(&spec.texture, authored.map_or(0, AlphaMode::gpu_bits))
                    .is_some()
                {
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
                    if state.name.starts_with("minecraft:") || state.name.starts_with("numeric:") {
                        self.resolve_id_at(state, face, pos)
                            .unwrap_or_else(|| MISSING_TEXTURE_ID.into())
                    } else {
                        // No name-derived fallback for modded blocks. Without
                        // an exact Java JSON face the source model is unknown.
                        MISSING_TEXTURE_ID.to_owned()
                    },
                    [0.0, 0.0, 16.0, 16.0],
                    0,
                    face.java_uv_indices(),
                )
            }
        };
        let region = self
            .atlas
            .as_ref()?
            .material(&id, authored.map_or(0, AlphaMode::gpu_bits))?;
        let alpha = if let Some(mode) = authored {
            mode
        } else {
            self.sprite_alpha(&id)?
        };
        Some((
            orient_uv(region.corners_for(uv, rot), perm),
            alpha,
            region.page,
        ))
    }
    /// Raw source UVs for arbitrary JSON models, before atlas clipping.
    /// Generic static tiling is independent of individual block identities.
    pub fn atlas_repeating_face_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element_index: usize,
        pos: impl Into<crate::resource_models::ModelSample> + Copy,
    ) -> Option<([[f32; 2]; 4], AtlasUv, Option<texture_sampler::Sampler>)> {
        let spec = self
            .models
            .element_face_at(state, face, element_index, pos)?;
        if spec.uv.iter().all(|v| (0.0..=16.0).contains(v)) {
            return None;
        }
        let mode = spec.render_type.map_or(0, AlphaMode::gpu_bits);
        let id = if self.source_image(&spec.texture, mode).is_some() {
            spec.texture.as_str()
        } else {
            MISSING_TEXTURE_ID
        };
        let source = self.atlas.as_ref()?;
        let atlas = source.material(id, mode)?.clone();
        let sampler = match source.native_sampler(&atlas, mode) {
            Ok(sampler) => sampler,
            Err(error) => {
                log::error!("[models] {} 的源采样无效：{error}", state.name);
                return None;
            }
        };
        let [u0, v0, u1, v1] = spec.uv;
        let mut corners = [[u0, v0], [u1, v0], [u1, v1], [u0, v1]];
        for _ in 0..(spec.rotation / 90) {
            corners.rotate_right(1);
        }
        Some((orient_uv(corners, spec.uvlock), atlas, sampler))
    }
    /// 原版颜色提供器与所选客户端内置定义共同决定点采样颜色；保持全亮，不引入阴影。
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
    pub(crate) fn native_fluid_tint(
        &self,
        kind: mcedit2_world::fluid_surface::Kind,
        biome: Option<&str>,
    ) -> [f32; 3] {
        match kind {
            mcedit2_world::fluid_surface::Kind::Water => {
                self.tints.biome(biome, "water", (0, 0, 0))
            }
            mcedit2_world::fluid_surface::Kind::Lava => [1.0; 3],
        }
    }
    pub(crate) fn numeric_biome_choices(&self) -> Option<(&str, Vec<NumericBiomeChoice>)> {
        let profile = self.tints.numeric.as_ref()?;
        Some((
            &profile.source,
            profile
                .choices()
                .map(|(id, name)| NumericBiomeChoice {
                    id,
                    name: name.to_owned(),
                })
                .collect(),
        ))
    }
    #[cfg(test)]
    pub fn face_tint_with_biome_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element: usize,
        pos: (i32, i32, i32),
        biome: Option<&str>,
    ) -> [f32; 3] {
        self.face_tint_index(state, face, element, pos)
            .map_or([1.; 3], |index| {
                self.tints
                    .block(self.models.render_state(state), index, biome, pos)
            })
    }
    pub(crate) fn tint_context<'a>(
        &'a self,
        source: &'a mcedit2_world::biome_source::Region,
    ) -> biome_colors::Context<'a> {
        biome_colors::Context::new(self, source)
    }
    pub(crate) fn dynamic_tint(&self, state: &State, index: u32) -> bool {
        self.tints.dynamic(self.render_state(state), index)
    }
    pub(crate) fn face_tint_index(
        &self,
        state: &State,
        face: crate::isometric::Face,
        element: usize,
        pos: impl Into<crate::resource_models::ModelSample> + Copy,
    ) -> Option<u32> {
        let state = self.models.render_state(state);
        // Vanilla fluids are rendered by a built-in fluid renderer and do
        // not necessarily have block-model tintindex entries. Respect the
        // resource sprite itself: missing textures must remain purple/black.
        if state.name == "minecraft:water" {
            return self.resolve_id_at(state, face, pos).map(|_| 0);
        }
        let spec = self.models.element_face_at(state, face, element, pos);
        let tint_index = if let Some(spec) = spec {
            if self
                .source_image(
                    &spec.texture,
                    spec.render_type.map_or(0, AlphaMode::gpu_bits),
                )
                .is_none()
            {
                return None;
            }
            spec.tint_index?
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
                return None;
            }
            let id = self.resolve_id_at(state, face, pos)?;
            if !self.tiles.contains_key(&id) || !known_vanilla_biome_tint(state, face) {
                return None;
            }
            0
        };
        Some(tint_index)
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
        pos: impl Into<crate::resource_models::ModelSample> + Copy,
    ) -> Option<Vec<StaticObjTriangle>> {
        if self.declarative_block_replaces(state) == Some(true) {
            return None;
        }
        let chosen = self.models.obj_at(state, pos)?;
        let atlas = self.atlas.as_ref()?;
        let mut out = Vec::new();
        for tri in chosen {
            let material_flags =
                tri.material_flags | tri.render_type.map_or(0, AlphaMode::gpu_bits);
            let id = if self.source_image(&tri.texture, material_flags).is_some() {
                tri.texture.as_str()
            } else {
                MISSING_TEXTURE_ID
            };
            let slot = atlas.material(id, material_flags)?;
            let rgba = tri
                .colors
                .map(|color| std::array::from_fn(|i| color[i] * tri.tint[i]));
            let alpha = if rgba.iter().any(|color| color[3] < 0.999) {
                AlphaMode::Blend
            } else {
                self.source_alpha(id, material_flags)?
            };
            let mode = if tri.alpha_mode.is_some() {
                crate::material_mode::encode(
                    alpha.gpu_bits(),
                    tri.alpha_mode.as_deref(),
                    tri.alpha_cutoff,
                    tri.double_sided,
                )
            } else {
                tri.render_type.unwrap_or(alpha).gpu_bits()
                    | if tri.double_sided {
                        0
                    } else {
                        crate::material_mode::SINGLE_SIDED
                    }
            };
            let mode = mode | tri.material_flags;
            let native_clamp = match atlas.native_sampler(slot, mode) {
                Ok(sampler) => sampler.is_some_and(|sampler| sampler.clamp),
                Err(error) => {
                    log::error!("[models] {} 的材质采样无效：{error}", state.name);
                    return None;
                }
            };
            let mut emit = |xyz, uv, rgba| {
                out.push(StaticObjTriangle {
                    xyz,
                    uv,
                    page: slot.page,
                    mode,
                    rgba,
                    cull: tri.cull,
                    tint_index: (id != MISSING_TEXTURE_ID)
                        .then_some(tri.tint_index)
                        .flatten(),
                });
            };
            let valid = if native_clamp {
                crate::static_triangle_uv::clamp_colored(tri.xyz, tri.uv, rgba, slot, &mut emit)
            } else {
                crate::static_triangle_uv::tile_colored(
                    tri.xyz,
                    tri.uv,
                    rgba,
                    slot,
                    self.geometry_budget(),
                    &mut emit,
                )
            };
            if !valid {
                log::error!(
                    "[models] {} 的静态三角形坐标、颜色或重复 UV 超出范围",
                    state.name
                );
                return None;
            }
        }
        Some(out)
    }
    pub(crate) fn single_block_tint(&self, state: &State) -> Result<[f32; 3], String> {
        let state = if state.name.starts_with("numeric:") {
            &self
                .models
                .legacy
                .as_ref()
                .ok_or("数字方块颜色缺少准确旧注册表")?
                .resolve(state)
                .ok_or_else(|| format!("旧注册目录没有方块颜色来源 {}", state.name))?
                .source_state
        } else {
            state
        };
        self.tints.single_block(state)
    }
    pub(crate) fn single_block_geometry(&self, state: &State) -> Option<Vec<crate::models::Part>> {
        self.models.parts_at(state, self.single_block_sample())
    }
    pub(crate) fn single_block_sample(&self) -> crate::resource_models::ModelSample {
        crate::resource_models::ModelSample::Seed(if self.item_data_version() == Some(1343) {
            0
        } else {
            42
        })
    }
    pub(crate) fn legacy_carried_name(&self, name: &str, meta: u16) -> Result<State, String> {
        self.models
            .legacy
            .as_ref()
            .ok_or("旧搬运方块缺少准确资源注册表")?
            .named_numeric_state(name, meta)
    }
    pub(crate) fn legacy_carried_numeric(&self, id: i16, meta: u16) -> Result<State, String> {
        let source = State {
            name: format!("numeric:{id}"),
            properties: vec![("data".into(), meta.to_string())],
        };
        self.models
            .legacy
            .as_ref()
            .ok_or("旧搬运方块缺少准确资源注册表")?
            .resolve(&source)
            .ok_or_else(|| format!("旧资源注册目录没有搬运方块 {id}:{meta}"))?;
        Ok(source)
    }
    pub fn static_obj_tint(
        &self,
        state: &State,
        pos: (i32, i32, i32),
        biome: Option<&str>,
        tint_index: u32,
    ) -> [f32; 3] {
        self.tints
            .block(self.models.render_state(state), tint_index, biome, pos)
    }
    /// 位置、材质和颜色输入的稳定身份；相机移动不改变它。
    pub fn geometry_id(&self) -> (usize, usize, usize) {
        (
            Arc::as_ptr(&self.models) as usize,
            Arc::as_ptr(&self.tiles) as usize,
            Arc::as_ptr(&self.tints) as usize,
        )
    }
    /// 源快照同时锚定 Alpha 与颜色输入；读取完成前不混用新旧资源。
    pub(crate) fn visibility_id(&self) -> VisibilityIdentity {
        VisibilityIdentity {
            block_layers: self.block_layers,
            world_data: Arc::downgrade(&self.world_data),
            fonts: Arc::downgrade(&self.fonts),
            models: Arc::downgrade(&self.models),
            tiles: Arc::downgrade(&self.tiles),
            simple_tiles: Arc::downgrade(&self.simple_tiles),
            simple_samplers: Arc::downgrade(&self.simple_samplers),
            png_layouts: Arc::downgrade(&self.png_layouts),
            live_animations: Arc::downgrade(&self.live_animations),
            animations: Arc::downgrade(&self.animations),
            tints: Arc::downgrade(&self.tints),
        }
    }
    /// 与实际面发射使用同一边界判定；无 cullface、内部面与静态三角形不能随整格省略。
    pub(crate) fn has_unconditional_geometry(&self, state: &State, pos: (i32, i32, i32)) -> bool {
        if !self.world_block_visible(state) {
            return false;
        }
        if self.declarative_block_replaces(state).is_some() {
            return true;
        }
        use crate::isometric::Face;
        if self
            .models
            .obj_at(state, pos)
            .is_some_and(|triangles| !triangles.is_empty())
        {
            return true;
        }
        let display = self.render_state(state);
        let parts = self.geometry_at(state, pos).unwrap_or_else(|| {
            crate::models::boxes(display)
                .into_iter()
                .map(crate::models::Part::cube)
                .collect()
        });
        for (index, part) in parts.iter().enumerate() {
            for face in [
                Face::Top,
                Face::Bottom,
                Face::PosX,
                Face::NegX,
                Face::PosZ,
                Face::NegZ,
            ] {
                if self.models.face_defined_at(state, face, index, pos) == Some(false) {
                    continue;
                }
                match self.models.face_cull_direction_at(state, face, index, pos) {
                    Some(Some(direction)) => {
                        if !crate::isometric::on_world_boundary(part, face, direction) {
                            return true;
                        }
                    }
                    Some(None) => return true,
                    None => {
                        let (x, y, z) = face.normal();
                        let (axis, positive) = if x != 0 {
                            (0, x > 0)
                        } else if y != 0 {
                            (1, y > 0)
                        } else {
                            (2, z > 0)
                        };
                        if !part.boundary(axis, positive) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
    pub fn model_count(&self) -> usize {
        self.models.count()
    }
    pub(crate) fn sprite_alpha(&self, id: &str) -> Option<AlphaMode> {
        self.alpha_cache.get(self, id).map(|(_, animated)| animated)
    }
    pub(crate) fn has_legacy_chests(&self) -> bool {
        !self.models.legacy_chests.is_empty()
    }
    pub(crate) fn legacy_chest_state<'a>(&'a self, state: &'a State) -> Option<&'a State> {
        let state = self.models.render_state(state);
        self.models
            .legacy_chests
            .contains(&state.name)
            .then_some(state)
    }
    pub fn geometry(&self, state: &State) -> Option<Vec<crate::models::Part>> {
        self.geometry_at(state, (0, 0, 0))
    }
    pub fn geometry_at(
        &self,
        state: &State,
        pos: impl Into<crate::resource_models::ModelSample> + Copy,
    ) -> Option<Vec<crate::models::Part>> {
        if self.declarative_block_replaces(state) == Some(true) {
            return Some(Vec::new());
        }
        if self.models.vanilla_banner_supported(state)
            || self.models.vanilla_skull_supported(state)
            || self.models.vanilla_sign_supported(state)
            || self.models.vanilla_piston_supported(state)
        {
            return Some(Vec::new());
        }
        if self.models.unsupported_legacy_skull(state) {
            return Some(vec![crate::models::Part::cube(crate::models::Box3::FULL)]);
        }
        self.models.parts_at(state, pos).or_else(|| {
            self.models
                .declares_state(state)
                .then(|| vec![crate::models::Part::cube(crate::models::Box3::FULL)])
        })
    }
    pub(crate) fn model_variant_plan(
        &self,
        state: &State,
    ) -> Option<crate::resource_models::VariantPlan> {
        self.models.variant_plan(state)
    }
    #[cfg(test)]
    pub(crate) fn model_position_dependent(&self, state: &State) -> bool {
        self.models
            .variant_plan(state)
            .is_some_and(|plan| plan.position_dependent())
    }
    pub fn model_variant_key(&self, state: &State, pos: (i32, i32, i32)) -> u64 {
        self.models.weighted_selection_key(state, pos)
    }
    /// 动态方块实体按实际覆盖后的像素合成材质，不绕过资源包顺序。
    pub fn get(&self, id: &str) -> Option<&Tile> {
        self.tiles.get(id).or_else(|| self.fonts.image(id))
    }
    /// 只接管已核实的原版实体占位模型；显式资源模型继续走地形管线。
    pub(crate) fn vanilla_banner_supported(&self, state: &State) -> bool {
        self.models.vanilla_banner_supported(state)
    }
    pub(crate) fn vanilla_skull_supported(&self, state: &State) -> bool {
        self.models.vanilla_skull_supported(state)
    }
    pub(crate) fn vanilla_sign_supported(&self, state: &State) -> bool {
        self.models.vanilla_sign_supported(state)
    }
    pub(crate) fn vanilla_piston_supported(&self, state: &State) -> bool {
        self.models.vanilla_piston_supported(state)
    }
    /// 只返回准确资源版本与实际占位模型共同确认的旧动态方块状态，用于扫描缺失 BE。
    pub(crate) fn block_visual_numeric_states(&self) -> BTreeSet<(u16, u8)> {
        self.models
            .legacy
            .iter()
            .flat_map(|registry| registry.block_visual_states())
            .filter_map(|(key, state)| {
                (self.models.vanilla_banner_supported(state)
                    || self.models.vanilla_skull_supported(state)
                    || self.models.vanilla_sign_supported(state)
                    || self.models.vanilla_piston_supported(state))
                .then_some(key)
            })
            .collect()
    }
    #[cfg(test)]
    pub fn resolve_id(&self, state: &State, face: crate::isometric::Face) -> Option<String> {
        self.resolve_id_at(state, face, (0, 0, 0))
    }
    pub(crate) fn procedural_fluid_supported(&self, state: &State, pos: (i32, i32, i32)) -> bool {
        !self.models.declares_state(state) || self.models.parts_at(state, pos).is_some()
    }
    pub fn resolve_id_at(
        &self,
        state: &State,
        face: crate::isometric::Face,
        pos: impl Into<crate::resource_models::ModelSample> + Copy,
    ) -> Option<String> {
        if self.models.unsupported_legacy_skull(state) {
            return None;
        }
        // 已有声明但无法解释时显示缺失模型，不能借用同名原版 PNG 掩盖专用渲染器缺口。
        if self.models.declares_state(state) && self.models.parts_at(state, pos).is_none() {
            return None;
        }
        if state.name.starts_with("numeric:") {
            if let Some(key) = self.models.texture_at(state, face, pos) {
                if self.tiles.contains_key(&key) {
                    return Some(key);
                }
            }
        }
        let state = self.models.render_state(state);
        let (ns, name) = state.name.split_once(':')?;
        if ns == "numeric" {
            return None;
        }
        // Only the minecraft namespace has the built-in legacy sprite
        // conventions. Foreign namespaces require an actual resolved JSON
        // element face and must not borrow same-named PNGs.
        if ns != "minecraft" {
            return self
                .models
                .element_face_at(state, face, 0, pos)
                .map(|spec| spec.texture)
                .filter(|id| self.tiles.contains_key(id));
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
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Materials {
        animation_tests::verify_materials();
    }
    if category == crate::test_categories::Category::Rendering {
        animation_tests::verify_rendering();
    }
    if category == crate::test_categories::Category::Background {
        animation_tests::verify_background();
    }
    tests::run_test_category(category);
    java_assets::run_test_category(category);
    builtin_tests::run_test_category(category);
    java_obj_tests::run_test_category(category);
    shulker_tests::run_test_category(category);
    stack_tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared_alpha_types_preserve_coverage_and_blended_tripwire() {
        let tripwire = AlphaMode::from_render_type("tripwire").unwrap();
        assert_eq!(tripwire.gpu_bits() & 3, 2);
        let coverage = |mode: AlphaMode, alpha: u8| {
            crate::material_mode::covered_alpha_f32(mode.gpu_bits(), alpha as f32 / 255.)
        };
        assert_eq!(coverage(tripwire, 25), None);
        assert_eq!(coverage(tripwire, 26), Some(26. / 255.));
        assert_eq!(coverage(tripwire, 128), Some(128. / 255.));
        assert_eq!(coverage(AlphaMode::AuthoredOpaque, 0), Some(1.));
        assert_eq!(coverage(AlphaMode::CutoutLow, 25), None);
        assert_eq!(coverage(AlphaMode::CutoutLow, 26), Some(1.));
        assert_eq!(coverage(AlphaMode::Cutout, 127), None);
        assert_eq!(coverage(AlphaMode::Cutout, 128), Some(1.));
        assert_eq!(coverage(AlphaMode::Blend, 0), None);
        assert_eq!(coverage(AlphaMode::Blend, 1), Some(1. / 255.));
        assert!(AlphaMode::from_render_type("test:unknown").is_err());
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

    fn vanilla_colormap_samples_registered_climate_and_skips_unknown_mods() {
        let mut tints = biome_colors::Resources::default();
        tints.grass_colormap = Some(Tile {
            width: 256,
            height: 256,
            rgba: [12, 84, 192, 255].repeat(256 * 256),
        });
        let bank = TextureBank {
            tints: Arc::new(tints),
            ..TextureBank::default()
        };
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

    fn missing_named_block_renders_checker_atlas_and_missing_json_face_stays_absent() {
        use crate::isometric::Face;
        let mut bank = TextureBank::default();
        let mut tiles = BTreeMap::new();
        tiles.insert(MISSING_TEXTURE_ID.to_string(), missing_texture());
        bank.atlas = Some(Arc::new(TextureAtlas::create(&tiles)));
        bank.tiles = Arc::new(tiles);
        let unknown = state("minecraft:unknown_block", vec![]);
        let (_, mode, page) = bank
            .atlas_face_style_at(&unknown, Face::Top, 0, (0, 0, 0))
            .expect("unknown block should use missing texture");
        assert_eq!(mode, AlphaMode::Opaque);
        assert!(bank.atlas().unwrap().page(page).is_some());
        // 未知插件渲染器覆盖原版 ID 时，同名 PNG 也不能冒充有效模型。
        let models = Arc::make_mut(&mut bank.models);
        for (path, json) in [
            (
                "assets/minecraft/blockstates/stone.json",
                r#"{"variants":{"":{"model":"minecraft:block/custom"}}}"#,
            ),
            (
                "assets/demo/blockstates/machine.json",
                r#"{"variants":{"":{"model":"demo:block/custom"}}}"#,
            ),
            (
                "assets/minecraft/models/block/custom.json",
                r##"{"loader":"demo:special","textures":{"all":"minecraft:block/stone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all"}}}]}"##,
            ),
            (
                "assets/demo/models/block/custom.json",
                r#"{"parent":"minecraft:block/custom"}"#,
            ),
        ] {
            models.insert(path, json.as_bytes()).unwrap();
        }
        let mut tiles = (*bank.tiles).clone();
        tiles.insert(
            "minecraft:stone".into(),
            Tile {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
            },
        );
        bank.atlas = Some(Arc::new(TextureAtlas::create(&tiles)));
        bank.tiles = Arc::new(tiles);
        for state in [
            state("minecraft:stone", vec![]),
            state("demo:machine", vec![]),
        ] {
            assert!(bank.resolve_id_at(&state, Face::Top, (0, 0, 0)).is_none());
            let parts = bank.geometry_at(&state, (0, 0, 0)).unwrap();
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].bounds, crate::models::Box3::FULL);
            for face in [
                Face::Top,
                Face::Bottom,
                Face::PosX,
                Face::NegX,
                Face::PosZ,
                Face::NegZ,
            ] {
                let (uv, _, _) = bank
                    .atlas_face_style_at(&state, face, 0, (0, 0, 0))
                    .unwrap();
                let expected = bank
                    .atlas()
                    .unwrap()
                    .entries
                    .get(MISSING_TEXTURE_ID)
                    .unwrap();
                assert_eq!(
                    uv,
                    orient_uv(
                        expected.corners_for([0., 0., 16., 16.], 0),
                        face.java_uv_indices()
                    )
                );
            }
        }
    }

    fn missing_texture_is_opaque_minecraft_purple_black_checker() {
        let tile = missing_texture();
        assert_eq!(tile.alpha_mode(), AlphaMode::Opaque);
        assert_eq!(tile.pixel(0.1, 0.1), [248, 0, 248, 255]);
        assert_eq!(tile.pixel(0.9, 0.1), [0, 0, 0, 255]);
        assert_eq!(tile.pixel(0.1, 0.9), [0, 0, 0, 255]);
        assert_eq!(tile.pixel(0.9, 0.9), [248, 0, 248, 255]);
    }

    fn original_glass_cutout_and_translucent_stained_glass_remain_distinct() {
        use mcedit2_world::surface::VoxelMaterial;
        let clear = state("minecraft:glass", vec![]);
        let stained = state("minecraft:blue_stained_glass", vec![]);
        assert_eq!(
            classify_block_alpha(&clear, VoxelMaterial::Transparent, AlphaMode::Cutout),
            (AlphaMode::CutoutLow, 1.0)
        );
        assert_eq!(
            classify_block_alpha(&stained, VoxelMaterial::Transparent, AlphaMode::Blend),
            (AlphaMode::DeclaredBlend, 1.0)
        );
        let ice = state("minecraft:ice", vec![]);
        assert_eq!(
            classify_block_alpha(&ice, VoxelMaterial::Transparent, AlphaMode::Opaque),
            (AlphaMode::DeclaredBlend, 1.0)
        );
    }

    fn leaves_cutout_is_not_forced_into_transparent_blending() {
        use mcedit2_world::surface::VoxelMaterial;
        let state = State {
            name: "minecraft:oak_leaves".into(),
            properties: vec![],
        };
        assert_eq!(
            classify_block_alpha(&state, VoxelMaterial::Foliage, AlphaMode::Cutout),
            (AlphaMode::DeclaredCutoutMipped, 1.0)
        );
    }

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

    fn atlas_packs_sorted_ids_without_cross_tile_color_bleed() {
        let mut tiles = BTreeMap::new();
        tiles.insert("a:first".into(), solid([255, 0, 0, 255]));
        tiles.insert("b:second".into(), solid([0, 0, 255, 128]));
        let atlas = TextureAtlas::create(&tiles);
        assert!(atlas.pages[0].width >= MIN_ATLAS_CELL + 2);
        assert!(atlas.pages[0].height >= MIN_ATLAS_CELL + 2);
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
        Arc::make_mut(&mut bank.models)
            .insert(
                "assets/test/blockstates/casing.json",
                br#"{"variants":{"":{"model":"test:block/casing"}}}"#,
            )
            .unwrap();
        Arc::make_mut(&mut bank.models)
            .insert(
                "assets/test/models/block/casing.json",
                br#"{"parent":"minecraft:block/cube_all","textures":{"all":"test:block/casing"}}"#,
            )
            .unwrap();
        bank.atlas = Some(std::sync::Arc::new(TextureAtlas::create(&bank.tiles)));
        let state = state("test:casing", vec![]);
        let (uv, alpha, page) = bank.atlas_face_style(&state, Face::Top, 0).unwrap();
        assert_eq!(page, 0);
        assert_eq!(alpha, AlphaMode::Cutout);
        assert_eq!(bank.alpha_mode(&state, Face::Top), Some(alpha));
        let first = bank.atlas_uv(&state, Face::Top).unwrap();
        assert_eq!(uv[0], first.start);
    }

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

    fn variable_resolution_paging_preserves_hd_source_tiles() {
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
        for (name, uv) in atlas.entries.iter() {
            let page = atlas.page(uv.page).unwrap();
            let [u, v] = uv.sample(0.5, 0.5);
            let offset = (((v * page.height as f32) as usize) * page.width as usize
                + (u * page.width as f32) as usize)
                * 4;
            let source = name.strip_prefix("@simple/").unwrap_or(name);
            let expected = if source == "z:large" {
                212
            } else {
                source
                    .strip_prefix("a:small_")
                    .unwrap()
                    .parse::<u16>()
                    .unwrap() as u8
            };
            assert_eq!(page.rgba[offset], expected, "{name}");
        }
    }

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

    fn invalid_png_header_rejected_before_allocating_pixels() {
        assert!(decode_png(b"not a png").is_err());
        let mut bytes = vec![0u8; 32];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[16..20].copy_from_slice(&10000u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes());
        assert!(decode_png(&bytes).unwrap_err().contains("dimensions"));
    }

    fn material_map_d_combines_luminance_and_png_alpha_per_pixel() {
        let diffuse = Tile {
            width: 2,
            height: 1,
            rgba: vec![120, 180, 240, 255, 30, 60, 90, 128],
        };
        let alpha = Tile {
            width: 2,
            height: 1,
            rgba: vec![0, 0, 0, 255, 255, 255, 255, 64],
        };
        let output = combine_opacity(&diffuse, &alpha);
        assert_eq!(&output.rgba[0..3], &[120, 180, 240]);
        assert_eq!(output.rgba[3], 0);
        assert_eq!(&output.rgba[4..7], &[30, 60, 90]);
        assert_eq!(output.rgba[7], 32);
    }

    fn opacity_material_texture_ids_cannot_escape_pack_namespace() {
        assert_eq!(
            material_texture_id("model:example:block/widget", "example:item/mask"),
            Some("example:item/mask".into())
        );
        assert_eq!(
            material_texture_id("model:example:block/widget", "item/opacity"),
            Some("example:item/opacity".into())
        );
        assert_eq!(
            material_texture_id("model:example:block/widget", "../secret"),
            None
        );
    }

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
        assert_eq!(
            texture_id("assets/create.addon/textures/panel.png"),
            Some("create.addon:panel".into())
        );
    }

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
            texture_id("assets/minecraft/textures/colormap/grass.png"),
            None
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

    fn external_assets_directory_symlink_is_not_traversed() {
        use std::os::unix::fs::symlink;
        let root =
            std::env::temp_dir().join(format!("mcedit2-linked-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("pack")).unwrap();
        std::fs::create_dir_all(root.join("external")).unwrap();
        symlink(root.join("external"), root.join("pack/assets")).unwrap();
        let error = TextureBank::open(&root.join("pack")).unwrap_err();
        assert!(error.contains("Linked resource-pack assets directory"));
        let _ = std::fs::remove_dir_all(&root);
    }

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

    fn nearest_sampler_clamps_and_preserves_alpha() {
        let tile = Tile {
            width: 2,
            height: 2,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 160, 7, 8, 9, 255, 10, 11, 12, 0],
        };
        assert_eq!(tile.pixel(0.75, 0.25), [4, 5, 6, 160]);
        assert_eq!(tile.pixel(9.0, 9.0), [10, 11, 12, 0]);
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Materials {
            numeric_biome_colors::validate_numeric_pipeline();
            biome_colors::validate_world_color_pipeline();
            declared_alpha_types_preserve_coverage_and_blended_tripwire();
            vanilla_colormap_samples_registered_climate_and_skips_unknown_mods();
            grass_and_water_tint_use_original_biome_name();
            vanilla_vegetation_without_json_parent_uses_only_real_sprite_tints();
            water_tint_respects_available_original_sprite_and_missing_checker();
            fluid_top_and_side_use_distinct_original_still_and_flow_sprites();
            minecraft_animation_metadata_uses_custom_order_and_tick_lengths();
            animated_atlas_updates_gpu_and_canvas_source_from_one_tick();
            meta_file_order_is_independent_of_image_loading();
            static_source_frame_obeys_mcmeta_nonzero_first_index();
            cutout_atlas_avoids_black_filter_fringe_without_changing_alpha();
            model_face_without_texture_cannot_borrow_another_named_side();
            missing_named_block_renders_checker_atlas_and_missing_json_face_stays_absent();
            missing_texture_is_opaque_minecraft_purple_black_checker();
            original_glass_cutout_and_translucent_stained_glass_remain_distinct();
            leaves_cutout_is_not_forced_into_transparent_blending();
            modern_texture_and_metadata_paths_take_priority_regardless_of_order();
            modern_and_pre_flattening_vanilla_block_paths_share_identity();
            atlas_packs_sorted_ids_without_cross_tile_color_bleed();
            atlas_alpha_sampling_retains_transparent_holes();
            alpha_modes_distinguish_cutout_from_true_translucency();
            one_pass_gpu_face_style_preserves_cutout_and_uv_coordinates();
            uvlock_handles_corner_reflection_as_well_as_rotation();
            per_face_uv_crop_and_rotation_stay_inside_atlas_slot();
            variable_resolution_paging_preserves_hd_source_tiles();
            animation_updates_only_its_layer_and_preserves_uv();
            mixed_resolution_atlas_preserves_hd_and_small_sprites();
            sorted_large_sprite_keeps_stable_texture_id_mapping();
            sparse_hd_pack_chooses_source_resolution();
            invalid_png_header_rejected_before_allocating_pixels();
            material_map_d_combines_luminance_and_png_alpha_per_pixel();
            opacity_material_texture_ids_cannot_escape_pack_namespace();
            gltf_images_in_model_root_have_stable_noncolliding_atlas_id();
            all_model_texture_families_are_namespaced_and_collision_free();
            dotted_modded_namespace_and_texture_filename_load_without_loss();
            #[cfg(unix)]
            external_assets_directory_symlink_is_not_traversed();
            path_parser_is_namespaced_and_does_not_accept_traversal();
            exact_face_rule_and_mod_namespace();
            nearest_sampler_clamps_and_preserves_alpha();
        }
    }
}

#[cfg(test)]
#[path = "builtin_model_tests.rs"]
pub(crate) mod builtin_tests;
#[cfg(test)]
#[path = "java_obj_tests.rs"]
pub(crate) mod java_obj_tests;
#[cfg(test)]
#[path = "shulker_model_tests.rs"]
pub(crate) mod shulker_tests;
#[cfg(test)]
#[path = "texture_stack_tests.rs"]
mod stack_tests;
