//! Canvas 使用与 GPU 相同的等轴三角形和材质，逐像素采样，不按面数降级。
use crate::{
    gpu_mesh::{Quality, Vertex},
    textures::TextureAtlas,
};
use iced::{widget::image::Handle, Size};
use std::sync::{Arc, Mutex};

pub(crate) type CachedImage = (
    Size,
    Quality,
    crate::lighting::Key,
    Result<Handle, String>,
    Arc<crate::projected_entities::Rendered>,
);
#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) image: Option<CachedImage>,
    pub(crate) pending: Option<(Size, crate::lighting::Key)>,
}
pub(crate) type Cache = Arc<Mutex<State>>;
/// 图片与辅助线必须置于独立 iced 图层：同一 Frame 会先画全部网格，最后画图片。
pub(crate) struct ImageLayer(pub crate::isometric::IsometricSurface);
impl iced::widget::canvas::Program<crate::Message> for ImageLayer {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &iced::Theme,
        bounds: iced::Rectangle,
        _: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Text};
        let mut frame = Frame::new(renderer, bounds.size());
        let cached = self
            .0
            .canvas_cache
            .lock()
            .expect("Canvas 栅格与呈现状态锁被毒化")
            .image
            .as_ref()
            .filter(|(size, quality, light, _, _)| {
                *size == bounds.size()
                    && *quality == self.0.gpu_quality
                    && *light == self.0.lighting.key()
            })
            .map(|(_, _, _, image, rendered)| (image.clone(), Arc::clone(rendered)));
        match cached {
            Some((Ok(image), rendered)) => {
                *self
                    .0
                    .presented
                    .lock()
                    .expect("Canvas 栅格与呈现状态锁被毒化") = Some(rendered);
                frame.draw_image(
                    iced::Rectangle::with_size(bounds.size()),
                    iced::advanced::image::Image::new(image)
                        .filter_method(iced::advanced::image::FilterMethod::Nearest),
                )
            }
            result => {
                *self
                    .0
                    .presented
                    .lock()
                    .expect("Canvas 栅格与呈现状态锁被毒化") = None;
                frame.fill_text(Text {
                    content: match &result {
                        Some((Err(error), _)) => format!("Canvas 渲染失败：{error}"),
                        _ => "正在绘制材质…".into(),
                    },
                    color: if result.is_some() {
                        iced::Color::from_rgb(1., 0.35, 0.35)
                    } else {
                        iced::Color::from_rgb(0.8, 0.85, 0.9)
                    },
                    ..Default::default()
                })
            }
        }
        vec![frame.into_geometry()]
    }
}
impl crate::isometric::IsometricSurface {
    pub(crate) fn canvas_image(&self, size: Size) -> Option<Result<Handle, String>> {
        self.canvas_cache
            .lock()
            .expect("Canvas 栅格与呈现状态锁被毒化")
            .image
            .as_ref()
            .filter(|(cached, quality, light, result, rendered)| {
                *cached == size
                    && *quality == self.gpu_quality
                    && *light == self.lighting.key()
                    && (result.is_err()
                        || rendered
                            .atlas
                            .as_ref()
                            .is_none_or(|atlas| atlas.animation_is_current()))
            })
            .map(|(_, _, _, result, _)| result.clone())
    }
    /// 复用投影网格；取消或过期请求不得发布部分图片。
    pub(crate) fn canvas_image_cancellable(
        &self,
        size: Size,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Result<Handle, String>> {
        if cancelled() {
            return None;
        }
        if let Some(image) = self.canvas_image(size) {
            return Some(image);
        }
        let vertices = self.gpu_vertices_cancellable(size, cancelled)?;
        let rendered = self
            .rendered_cache
            .lock()
            .expect("Canvas 栅格与呈现状态锁被毒化")
            .as_ref()
            .filter(|r| r.viewport.logical == size)
            .cloned()?;
        let mut snapshot = (*rendered).clone();
        snapshot.atlas = match rendered
            .atlas
            .as_ref()
            .map(|atlas| atlas.snapshot())
            .transpose()
        {
            Ok(atlas) => atlas.map(Arc::new),
            Err(error) => {
                log::error!("[canvas] 动画快照失败：{error}");
                return Some(Err(error));
            }
        };
        let rendered = Arc::new(snapshot);
        let result = match render_lit(
            &vertices,
            rendered.atlas.as_deref(),
            size,
            self.gpu_quality,
            &self.lighting,
            cancelled,
        ) {
            Ok(Some(raster)) => Ok(raster.into_image()),
            Ok(None) => return None,
            Err(error) => {
                log::error!("[canvas] 光栅化失败：{error}");
                Err(error)
            }
        };
        if cancelled() {
            return None;
        }
        let mut cache = self
            .canvas_cache
            .lock()
            .expect("Canvas 栅格与呈现状态锁被毒化");
        if cancelled() {
            return None;
        }
        cache.image = Some((
            size,
            self.gpu_quality,
            self.lighting.key(),
            result.clone(),
            rendered,
        ));
        Some(result)
    }
}
/// RGBA8 中间画面遵循生产 GPU 通道：透明部分为预乘 Alpha。
pub(crate) struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl Raster {
    /// iced 图片接收直通 Alpha；只在发布已完成画面时转换一次。
    pub fn into_image(mut self) -> Handle {
        for p in self.rgba.chunks_exact_mut(4) {
            if p[3] > 0 && p[3] < 255 {
                for i in 0..3 {
                    p[i] = ((p[i] as u32 * 255 + p[3] as u32 / 2) / p[3] as u32).min(255) as u8;
                }
            }
        }
        Handle::from_rgba(self.width, self.height, self.rgba)
    }
}

/// 与资源图集 GPU 的 clamp-to-edge 和 nearest / linear 采样相同。
pub(crate) fn sample(
    atlas: Option<&TextureAtlas>,
    uv: [f32; 2],
    page: u32,
    quality: Quality,
) -> Result<[f32; 4], String> {
    sample_gradients(atlas, uv, page, quality, 0, [[0.; 2]; 2])
}

pub(crate) fn sample_gradients(
    atlas: Option<&TextureAtlas>,
    uv: [f32; 2],
    page: u32,
    quality: Quality,
    mode: u32,
    gradients: [[f32; 2]; 2],
) -> Result<[f32; 4], String> {
    let Some(atlas) = atlas else {
        return Ok([1.0; 4]);
    };
    let lod = atlas.mip_page(page)?.lod_at(uv, mode, gradients)?;
    atlas.sample_lod(uv, page, lod, quality == Quality::Detailed, mode)
}
/// 输入为屏幕像素；实体与受限视图的透明拾取复用同一 LOD。
pub(crate) fn sample_projected(
    atlas: Option<&TextureAtlas>,
    uv: [f32; 2],
    v: [Vertex; 3],
    quality: Quality,
    viewport: crate::render_viewport::RenderViewport,
) -> Result<[f32; 4], String> {
    let gradients = crate::material_mips::affine_gradients(
        v.map(|v| {
            [
                f64::from(v.xy[0]) * f64::from(viewport.raster.width)
                    / f64::from(viewport.logical.width),
                f64::from(v.xy[1]) * f64::from(viewport.raster.height)
                    / f64::from(viewport.logical.height),
            ]
        }),
        v.map(|v| v.uv),
    )?;
    sample_gradients(atlas, uv, v[0].atlas_page, quality, v[0].mode, gradients)
}

fn allocated<T: Clone>(len: usize, value: T) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|e| format!("Canvas 工作内存分配失败：{e}"))?;
    out.resize(len, value);
    Ok(out)
}

