//! 原图集每个材质独立下采样；绝不跨槽读取或重建原 UV。
use crate::textures::AtlasPage;
use bytemuck::{Pod, Zeroable};
use std::{collections::BTreeMap, sync::Arc};

/// WGSL storage buffer 对应结构，材质 ID 从 1 开始，0 表示无材质。
pub(crate) const MAX_LEVELS: usize = 16;
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuMaterial {
    pub base_uv: [f32; 4],
    /// 对应 LOD 1..=count；四项为辅助图像上首尾像素中心 UV。
    pub levels: [[f32; 4]; MAX_LEVELS],
    /// x/y 为 0.5/0.1 cutout 覆盖率校正，z/w 保留为零。
    pub coverage: [[f32; 4]; MAX_LEVELS],
    pub count: u32,
    /// padding[0] 保存 SimpleTexture 的 blur/clamp 与运行时强制采样标志。
    pub padding: [u32; 3],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SpriteRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug)]
pub(crate) struct MipPage {
    pub base: Arc<AtlasPage>,
    /// 每个原页一张 RGBA8Unorm 辅助纹理，各级各自有一像素边界。
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// R32Uint，0=无所属材质，N 表示 materials[N-1]。
    pub owner_width: u32,
    pub owner_height: u32,
    pub owner_scale: u32,
    pub owner: Vec<u32>,
    pub materials: Vec<GpuMaterial>,
    /// 构造时已验证互不覆盖的完整 owner 矩形，半开区间；没有证明时不得提前指定材质。
    pub(crate) owner_rects: Vec<[u32; 4]>,
}
#[derive(Clone, Copy)]
struct Level {
    material: usize,
    level: usize,
    w: u32,
    h: u32,
    x: u32,
    y: u32,
}
fn bytes(w: u32, h: u32, unit: usize) -> Result<usize, String> {
    (w as usize)
        .checked_mul(h as usize)
        .and_then(|v| v.checked_mul(unit))
        .ok_or_else(|| "mipmap 内存大小溢出".into())
}
fn allocate<T: Clone>(len: usize, value: T) -> Result<Vec<T>, String> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(len)
        .map_err(|e| format!("材质 mip 内存分配失败：{e}"))?;
    result.resize(len, value);
    Ok(result)
}
fn rgba_pixel(page: &AtlasPage, x: u32, y: u32) -> Result<[u8; 4], String> {
    if x >= page.width || y >= page.height {
        return Err("材质来源像素越界".into());
    }
    let at = ((y as usize) * (page.width as usize) + x as usize) * 4;
    let p = page.rgba.get(at..at + 4).ok_or("材质来源 RGBA 不完整")?;
    Ok([p[0], p[1], p[2], p[3]])
}
fn downsample(
    src: &[[u8; 4]],
    w: u32,
    h: u32,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<(Vec<[u8; 4]>, u32, u32), String> {
    let nw = w.div_ceil(2);
    let nh = h.div_ceil(2);
    let mut out = Vec::new();
    out.try_reserve_exact(bytes(nw, nh, 1)?)
        .map_err(|e| format!("mipmap 工作内存分配失败：{e}"))?;
    for y in 0..nh {
        if y % 32 == 0 {
            check()?;
        }
        for x in 0..nw {
            let mut alpha = 0u32;
            let mut rgb = [0u64; 3];
            let mut plain = [0u64; 3];
            let mut count = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = x * 2 + dx;
                    let sy = y * 2 + dy;
                    if sx >= w || sy >= h {
                        continue;
                    }
                    let p = src[(sy as usize) * (w as usize) + sx as usize];
                    let a = u32::from(p[3]);
                    alpha += a;
                    count += 1;
                    for c in 0..3 {
                        rgb[c] += u64::from(p[c]) * u64::from(a);
                        plain[c] += u64::from(p[c]);
                    }
                }
            }
            let mut result = [0u8; 4];
            result[3] = ((alpha + count / 2) / count) as u8;
            for c in 0..3 {
                result[c] = if alpha == 0 {
                    ((plain[c] + u64::from(count / 2)) / u64::from(count)) as u8
                } else {
                    ((rgb[c] + u64::from(alpha / 2)) / u64::from(alpha)) as u8
                };
            }
            out.push(result);
        }
    }
    Ok((out, nw, nh))
}
fn coverage(pixels: &[[u8; 4]], threshold: f32) -> f64 {
    pixels
        .iter()
        .filter(|p| f32::from(p[3]) / 255.0 >= threshold)
        .count() as f64
        / pixels.len() as f64
}
/// 在有限分辨率下选择最接近原覆盖率的非负 alpha 缩放，避免过度放大噪点。
fn coverage_scale(pixels: &[[u8; 4]], target: f64, threshold: f32) -> f32 {
    let mut hist = [0usize; 256];
    for p in pixels {
        hist[p[3] as usize] += 1;
    }
    let total = pixels.len() as f64;
    let mut above = 0usize;
    let mut best = (f64::INFINITY, f32::INFINITY, 1.0f32);
    for value in (1..=255).rev() {
        above += hist[value];
        if hist[value] == 0 {
            continue;
        }
        let scale = (threshold * 255.0 / (value as f32) + 1e-5).min(8.0);
        if (value as f32 / 255.0) * scale + 1e-7 < threshold {
            continue;
        }
        let error = (above as f64 / total - target).abs();
        let deviation = (scale - 1.0).abs();
        if error < best.0 || (error == best.0 && deviation < best.1) {
            best = (error, deviation, scale);
        }
    }
    let original = coverage(pixels, threshold);
    let error = (original - target).abs();
    if error <= best.0 {
        1.0
    } else {
        best.2
    }
}

