//! 字体、SimpleTexture 与 atlas 共用一次 PNG 解码，各自保留所需源与错误。
use super::Tile;
use image::{ImageFormat, RgbaImage};
type Frame = (u32, u32, u32, u32, bool);
pub(super) struct Output {
    pub layout: Option<[u8; 2]>,
    pub diagnostics: Vec<String>,
    pub font: Option<Result<Tile, String>>,
    pub simple: Option<Result<Tile, String>>,
    pub sampler: Option<Result<super::texture_sampler::Sampler, String>>,
    pub material: Option<Result<(Tile, bool), String>>,
    pub animation: Option<crate::textures::texture_animation::Sprite>,
}
pub(super) struct Plan {
    diagnostics: Vec<String>,
    dimensions: Result<[u32; 2], String>,
    font: bool,
    sampler: Option<Result<super::texture_sampler::Sampler, String>>,
    material: Option<Result<Option<Frame>, String>>,
    animation: Option<crate::textures::texture_animation::Sequence>,
}
fn dimensions(data: &[u8]) -> Result<[u32; 2], String> {
    if data.len() < 24 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("PNG 头部或长度无效".into());
    }
    let width = u32::from_be_bytes(data[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(data[20..24].try_into().unwrap());
    if width == 0
        || height == 0
        || u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4))
            .is_none_or(|n| n > isize::MAX as u64)
    {
        return Err("PNG 尺寸或地址范围无效".into());
    }
    Ok([width, height])
}
#[path = "java_animation.rs"]
mod animation;
#[cfg(test)]
pub(super) fn verify_reference() {
    animation::verify_reference();
}

impl Plan {
    pub fn new(data: &[u8], metadata: Option<&[u8]>, font: bool, material: bool) -> Self {
        let dimensions = dimensions(data);
        let mut diagnostics = Vec::new();
        let mut animation = None;
        let sampler = material.then(|| super::texture_sampler::Sampler::read(metadata));
        let material = material.then(|| match &dimensions {
            Ok([width, height]) => animation::select(*width, *height, metadata).map(|selection| {
                diagnostics = selection.diagnostics;
                animation = selection.animation;
                selection.frame
            }),
            Err(error) => Err(error.clone()),
        });
        Self {
            diagnostics,
            dimensions,
            font,
            sampler,
            material,
            animation,
        }
    }
    /// 预留输入、16 位解码 / 转换、完整源与各合同输出；数量不构成图片上限。
    pub fn working_bytes(&self, input_bytes: usize) -> Result<usize, String> {
        let pixels = if self.font || self.material.is_some() {
            self.dimensions
                .as_ref()
                .map_or(0, |[w, h]| u64::from(*w) * u64::from(*h) * 4)
        } else {
            0
        };
        let bytes = pixels
            .checked_mul(4)
            .and_then(|n| n.checked_add(input_bytes as u64))
            .and_then(|n| {
                n.checked_add(
                    self.diagnostics
                        .iter()
                        .map(|d| d.len() as u64 + std::mem::size_of::<String>() as u64)
                        .sum::<u64>(),
                )
            })
            .and_then(|n| {
                n.checked_add(self.animation.as_ref().map_or(0, |s| {
                    s.steps.len() as u64
                        * std::mem::size_of::<crate::textures::texture_animation::Step>() as u64
                }))
            })
            .and_then(|n| {
                n.checked_add(
                    self.sampler
                        .as_ref()
                        .and_then(|sampler| sampler.as_ref().err())
                        .map_or(0, |error| error.len() as u64),
                )
            })
            .ok_or("PNG 解码工作集大小溢出")?;
        usize::try_from(bytes).map_err(|_| "PNG 解码工作集超过地址范围".into())
    }
    pub fn decode(self, data: &[u8], cancelled: &dyn Fn() -> bool) -> Output {
        let mut output = Output {
            layout: None,
            diagnostics: self.diagnostics,
            font: None,
            simple: None,
            sampler: self.sampler,
            material: None,
            animation: None,
        };
        if !self.font && self.material.is_none() {
            return output;
        }
        let image: Result<RgbaImage, String> = (|| {
            if cancelled() {
                return Err("材质加载已取消".into());
            }
            self.dimensions.as_ref().map_err(Clone::clone)?;
            let image = image::load_from_memory_with_format(data, ImageFormat::Png)
                .map_err(|error| format!("PNG 无效：{error}"))?
                .into_rgba8();
            if cancelled() {
                return Err("材质加载已取消".into());
            }
            Ok(image)
        })();
        match image {
            Ok(image) => {
                output.layout = Some([data[24], data[25]]);
                if !self.font && matches!(&self.material, Some(Ok(None))) {
                    output.material = Some(Ok((
                        Tile {
                            width: image.width(),
                            height: image.height(),
                            rgba: image.into_raw(),
                        },
                        false,
                    )));
                    return output;
                }
                let whole = self
                    .material
                    .as_ref()
                    .is_some_and(|material| !matches!(material, Ok(None)));
                if let Some(material) = &self.material {
                    output.material = Some(match material {
                        Ok(frame) => tile(&image, *frame, cancelled),
                        Err(error) => Err(error.clone()),
                    });
                }
                let sprite = self
                    .animation
                    .filter(|_| output.material.as_ref().is_some_and(Result::is_ok));
                let preserve = whole || sprite.is_some();
                if self.font && preserve {
                    output.font = Some(Ok(Tile {
                        width: image.width(),
                        height: image.height(),
                        rgba: image.as_raw().clone(),
                    }));
                }
                if preserve {
                    let source = Tile {
                        width: image.width(),
                        height: image.height(),
                        rgba: image.into_raw(),
                    };
                    if let Some(sequence) = sprite {
                        output.animation =
                            Some(crate::textures::texture_animation::Sprite { source, sequence });
                    } else {
                        output.simple = Some(Ok(source));
                    }
                } else if self.font {
                    output.font = Some(Ok(Tile {
                        width: image.width(),
                        height: image.height(),
                        rgba: image.into_raw(),
                    }));
                }
            }
            Err(error) => {
                if self.font {
                    output.font = Some(Err(error.clone()));
                }
                if let Some(material) = self.material {
                    output.material = Some(Err(material.err().unwrap_or_else(|| error.clone())));
                    output.simple = Some(Err(error));
                }
            }
        }
        output
    }
}
fn tile(
    image: &RgbaImage,
    frame: Option<Frame>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Tile, bool), String> {
    let (x, y, width, height, animated) =
        frame.unwrap_or((0, 0, image.width(), image.height(), false));
    let length = usize::try_from(u64::from(width) * u64::from(height) * 4)
        .map_err(|_| "静态帧超过地址范围")?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(length)
        .map_err(|e| format!("无法分配静态 PNG 帧：{e}"))?;
    let source = image.as_raw();
    let row_bytes = width as usize * 4;
    for row in y..y + height {
        if row.is_multiple_of(32) && cancelled() {
            return Err("材质加载已取消".into());
        }
        let start = ((u64::from(row) * u64::from(image.width()) + u64::from(x)) * 4) as usize;
        rgba.extend_from_slice(
            source
                .get(start..start + row_bytes)
                .ok_or("PNG 帧范围与解码结果不匹配")?,
        );
    }
    Ok((
        Tile {
            width,
            height,
            rgba,
        },
        animated,
    ))
}