pub(crate) fn render(
    vertices: &[Vertex],
    atlas: Option<&TextureAtlas>,
    size: Size,
    quality: Quality,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Raster>, String> {
    render_item(vertices, atlas, size, quality, &[], cancelled)
}
/// 世界预览使用已核实的光照表；只重绘像素，不修改共享源顶点或拾取索引。
pub(crate) fn render_lit(
    vertices: &[Vertex],
    atlas: Option<&TextureAtlas>,
    size: Size,
    quality: Quality,
    lighting: &crate::lighting::Table,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Raster>, String> {
    render_with_lighting(
        vertices,
        atlas,
        size,
        quality,
        &[],
        Some(lighting),
        cancelled,
    )
}
/// 附魔通道复用同一深度缓冲、三角形覆盖规则和实际图集 UV。
pub(crate) fn render_item(
    vertices: &[Vertex],
    atlas: Option<&TextureAtlas>,
    size: Size,
    quality: Quality,
    glint: &[crate::item_glint::Pass<'_>],
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Raster>, String> {
    render_with_lighting(vertices, atlas, size, quality, glint, None, cancelled)
}
fn render_with_lighting(
    vertices: &[Vertex],
    atlas: Option<&TextureAtlas>,
    size: Size,
    quality: Quality,
    glint: &[crate::item_glint::Pass<'_>],
    lighting: Option<&crate::lighting::Table>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Raster>, String> {
    let snapshot = atlas.map(TextureAtlas::snapshot).transpose()?;
    let atlas = snapshot.as_ref();
    if cancelled() {
        return Ok(None);
    }
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.
        || size.height <= 0.
        || size.width.ceil() as f64 > u32::MAX as f64
        || size.height.ceil() as f64 > u32::MAX as f64
    {
        return Err("Canvas 画面尺寸无效".into());
    }
    if !vertices.len().is_multiple_of(6) {
        return Err("Canvas 源面顶点没有按六顶点对齐".into());
    }
    let width = size.width.ceil() as u32;
    let height = size.height.ceil() as u32;
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or("Canvas 像素数量溢出")?;
    let bytes = pixels.checked_mul(4).ok_or("Canvas 画面大小溢出")?;
    let mut rgba = allocated(bytes, 0u8)?;
    if vertices
        .iter()
        .any(|vertex| vertex.mode & crate::material_mode::ADDITIVE != 0)
    {
        let lighting = lighting.ok_or("加色场景需要明确的背景日光参数")?;
        let [top, bottom] = crate::sky_background::colors(lighting.settings.daylight);
        for (y, row) in rgba.chunks_exact_mut(width as usize * 4).enumerate() {
            if y % 32 == 0 && cancelled() {
                return Ok(None);
            }
            let t = (y as f32 + 0.5) / height as f32;
            let rgb: [u8; 3] = std::array::from_fn(|i| unorm(top[i] * (1. - t) + bottom[i] * t));
            for pixel in row.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
    }
    let mut depth = allocated(pixels, 1f32)?;
    for (index, face) in vertices.chunks_exact(6).enumerate() {
        if index % 256 == 0 && cancelled() {
            return Ok(None);
        }
        for v in face {
            if v.world_mode != 0
                || !v
                    .xy
                    .iter()
                    .chain(&v.rgba)
                    .chain(&v.uv)
                    .all(|n| n.is_finite())
                || !v.depth.is_finite()
                || !(0. ..=1.).contains(&v.depth)
                || v.mode != face[0].mode
                || v.atlas_page != face[0].atlas_page
            {
                return Err(format!("Canvas 源面 {index} 含无效投影顶点或材质"));
            }
        }
        if let Some(atlas) = atlas {
            if atlas.page(face[0].atlas_page).is_none() {
                return Err(format!("Canvas 源面 {index} 图集页不存在"));
            }
        }
    }
    let mut target = Target {
        width,
        height,
        rgba: &mut rgba,
        depth: &mut depth,
        atlas,
        quality,
        lighting,
    };
    // 原版绊线同时混合与写 Z，在普通不透明之后、普通透明之前执行。
    use crate::material_mode::Pass;
    for stage in [Pass::Opaque, Pass::Tripwire, Pass::Translucent] {
        for (index, face) in vertices.chunks_exact(6).enumerate() {
            if index % 256 == 0 && cancelled() {
                return Ok(None);
            }
            if crate::material_mode::pass(face[0].mode) != stage {
                continue;
            }
            for triangle in face.chunks_exact(3) {
                if !target.triangle(
                    [triangle[0], triangle[1], triangle[2]],
                    stage,
                    None,
                    cancelled,
                )? {
                    return Ok(None);
                }
            }
        }
    }
    for overlay in glint {
        for (index, face) in vertices.chunks_exact(6).enumerate() {
            if index % 256 == 0 && cancelled() {
                return Ok(None);
            }
            for triangle in face.chunks_exact(3) {
                if !target.triangle(
                    [triangle[0], triangle[1], triangle[2]],
                    Pass::Opaque,
                    Some(overlay),
                    cancelled,
                )? {
                    return Ok(None);
                }
            }
        }
    }
    if cancelled() {
        return Ok(None);
    }
    Ok(Some(Raster {
        width,
        height,
        rgba,
    }))
}

struct Target<'a> {
    width: u32,
    height: u32,
    rgba: &'a mut [u8],
    depth: &'a mut [f32],
    atlas: Option<&'a TextureAtlas>,
    quality: Quality,
    lighting: Option<&'a crate::lighting::Table>,
}
fn edge(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn top_left(a: [f64; 2], b: [f64; 2]) -> bool {
    b[1] < a[1] || b[1] == a[1] && b[0] > a[0]
}
impl Target<'_> {
    fn triangle(
        &mut self,
        mut v: [Vertex; 3],
        stage: crate::material_mode::Pass,
        overlay: Option<&crate::item_glint::Pass<'_>>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<bool, String> {
        let mut p = v.map(|v| {
            [
                (v.xy[0] as f64 + 1.) * self.width as f64 * 0.5,
                (1. - v.xy[1] as f64) * self.height as f64 * 0.5,
            ]
        });
        let mut area = edge(p[0], p[1], p[2]);
        if area == 0. {
            return Ok(true);
        }
        // WebGPU 默认 CCW 在屏幕向下的 Y 轴中为负面积。
        if v[0].mode & crate::material_mode::SINGLE_SIDED != 0 && area > 0. {
            return Ok(true);
        }
        if area < 0. {
            p.swap(1, 2);
            v.swap(1, 2);
            area = -area;
        }
        let gradients = crate::material_mips::affine_gradients(p, v.map(|v| v.uv))?;
        let minimum = |axis: usize| p.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
        let maximum = |axis: usize| p.iter().map(|p| p[axis]).fold(f64::NEG_INFINITY, f64::max);
        let x0 = (minimum(0) - 0.5).ceil().max(0.).min(self.width as f64) as u32;
        let x1 = (maximum(0) - 0.5)
            .floor()
            .max(-1.)
            .min(self.width as f64 - 1.) as i64;
        let y0 = (minimum(1) - 0.5).ceil().max(0.).min(self.height as f64) as u32;
        let y1 = (maximum(1) - 0.5)
            .floor()
            .max(-1.)
            .min(self.height as f64 - 1.) as i64;
        if x1 < x0 as i64 || y1 < y0 as i64 {
            return Ok(true);
        }
        if overlay.is_none() {
            for vertex in &mut v {
                let mut color = crate::material_mode::vertex_color(vertex.mode, vertex.rgba);
                if let Some(table) = self.lighting {
                    let light = table.rgb(vertex.light)?;
                    for axis in 0..3 {
                        color[axis] *= light[axis];
                    }
                }
                vertex.rgba = color;
            }
        }
        let pairs = [(1, 2), (2, 0), (0, 1)];
        let inclusive = pairs.map(|(a, b)| top_left(p[a], p[b]));
        for y in y0..=y1 as u32 {
            if y % 32 == 0 && cancelled() {
                return Ok(false);
            }
            for x in x0..=x1 as u32 {
                let point = [x as f64 + 0.5, y as f64 + 0.5];
                let e = pairs.map(|(a, b)| edge(p[a], p[b], point));
                if e.iter()
                    .zip(inclusive)
                    .any(|(&e, include)| e < 0. || e == 0. && !include)
                {
                    continue;
                }
                let weights = e.map(|e| (e / area) as f32);
                // 用基点和差值插值，使常量属性保持原值。直接累加三份权重会让
                // Alpha=0.5 落到 cutoff 以下，或让恒定远深度变成大于 1 的值。
                let interpolate =
                    |a: [f32; 3]| a[0] + (a[1] - a[0]) * weights[1] + (a[2] - a[0]) * weights[2];
                let z = interpolate(v.map(|v| v.depth)).clamp(0., 1.);
                let index = y as usize * self.width as usize + x as usize;
                if z > self.depth[index] {
                    continue;
                }
                if v[0].mode & crate::material_mode::DEPTH_EQUAL != 0 && z != self.depth[index] {
                    continue;
                }
                if let Some(overlay) = overlay {
                    // 原版 EQUAL 深度和 COLOR_WRITE：后方、镂空和透明背景不会产生光效 Alpha。
                    if z != self.depth[index] {
                        continue;
                    }
                    let uv = std::array::from_fn(|axis| interpolate(v.map(|v| v.uv[axis])));
                    let sample = overlay.sample(uv);
                    if sample[3] < 0.1 {
                        continue;
                    }
                    let out = &mut self.rgba[index * 4..index * 4 + 4];
                    for i in 0..3 {
                        out[i] = unorm(sample[i] * sample[i] + out[i] as f32 / 255.);
                    }
                    continue;
                }
                let uv = std::array::from_fn(|axis| interpolate(v.map(|v| v.uv[axis])));
                let color: [f32; 4] =
                    std::array::from_fn(|axis| interpolate(v.map(|v| v.rgba[axis])));
                let texel = sample_gradients(
                    self.atlas,
                    uv,
                    v[0].atlas_page,
                    self.quality,
                    v[0].mode,
                    gradients,
                )?;
                let Some(shaded) = crate::material_mode::shade_texel(v[0].mode, texel, color)
                else {
                    continue;
                };
                if v[0].mode & crate::material_mode::ADDITIVE != 0 {
                    let out = &mut self.rgba[index * 4..index * 4 + 4];
                    for i in 0..3 {
                        out[i] = unorm(shaded[i] + out[i] as f32 / 255.);
                    }
                    if stage != crate::material_mode::Pass::Translucent {
                        self.depth[index] = z;
                    }
                    continue;
                }
                let alpha = shaded[3];
                let out = &mut self.rgba[index * 4..index * 4 + 4];
                if stage != crate::material_mode::Pass::Opaque {
                    for i in 0..3 {
                        out[i] = unorm(shaded[i] * alpha + out[i] as f32 / 255. * (1. - alpha));
                    }
                    out[3] = unorm(alpha + out[3] as f32 / 255. * (1. - alpha));
                } else {
                    for i in 0..3 {
                        out[i] = unorm(shaded[i]);
                    }
                    out[3] = unorm(alpha);
                }
                if stage != crate::material_mode::Pass::Translucent {
                    self.depth[index] = z;
                }
            }
        }
        Ok(true)
    }
}
fn unorm(value: f32) -> u8 {
    (value.clamp(0., 1.) * 255.).round() as u8
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
#[path = "canvas_raster_tests.rs"]
pub(crate) mod tests;