/// 从已构建的原页和其实际槽构建辅助图；重复槽只生成一次，内存按真实字节核算。
pub(crate) fn build(
    base: Arc<AtlasPage>,
    rectangles: &[SpriteRect],
    budget: u64,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<(Arc<MipPage>, u64), String> {
    if base.width == 0 || base.height == 0 || base.rgba.len() != bytes(base.width, base.height, 4)?
    {
        return Err("mipmap 原图集页尺寸无效".into());
    }
    let mut unique = BTreeMap::<SpriteRect, ()>::new();
    for &rect in rectangles {
        unique.insert(rect, ());
    }
    if unique.is_empty() {
        return Err("mipmap 页没有材质槽".into());
    }
    let sprites: Vec<_> = unique.into_keys().collect();
    let mut owner_scale = 2u32;
    for rect in &sprites {
        if rect.width == 0 || rect.height == 0 || rect.x == 0 || rect.y == 0 {
            return Err("mipmap 材质槽缺少边界".into());
        }
        let right = rect
            .x
            .checked_add(rect.width)
            .and_then(|x| x.checked_add(1))
            .ok_or("材质槽 X 溢出")?;
        let bottom = rect
            .y
            .checked_add(rect.height)
            .and_then(|y| y.checked_add(1))
            .ok_or("材质槽 Y 溢出")?;
        if right > base.width || bottom > base.height {
            return Err("mipmap 材质槽超出图集".into());
        }
        let left = rect.x - 1;
        let top = rect.y - 1;
        // 所有左 / 上边界对齐后，不相交的矩形在 2×2 owner 网格也不相交。
        // 右 / 下为奇数时向外覆盖一格即可；实际重叠仍由后续 owner 写入拒绝。
        if left % 2 != 0 || top % 2 != 0 {
            owner_scale = 1;
        }
    }
    let owner_width = base.width.div_ceil(owner_scale);
    let owner_height = base.height.div_ceil(owner_scale);
    let mut plans = Vec::new();
    let mut scratch = 0usize;
    for (id, rect) in sprites.iter().enumerate() {
        let base_bytes = bytes(rect.width, rect.height, 4)?;
        let next_bytes = bytes(rect.width.div_ceil(2), rect.height.div_ceil(2), 4)?;
        scratch = scratch.max(
            base_bytes
                .checked_add(next_bytes)
                .ok_or("mipmap 临时内存溢出")?,
        );
        let (mut w, mut h) = (rect.width, rect.height);
        let mut n = 0;
        while w > 1 || h > 1 {
            w = w.div_ceil(2);
            h = h.div_ceil(2);
            if n >= MAX_LEVELS {
                return Err("单材质 mip 级数超出合同".into());
            }
            plans.push(Level {
                material: id,
                level: n,
                w,
                h,
                x: 0,
                y: 0,
            });
            n += 1;
        }
    }
    plans.sort_unstable_by_key(|p| {
        (
            std::cmp::Reverse(p.h),
            std::cmp::Reverse(p.w),
            p.material,
            p.level,
        )
    });
    let width = base.width.max(3);
    let mut x = 0u32;
    let mut y = 0u32;
    let mut row_height = 0u32;
    for plan in &mut plans {
        let pw = plan.w.checked_add(2).ok_or("mipmap 宽度溢出")?;
        let ph = plan.h.checked_add(2).ok_or("mipmap 高度溢出")?;
        if pw > width {
            return Err("mipmap 材质宽度超过辅助图页".into());
        }
        if x.checked_add(pw).is_none_or(|v| v > width) {
            y = y.checked_add(row_height).ok_or("mipmap 排布溢出")?;
            x = 0;
            row_height = 0;
        }
        plan.x = x;
        plan.y = y;
        x += pw;
        row_height = row_height.max(ph);
    }
    let height = y
        .checked_add(row_height)
        .ok_or("mipmap 辅助图高度溢出")?
        .max(1);
    let final_bytes = bytes(width, height, 4)?
        .checked_add(bytes(owner_width, owner_height, 4)?)
        .and_then(|n| {
            n.checked_add(sprites.len().checked_mul(
                std::mem::size_of::<GpuMaterial>() + std::mem::size_of::<[u32; 4]>(),
            )?)
        })
        .ok_or("mipmap 总大小溢出")?;
    let work_bytes = plans
        .capacity()
        .checked_mul(std::mem::size_of::<Level>())
        .and_then(|n| {
            n.checked_add(
                sprites
                    .capacity()
                    .checked_mul(std::mem::size_of::<SpriteRect>())?,
            )
        })
        .and_then(|n| n.checked_add(scratch))
        .ok_or("mipmap 工作计划内存溢出")?;
    let required = (final_bytes as u64)
        .checked_add(work_bytes as u64)
        .ok_or("mipmap 临时预算溢出")?;
    if required > budget {
        return Err(format!(
            "材质 mip 与工作内存需要 {} MiB，预算剩余 {} MiB",
            required.div_ceil(1024 * 1024),
            budget / (1024 * 1024)
        ));
    }
    let mut owner = allocate(bytes(owner_width, owner_height, 1)?, 0u32)?;
    let mut materials = allocate(sprites.len(), GpuMaterial::zeroed())?;
    let mut owner_rects = allocate(sprites.len(), [0u32; 4])?;
    for (i, rect) in sprites.iter().enumerate() {
        let id = u32::try_from(i + 1).map_err(|_| "材质数量超出索引范围")?;
        let x0 = (rect.x - 1) / owner_scale;
        let y0 = (rect.y - 1) / owner_scale;
        let x1 = (rect.x + rect.width + 1).div_ceil(owner_scale);
        let y1 = (rect.y + rect.height + 1).div_ceil(owner_scale);
        for yy in y0..y1 {
            if yy % 32 == 0 {
                check()?;
            }
            for xx in x0..x1 {
                let cell = &mut owner[(yy as usize) * (owner_width as usize) + xx as usize];
                if *cell != 0 && *cell != id {
                    return Err("mipmap 材质槽重叠或 owner 边界不安全".into());
                }
                *cell = id;
            }
        }
        owner_rects[i] = [x0, y0, x1, y1];
        materials[i].base_uv = [
            (rect.x as f32 + 0.5) / base.width as f32,
            (rect.y as f32 + 0.5) / base.height as f32,
            (rect.x as f32 + rect.width as f32 - 0.5) / base.width as f32,
            (rect.y as f32 + rect.height as f32 - 0.5) / base.height as f32,
        ];
    }
    let mut rgba = allocate(bytes(width, height, 4)?, 0u8)?;
    plans.sort_unstable_by_key(|p| (p.material, p.level));
    let mut cursor = 0;
    for (i, rect) in sprites.iter().enumerate() {
        check()?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(bytes(rect.width, rect.height, 1)?)
            .map_err(|e| format!("mipmap 材质工作内存不足：{e}"))?;
        for sy in 0..rect.height {
            if sy % 32 == 0 {
                check()?;
            }
            for sx in 0..rect.width {
                pixels.push(rgba_pixel(&base, rect.x + sx, rect.y + sy)?);
            }
        }
        let targets = [coverage(&pixels, 0.5), coverage(&pixels, 0.1)];
        let (mut w, mut h) = (rect.width, rect.height);
        let start = cursor;
        while cursor < plans.len() && plans[cursor].material == i {
            cursor += 1;
        }
        for level in &plans[start..cursor] {
            let (next, nw, nh) = downsample(&pixels, w, h, check)?;
            pixels = next;
            w = nw;
            h = nh;
            if w != level.w || h != level.h {
                return Err("mipmap 排布与尺寸不一致".into());
            }
            let m = &mut materials[i];
            m.levels[level.level] = [
                (level.x as f32 + 1.5) / width as f32,
                (level.y as f32 + 1.5) / height as f32,
                (level.x as f32 + w as f32 + 0.5) / width as f32,
                (level.y as f32 + h as f32 + 0.5) / height as f32,
            ];
            m.coverage[level.level] = [
                coverage_scale(&pixels, targets[0], 0.5),
                coverage_scale(&pixels, targets[1], 0.1),
                0.0,
                0.0,
            ];
            m.count += 1;
            for dy in 0..h + 2 {
                for dx in 0..w + 2 {
                    let sx = dx.saturating_sub(1).min(w - 1);
                    let sy = dy.saturating_sub(1).min(h - 1);
                    let p = pixels[(sy as usize) * w as usize + sx as usize];
                    let at =
                        (((level.y + dy) as usize) * width as usize + (level.x + dx) as usize) * 4;
                    rgba[at..at + 4].copy_from_slice(&p);
                }
            }
        }
    }
    Ok((
        Arc::new(MipPage {
            base,
            width,
            height,
            rgba,
            owner_width,
            owner_height,
            owner_scale,
            owner,
            materials,
            owner_rects,
        }),
        final_bytes as u64,
    ))
}

/// 与着色器一致：导数以屏幕像素为单位，先转换为原页 texel 后计算 LOD。
pub(crate) fn lod_from_uv_gradients(
    base_size: [u32; 2],
    ddx: [f32; 2],
    ddy: [f32; 2],
) -> Result<f32, String> {
    if base_size.contains(&0) || ddx.iter().chain(&ddy).any(|v| !v.is_finite()) {
        return Err("材质 LOD 导数无效".into());
    }
    let a = [ddx[0] * base_size[0] as f32, ddx[1] * base_size[1] as f32];
    let b = [ddy[0] * base_size[0] as f32, ddy[1] * base_size[1] as f32];
    let rho = (a[0] * a[0] + a[1] * a[1])
        .sqrt()
        .max((b[0] * b[0] + b[1] * b[1]).sqrt());
    Ok(if rho <= 1.0 { 0.0 } else { rho.log2() })
}
/// 由屏幕像素坐标解出仿射 UV 导数，供 Canvas 与二维拾取共用。
pub(crate) fn affine_gradients(
    p: [[f64; 2]; 3],
    uv: [[f32; 2]; 3],
) -> Result<[[f32; 2]; 2], String> {
    let x1 = p[1][0] - p[0][0];
    let y1 = p[1][1] - p[0][1];
    let x2 = p[2][0] - p[0][0];
    let y2 = p[2][1] - p[0][1];
    let det = x1 * y2 - x2 * y1;
    if !det.is_finite() || det == 0.0 {
        return Err("材质导数的屏幕三角形退化".into());
    }
    let dx = std::array::from_fn(|i| {
        ((f64::from(uv[1][i] - uv[0][i]) * y2 - f64::from(uv[2][i] - uv[0][i]) * y1) / det) as f32
    });
    let dy = std::array::from_fn(|i| {
        ((f64::from(uv[2][i] - uv[0][i]) * x1 - f64::from(uv[1][i] - uv[0][i]) * x2) / det) as f32
    });
    Ok([dx, dy])
}
/// 与 GPU 相同的 texel 边界规则，仅消除插值舍入带来的数个 ULP 差异。
fn stable_texel(value: f32) -> f32 {
    let nearest = value.round();
    if (value - nearest).abs() <= value.abs().max(1.0) * f32::EPSILON * 4.0 {
        nearest
    } else {
        value
    }
}
fn sample_pixels(
    pixels: &[u8],
    w: u32,
    h: u32,
    uv: [f32; 2],
    linear: bool,
) -> Result<[f32; 4], String> {
    if w == 0 || h == 0 || uv.iter().any(|v| !v.is_finite()) || pixels.len() != bytes(w, h, 4)? {
        return Err("mipmap 采样源无效".into());
    }
    let get = |x: i64, y: i64| {
        let x = x.clamp(0, i64::from(w) - 1) as usize;
        let y = y.clamp(0, i64::from(h) - 1) as usize;
        let at = (y * w as usize + x) * 4;
        std::array::from_fn::<_, 4, _>(|c| pixels[at + c] as f32 / 255.0)
    };
    if !linear {
        return Ok(get(
            stable_texel(uv[0].clamp(0.0, 1.0) * w as f32).floor() as i64,
            stable_texel(uv[1].clamp(0.0, 1.0) * h as f32).floor() as i64,
        ));
    }
    let x = stable_texel(uv[0].clamp(0.0, 1.0) * w as f32) - 0.5;
    let y = stable_texel(uv[1].clamp(0.0, 1.0) * h as f32) - 0.5;
    let ix = x.floor() as i64;
    let iy = y.floor() as i64;
    let fx = x - x.floor();
    let fy = y - y.floor();
    let a = get(ix, iy);
    let b = get(ix + 1, iy);
    let c = get(ix, iy + 1);
    let d = get(ix + 1, iy + 1);
    Ok(std::array::from_fn(|i| {
        a[i] * (1.0 - fx) * (1.0 - fy)
            + b[i] * fx * (1.0 - fy)
            + c[i] * (1.0 - fx) * fy
            + d[i] * fx * fy
    }))
}
/// 在材质自己的矩形内执行 GL 的 normalized nearest / linear 与 repeat / clamp。
/// 首尾中心 UV 是图集编码，不是 SimpleTexture 归一化坐标。
fn sample_native(
    pixels: &[u8],
    w: u32,
    h: u32,
    rect: [f32; 4],
    local: [f32; 2],
    sampler: crate::textures::texture_sampler::Sampler,
) -> Result<[f32; 4], String> {
    let dimensions = [w, h];
    let start: [i64; 2] =
        std::array::from_fn(|i| (rect[i] * dimensions[i] as f32 - 0.5).round() as i64);
    let size: [i64; 2] = std::array::from_fn(|i| {
        ((rect[i + 2] - rect[i]) * dimensions[i] as f32).round() as i64 + 1
    });
    if pixels.len() != bytes(w, h, 4)?
        || (0..2)
            .any(|i| start[i] < 0 || size[i] <= 0 || start[i] + size[i] > i64::from(dimensions[i]))
    {
        return Err("整图采样矩形超出实际图片".into());
    }
    let get = |x: i64, y: i64| {
        let address = |value: i64, axis: usize| {
            start[axis]
                + if sampler.clamp {
                    value.clamp(0, size[axis] - 1)
                } else {
                    value.rem_euclid(size[axis])
                }
        };
        let at = (address(y, 1) as usize * w as usize + address(x, 0) as usize) * 4;
        std::array::from_fn::<_, 4, _>(|c| pixels[at + c] as f32 / 255.)
    };
    let p: [f32; 2] = std::array::from_fn(|i| {
        stable_texel(local[i] * size[i] as f32) - if sampler.blur { 0.5 } else { 0. }
    });
    let x = p[0].floor() as i64;
    let y = p[1].floor() as i64;
    if !sampler.blur {
        return Ok(get(x, y));
    }
    let a = get(x, y);
    let b = get(x + 1, y);
    let c = get(x, y + 1);
    let d = get(x + 1, y + 1);
    let fx = p[0] - p[0].floor();
    let fy = p[1] - p[1].floor();
    Ok(std::array::from_fn(|i| {
        a[i] * (1. - fx) * (1. - fy)
            + b[i] * fx * (1. - fy)
            + c[i] * (1. - fx) * fy
            + d[i] * fx * fy
    }))
}
fn scale_for_mode(coverage: [f32; 4], mode: u32) -> f32 {
    if mode & 3 != 1 {
        return 1.0;
    }
    if mode & 32 != 0 {
        return coverage[1];
    }
    if mode & 16 != 0 {
        let cutoff = (mode >> 8) & 255;
        return if cutoff == 127 || cutoff == 128 {
            coverage[0]
        } else if cutoff == 25 || cutoff == 26 {
            coverage[1]
        } else {
            1.0
        };
    }
    coverage[0]
}
impl MipPage {
    /// 整图坐标由首尾中心编码还原；LOD 的 footprint 必须按实际全部 texel 计算。
    pub(crate) fn lod_at(
        &self,
        uv: [f32; 2],
        mode: u32,
        mut gradients: [[f32; 2]; 2],
    ) -> Result<f32, String> {
        if let Some(owner) = self.owner_at(uv)? {
            let material = self.materials.get(owner).ok_or("LOD 材质归属无效")?;
            if mode & crate::material_mode::SIMPLE_TEXTURE != 0
                || material.padding[0] & crate::textures::texture_sampler::Sampler::FORCE != 0
            {
                let dimensions = [self.base.width, self.base.height];
                for axis in 0..2 {
                    let span = (material.base_uv[axis + 2] - material.base_uv[axis]).abs();
                    let texels = span * dimensions[axis] as f32;
                    let correction = if span > 0.00000001 {
                        (texels.round() + 1.) / texels
                    } else {
                        0.
                    };
                    for gradient in &mut gradients {
                        gradient[axis] *= correction;
                    }
                }
            }
        }
        lod_from_uv_gradients(
            [self.base.width, self.base.height],
            gradients[0],
            gradients[1],
        )
    }
    /// 真实 owner 为 1 起算，返回材质表的零起算索引。
    pub(crate) fn owner_at(&self, uv: [f32; 2]) -> Result<Option<usize>, String> {
        if uv.iter().any(|v| !v.is_finite()) {
            return Err("材质 UV 不是有限值".into());
        }
        let x = (uv[0].clamp(0., 0.999_999) * self.base.width as f32) as u32 / self.owner_scale;
        let y = (uv[1].clamp(0., 0.999_999) * self.base.height as f32) as u32 / self.owner_scale;
        let id = *self
            .owner
            .get(y as usize * self.owner_width as usize + x as usize)
            .ok_or("材质 owner 采样越界")?;
        Ok(id.checked_sub(1).map(|id| id as usize))
    }
    /// 四角 UV 的凸包始终在同一 owner 中时，片元无需再读取归属纹理。
    /// 额外保留一源像素余量，覆盖插值舍入及 owner 降采样边缘。
    pub(crate) fn owner_for_bounds(&self, uv: [f32; 4]) -> Option<u32> {
        if uv
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || uv[0] > uv[2]
            || uv[1] > uv[3]
        {
            return None;
        }
        let dimensions = [self.base.width, self.base.height];
        let lower: [u32; 2] = std::array::from_fn(|axis| {
            (uv[axis] * dimensions[axis] as f32 - 1.0).clamp(0.0, dimensions[axis] as f32 - 1.0)
                as u32
                / self.owner_scale
        });
        let upper: [u32; 2] = std::array::from_fn(|axis| {
            (uv[axis + 2] * dimensions[axis] as f32 + 1.0).clamp(0.0, dimensions[axis] as f32 - 1.0)
                as u32
                / self.owner_scale
        });
        let center = [(lower[0] + upper[0]) / 2, (lower[1] + upper[1]) / 2];
        let id = *self
            .owner
            .get(center[1] as usize * self.owner_width as usize + center[0] as usize)?;
        let index = id.checked_sub(1)? as usize;
        let rect = self.owner_rects.get(index)?;
        (index < self.materials.len()
            && lower[0] >= rect[0]
            && lower[1] >= rect[1]
            && upper[0] < rect[2]
            && upper[1] < rect[3])
            .then_some(id)
    }

    /// 纹理过滤、两级插值和 cutout 校正与 GPU 合同一致；LOD 由调用方从当前屏幕导数提供。
    pub(crate) fn sample_lod(
        &self,
        uv: [f32; 2],
        lod: f32,
        linear: bool,
        mode: u32,
    ) -> Result<[f32; 4], String> {
        if uv.iter().any(|v| !v.is_finite()) || !lod.is_finite() {
            return Err("材质 UV 或 LOD 不是有限值".into());
        }
        let px = (uv[0].clamp(0.0, 1.0) * self.base.width as f32)
            .floor()
            .min((self.base.width - 1) as f32) as u32;
        let py = (uv[1].clamp(0.0, 1.0) * self.base.height as f32)
            .floor()
            .min((self.base.height - 1) as f32) as u32;
        let index = (py / self.owner_scale) as usize * self.owner_width as usize
            + (px / self.owner_scale) as usize;
        let id = *self.owner.get(index).ok_or("材质 owner 采样越界")?;
        if id == 0 {
            if mode & crate::material_mode::SIMPLE_TEXTURE != 0 {
                return Err("整图采样缺少材质归属".into());
            }
            return sample_pixels(
                &self.base.rgba,
                self.base.width,
                self.base.height,
                uv,
                linear,
            );
        }
        let material = self
            .materials
            .get((id - 1) as usize)
            .ok_or("材质 owner 索引无效")?;
        let native = mode & crate::material_mode::SIMPLE_TEXTURE != 0
            || material.padding[0] & crate::textures::texture_sampler::Sampler::FORCE != 0;
        if mode & crate::material_mode::NO_MIPMAP != 0 && !native {
            return sample_pixels(
                &self.base.rgba,
                self.base.width,
                self.base.height,
                uv,
                linear,
            );
        }
        let base = material.base_uv;
        let local = std::array::from_fn::<_, 2, _>(|axis| {
            let span = base[axis + 2] - base[axis];
            if span.abs() <= f32::EPSILON {
                0.5
            } else {
                let coordinate = (uv[axis] - base[axis]) / span;
                if native {
                    coordinate
                } else {
                    coordinate.clamp(0.0, 1.0)
                }
            }
        });
        let at = |level: u32| -> Result<[f32; 4], String> {
            if level == 0 {
                if native {
                    return sample_native(
                        &self.base.rgba,
                        self.base.width,
                        self.base.height,
                        base,
                        local,
                        crate::textures::texture_sampler::Sampler::from_flags(material.padding[0]),
                    );
                }
                return sample_pixels(
                    &self.base.rgba,
                    self.base.width,
                    self.base.height,
                    uv,
                    linear,
                );
            }
            let i = (level - 1) as usize;
            let rect = *material.levels.get(i).ok_or("材质 mip 级数越界")?;
            let p = [
                rect[0] + (rect[2] - rect[0]) * local[0],
                rect[1] + (rect[3] - rect[1]) * local[1],
            ];
            let mut color = if native {
                sample_native(
                    &self.rgba,
                    self.width,
                    self.height,
                    rect,
                    local,
                    crate::textures::texture_sampler::Sampler::from_flags(material.padding[0]),
                )?
            } else {
                sample_pixels(&self.rgba, self.width, self.height, p, linear)?
            };
            color[3] = (color[3] * scale_for_mode(material.coverage[i], mode)).min(1.0);
            Ok(color)
        };
        let lod = if mode & crate::material_mode::NO_MIPMAP != 0 {
            0.
        } else {
            lod.clamp(0.0, material.count as f32)
        };
        let low = lod.floor() as u32;
        let high = (low + 1).min(material.count);
        let a = at(low)?;
        let b = at(high)?;
        let t = lod - low as f32;
        Ok(std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t))
    }
}
