//! 原版文字按资源字形四角发射，保持浮点排版、源像素分辨率与逐行绘制顺序。
use crate::{
    font_glyphs::{Fonts, Glyph},
    textures::{RuntimeTexture, TextureBank},
};
use mcedit2_world::sign::{Side, Span, Style};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct Part {
    pub texture: Arc<RuntimeTexture>,
    pub corners: [[f32; 2]; 4],
    pub outline: bool,
    pub depth: f32,
    pub uv: [f32; 4],
}
pub(crate) struct Prepared {
    pub parts: Vec<Part>,
    pub widths: [f32; 4],
}
struct Placed {
    glyph: Arc<Glyph>,
    style: Style,
    ch: char,
    x: f32,
    y: f32,
    color: u32,
}
fn image(rgba: Vec<u8>, width: u32, height: u32) -> Arc<RuntimeTexture> {
    let mut hash = Sha256::new();
    hash.update(width.to_le_bytes());
    hash.update(height.to_le_bytes());
    hash.update(&rgba);
    Arc::new(RuntimeTexture {
        sampler: None,
        id: format!("mcedit2:runtime/sign/{:x}", hash.finalize()),
        width,
        height,
        rgba,
    })
}
fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32, italic: bool) -> [[f32; 2]; 4] {
    let shear = |y| if italic { 1. - 0.25 * y } else { 0. };
    [
        [x0 + shear(y0), y0],
        [x0 + shear(y1), y1],
        [x1 + shear(y1), y1],
        [x1 + shear(y0), y0],
    ]
}
fn push(parts: &mut Vec<Part>, part: Part, budget: u64) -> Result<(), String> {
    let size = parts
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_mul(std::mem::size_of::<Part>()))
        .ok_or("文字几何工作集溢出")?;
    if size as u64 > budget {
        return Err("文字几何超过剩余资源预算".into());
    }
    parts.try_reserve(1).map_err(|e| e.to_string())?;
    parts.push(part);
    Ok(())
}
/// 每行先绘制八向描边，再绘制正文；素材保存原始像素，不截断伸出字面框的升降部。
pub(crate) fn prepare(
    side: &Side,
    bank: &TextureBank,
    fonts: &mut Fonts<'_>,
    language: crate::language::Language,
    max_width: u32,
    line_height: u32,
    text_color: u32,
    outline_color: Option<u32>,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Prepared>, String> {
    let budget = bank
        .atlas()
        .ok_or("文字准备缺少已完成图集")?
        .remaining_bytes();
    let mut parts = Vec::new();
    let mut widths = [0.; 4];
    let mut materials = BTreeMap::<(String, char, u32), Arc<RuntimeTexture>>::new();
    let mut solids = BTreeMap::<u32, Arc<RuntimeTexture>>::new();
    let mut material_bytes = 0usize;
    for (row, component) in side.messages.iter().enumerate() {
        if stop() {
            return Err("文字排版已取消".into());
        }
        let spans = mcedit2_world::sign::spans(component, &|key| {
            bank.translation(key, language).map(str::to_owned)
        })?;
        let mut line = Vec::new();
        let mut width = 0f32;
        let mut cursor = 0f32;
        let mut last_space = None;
        let mut has_advance = false;
        'line: for Span { text, style } in spans {
            if style.obfuscated {
                return Err("混淆文字需要动态字形帧，不能以源文字冒充显示".into());
            }
            for ch in text.chars() {
                if stop() {
                    return Err("文字排版已取消".into());
                }
                if ch == '\n' {
                    break 'line;
                }
                let glyph = fonts.glyph_or_missing(&style.font, ch, stop)?;
                let bold = if style.bold {
                    if glyph.legacy.is_some() {
                        1.
                    } else {
                        glyph.bold_offset
                    }
                } else {
                    0.
                };
                let measured = glyph
                    .legacy
                    .map_or(glyph.advance, |metrics| metrics.measure)
                    + bold;
                let advance = glyph.advance + bold;
                // 原版在检查溢出前记录当前空格；零宽字符不算已读的可测量文字。
                if ch == ' ' {
                    last_space = Some((line.len(), width));
                }
                if width + measured > max_width as f32 && has_advance {
                    if let Some((count, before)) = last_space {
                        line.truncate(count);
                        width = before;
                    }
                    break 'line;
                }
                has_advance |= measured != 0.;
                line.push(Placed {
                    glyph,
                    style: style.clone(),
                    ch,
                    x: cursor,
                    y: (row as u32 * line_height) as f32 - 2. * line_height as f32,
                    color: style.color.unwrap_or(text_color),
                });
                width += measured;
                cursor += advance;
            }
        }
        widths[row] = width.ceil();
        let center = -((width.ceil() as i32) / 2) as f32;
        for glyph in &mut line {
            glyph.x += center;
        }
        let offsets = outline_color
            .into_iter()
            .flat_map(|color| {
                [-1., 0., 1.].into_iter().flat_map(move |dx| {
                    [-1., 0., 1.].into_iter().filter_map(move |dy| {
                        ((dx, dy) != (0., 0.)).then_some((dx, dy, Some(color)))
                    })
                })
            })
            .chain(std::iter::once((0., 0., None)));
        for (dx, dy, outline) in offsets {
            for item in &line {
                if stop() {
                    return Err("文字几何准备已取消".into());
                }
                let color = outline.unwrap_or(item.color);
                let (dx, dy) = if outline.is_some() {
                    (dx * item.glyph.shadow_offset, dy * item.glyph.shadow_offset)
                } else {
                    (dx, dy)
                };
                if item.glyph.width != 0 && item.glyph.height != 0 {
                    let key = (item.style.font.clone(), item.ch, color);
                    let texture = if let Some(texture) = materials.get(&key) {
                        Arc::clone(texture)
                    } else {
                        let bytes = item.glyph.rgba.len();
                        material_bytes = material_bytes
                            .checked_add(bytes)
                            .ok_or("字形素材工作集溢出")?;
                        if material_bytes as u64 > budget {
                            return Err("字形素材超过剩余资源预算".into());
                        }
                        let mut rgba = Vec::new();
                        rgba.try_reserve_exact(bytes).map_err(|e| e.to_string())?;
                        for pixel in item.glyph.rgba.chunks_exact(4) {
                            let channel = |i: usize, shift: u32| {
                                ((u32::from(pixel[i]) * ((color >> shift) & 255u32) + 127) / 255)
                                    as u8
                            };
                            rgba.extend_from_slice(&[
                                channel(0, 16),
                                channel(1, 8),
                                channel(2, 0),
                                pixel[3],
                            ]);
                        }
                        let texture = image(rgba, item.glyph.width, item.glyph.height);
                        materials.insert(key, Arc::clone(&texture));
                        texture
                    };
                    for bold in [false, true] {
                        if bold && !item.style.bold {
                            continue;
                        }
                        let extra = if bold { item.glyph.bold_offset } else { 0. };
                        let left = item.glyph.left;
                        let top = item.glyph.top;
                        let extent = item.glyph.legacy.map_or(
                            [
                                item.glyph.width as f32 * item.glyph.scale,
                                item.glyph.height as f32 * item.glyph.scale,
                            ],
                            |metrics| metrics.extent,
                        );
                        let mut corners = rectangle(
                            left,
                            top,
                            left + extent[0],
                            top + extent[1],
                            item.style.italic,
                        );
                        for p in &mut corners {
                            p[0] += item.x + dx + extra;
                            p[1] += item.y + dy;
                        }
                        push(
                            &mut parts,
                            Part {
                                texture: Arc::clone(&texture),
                                corners,
                                outline: outline.is_some(),
                                depth: 0.,
                                uv: item
                                    .glyph
                                    .legacy
                                    .map_or([0., 0., 1., 1.], |metrics| metrics.uv),
                            },
                            budget,
                        )?;
                    }
                }
            }
            if outline.is_none() {
                for item in &line {
                    let color = item.color;
                    for (enabled, y) in
                        [(item.style.strikethrough, 4.5), (item.style.underlined, 9.)]
                    {
                        if !enabled {
                            continue;
                        }
                        let texture = Arc::clone(solids.entry(color).or_insert_with(|| {
                            image(
                                vec![
                                    ((color >> 16) & 255) as u8,
                                    ((color >> 8) & 255) as u8,
                                    (color & 255) as u8,
                                    255,
                                ],
                                1,
                                1,
                            )
                        }));
                        let x0 = item.x - 1.;
                        let x1 = item.x
                            + item.glyph.advance
                            + if item.style.bold {
                                item.glyph.bold_offset
                            } else {
                                0.
                            };
                        let y0 = item.y + y - 1.;
                        push(
                            &mut parts,
                            Part {
                                texture,
                                corners: rectangle(x0, y0, x1, y0 + 1., false),
                                outline: false,
                                depth: 0.01,
                                uv: [0., 0., 1., 1.],
                            },
                            budget,
                        )?;
                    }
                }
            }
        }
    }
    if stop() {
        return Err("文字几何准备已取消".into());
    }
    Ok((!parts.is_empty() || widths.iter().any(|w| *w > 0.)).then_some(Prepared { parts, widths }))
}
