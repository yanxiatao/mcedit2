//! 从资源包字体提供器读取真实字形；按来源共享，后台可取消，不使用系统字体替代资源字体。
use crate::textures::TextureBank;
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub(crate) struct Glyph {
    pub legacy: Option<LegacyMetrics>,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub scale: f32,
    pub top: f32,
    pub left: f32,
    pub advance: f32,
    pub bold_offset: f32,
    pub shadow_offset: f32,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct LegacyMetrics {
    pub extent: [f32; 2],
    pub uv: [f32; 4],
    pub measure: f32,
}
#[derive(Clone)]
enum Provider {
    Space(BTreeMap<char, f32>),
    Reference(String),
    Bitmap {
        file: String,
        height: f32,
        ascent: f32,
        chars: Vec<Vec<char>>,
        legacy: bool,
    },
    LegacyUnicode,
    Unihex {
        file: String,
        overrides: Vec<(u32, u32, u8, u8)>,
    },
    TrueType {
        file: String,
        size: f32,
        oversample: f32,
        shift: [f32; 2],
        skip: BTreeSet<char>,
    },
    Unsupported(String),
}

pub(crate) struct Fonts<'a> {
    unicode_112: bool,
    missing_logged: BTreeSet<String>,
    bank: &'a TextureBank,
    providers: BTreeMap<String, Arc<Vec<Provider>>>,
    hex: BTreeMap<String, Arc<BTreeMap<u32, crate::font_unihex::Glyph>>>,
    faces: BTreeMap<String, FontArc>,
    glyphs: BTreeMap<(String, char), Option<Arc<Glyph>>>,
    glyph_bytes: usize,
    working_bytes: usize,
}
fn qualified(value: &str) -> Result<String, String> {
    crate::resource_ids::qualified(value, "minecraft")
        .ok_or_else(|| format!("字体资源 ID 无效：{value}"))
}
fn text<'a>(doc: &'a Value, key: &str) -> Result<&'a str, String> {
    doc.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("字体定义缺少字符串 {key}"))
}
fn number(doc: &Value, key: &str, default: Option<f32>) -> Result<f32, String> {
    let value = doc
        .get(key)
        .map(|value| {
            value
                .as_f64()
                .map(|v| v as f32)
                .ok_or_else(|| format!("字体 {key} 不是数值"))
        })
        .transpose()?
        .or(default)
        .ok_or_else(|| format!("字体缺少 {key}"))?;
    if !value.is_finite() {
        return Err(format!("字体 {key} 不是有限数值"));
    }
    Ok(value)
}
fn character(value: &str) -> Result<char, String> {
    let mut chars = value.chars();
    let ch = chars.next().ok_or("字体字符为空")?;
    if chars.next().is_some() {
        return Err("字体字符字段含多个码点".into());
    }
    Ok(ch)
}
impl<'a> Fonts<'a> {
    pub(crate) fn new(bank: &'a TextureBank, language: crate::language::Language) -> Self {
        Self {
            bank,
            unicode_112: language == crate::language::Language::Chinese,
            missing_logged: BTreeSet::new(),
            providers: BTreeMap::new(),
            hex: BTreeMap::new(),
            faces: BTreeMap::new(),
            glyphs: BTreeMap::new(),
            glyph_bytes: 0,
            working_bytes: 0,
        }
    }
    fn budget(&self, extra: usize) -> Result<(), String> {
        let available = self
            .bank
            .atlas()
            .ok_or("字体准备缺少已完成图集")?
            .remaining_bytes();
        let requested = self
            .working_bytes
            .checked_add(self.glyph_bytes)
            .and_then(|v| v.checked_add(extra))
            .ok_or("字体工作集溢出")?;
        if requested as u64 > available {
            return Err(format!(
                "字体工作集需要 {requested} 字节，超过剩余预算 {available}"
            ));
        }
        Ok(())
    }
    fn providers(&mut self, id: &str) -> Result<Arc<Vec<Provider>>, String> {
        if let Some(providers) = self.providers.get(id) {
            return Ok(Arc::clone(providers));
        }
        if id == "minecraft:default"
            && self.bank.font_resources().legacy_112
            && self.bank.font_resources().definitions(id).is_none()
        {
            let doc: Value = serde_json::from_str(include_str!(
                "../../mcedit2-world/assets/legacy-font-1.12.2.json"
            ))
            .map_err(|e| format!("内置旧字符表损坏：{e}"))?;
            let chars = doc["chars"]
                .as_array()
                .ok_or("内置旧字符表缺少 chars")?
                .iter()
                .map(|row| {
                    row.as_str()
                        .map(|row| row.chars().collect::<Vec<_>>())
                        .ok_or("旧字符行无效")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if chars.len() != 16 || chars.iter().any(|row| row.len() != 16) {
                return Err("内置旧字符表必须为 16×16".into());
            }
            let mut providers = vec![Provider::Space(BTreeMap::from([(' ', 4.)]))];
            if !self.unicode_112 {
                providers.push(Provider::Bitmap {
                    file: "minecraft:font/ascii".into(),
                    height: 8.,
                    ascent: 7.,
                    chars,
                    legacy: true,
                });
            }
            providers.push(Provider::LegacyUnicode);
            let providers = Arc::new(providers);
            self.providers.insert(id.into(), Arc::clone(&providers));
            return Ok(providers);
        }
        let layers = self
            .bank
            .font_resources()
            .definitions(id)
            .ok_or_else(|| format!("缺少字体定义 {id}"))?;
        let mut providers = Vec::new();
        for layer in layers.iter().rev() {
            self.budget(layer.len().checked_mul(8).ok_or("字体定义解析工作集溢出")?)?;
            let doc: Value = mcedit2_world::client_json::object(layer)
                .map_err(|e| format!("字体 {id} JSON 无效：{e}"))?;
            for entry in doc["providers"]
                .as_array()
                .ok_or("字体 providers 不是数组")?
            {
                if let Some(filter) = entry.get("filter") {
                    let mut enabled = true;
                    for (key, value) in filter.as_object().ok_or("字体 filter 不是对象")? {
                        if !matches!(key.as_str(), "uniform" | "jp") {
                            return Err(format!("未知字体选项 {key}"));
                        }
                        // 当前编辑器采用原版两项选项关闭时的默认字体合同。
                        if value.as_bool().ok_or("字体 filter 不是布尔值")? {
                            enabled = false;
                        }
                    }
                    if !enabled {
                        continue;
                    }
                }
                let kind = text(entry, "type")?;
                let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
                providers.push(match kind {
                    "reference" => Provider::Reference(qualified(text(entry, "id")?)?),
                    "space" => {
                        let mut advances = BTreeMap::new();
                        for (ch, value) in
                            entry["advances"].as_object().ok_or("space 缺少 advances")?
                        {
                            let advance = value
                                .as_f64()
                                .map(|n| n as f32)
                                .filter(|n| n.is_finite())
                                .ok_or("space advance 无效")?;
                            advances.insert(character(ch)?, advance);
                        }
                        Provider::Space(advances)
                    }
                    "bitmap" => {
                        let height = number(entry, "height", Some(8.))?;
                        let ascent = number(entry, "ascent", None)?;
                        if height <= 0. || ascent > height {
                            return Err("bitmap height / ascent 无效".into());
                        }
                        let chars = entry["chars"]
                            .as_array()
                            .ok_or("bitmap 缺少 chars")?
                            .iter()
                            .map(|row| {
                                row.as_str()
                                    .map(|row| row.chars().collect::<Vec<_>>())
                                    .ok_or("bitmap 字符行不是 String")
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if chars.is_empty()
                            || chars[0].is_empty()
                            || chars.iter().any(|row| row.len() != chars[0].len())
                        {
                            return Err("bitmap 字符表为空或行宽不一致".into());
                        }
                        let file = qualified(text(entry, "file")?)?;
                        Provider::Bitmap {
                            file: file
                                .strip_suffix(".png")
                                .ok_or("bitmap 必须引用 PNG")?
                                .into(),
                            height,
                            ascent,
                            chars,
                            legacy: false,
                        }
                    }
                    "unihex" => {
                        let mut overrides = Vec::new();
                        if let Some(ranges) = entry.get("size_overrides") {
                            for range in
                                ranges.as_array().ok_or("unihex size_overrides 不是数组")?
                            {
                                let from = character(text(range, "from")?)? as u32;
                                let to = character(text(range, "to")?)? as u32;
                                let left = range["left"]
                                    .as_u64()
                                    .filter(|n| *n < 32)
                                    .ok_or("unihex left 无效")?
                                    as u8;
                                let right = range["right"]
                                    .as_u64()
                                    .filter(|n| *n < 32)
                                    .ok_or("unihex right 无效")?
                                    as u8;
                                if from > to || left > right {
                                    return Err("unihex 覆盖范围无效".into());
                                }
                                overrides.push((from, to, left, right));
                            }
                        }
                        Provider::Unihex {
                            file: qualified(text(entry, "hex_file")?)?,
                            overrides,
                        }
                    }
                    "ttf" => {
                        let size = number(entry, "size", Some(11.))?;
                        let oversample = number(entry, "oversample", Some(1.))?;
                        if size <= 0. || oversample <= 0. || !(size * oversample).is_finite() {
                            return Err("TTF size / oversample 无效".into());
                        }
                        let shift = if let Some(value) = entry.get("shift") {
                            let values = value
                                .as_array()
                                .filter(|v| v.len() == 2)
                                .ok_or("TTF shift 必须为两项数组")?;
                            let number = |value: &Value| {
                                value
                                    .as_f64()
                                    .map(|n| n as f32)
                                    .filter(|n| n.is_finite())
                                    .ok_or("TTF shift 必须为有限数值")
                            };
                            [number(&values[0])?, number(&values[1])?]
                        } else {
                            [0.; 2]
                        };
                        let mut skip = BTreeSet::new();
                        if let Some(value) = entry.get("skip") {
                            if let Some(text) = value.as_str() {
                                skip.extend(text.chars());
                            } else {
                                for text in value
                                    .as_array()
                                    .ok_or("TTF skip 必须为字符串或字符串数组")?
                                {
                                    skip.extend(
                                        text.as_str().ok_or("TTF skip 数组项必须为字符串")?.chars(),
                                    );
                                }
                            }
                        }
                        Provider::TrueType {
                            file: qualified(text(entry, "file")?)?,
                            size,
                            oversample,
                            shift,
                            skip,
                        }
                    }
                    other => Provider::Unsupported(other.to_owned()),
                });
            }
        }
        let cost = providers
            .iter()
            .try_fold(
                providers.len() * std::mem::size_of::<Provider>(),
                |sum, provider| {
                    let bytes = match provider {
                        Provider::Reference(id) => id.len(),
                        Provider::Space(values) => values.len().checked_mul(72)?,
                        Provider::Bitmap { file, chars, .. } => file
                            .len()
                            .checked_add(chars.len() * std::mem::size_of::<Vec<char>>())?
                            .checked_add(chars.iter().map(|row| row.len() * 4).sum::<usize>())?,
                        Provider::Unihex { file, overrides } => file.len().checked_add(
                            overrides.len() * std::mem::size_of::<(u32, u32, u8, u8)>(),
                        )?,
                        Provider::TrueType { file, skip, .. } => {
                            file.len().checked_add(skip.len().checked_mul(64)?)?
                        }
                        Provider::Unsupported(kind) => kind.len(),
                        Provider::LegacyUnicode => 0,
                    };
                    sum.checked_add(bytes)
                },
            )
            .ok_or("字体提供器缓存大小溢出")?;
        self.budget(cost)?;
        self.working_bytes += cost;
        let providers = Arc::new(providers);
        self.providers.insert(id.into(), Arc::clone(&providers));
        Ok(providers)
    }
    fn hex(
        &mut self,
        file: &str,
        stop: &dyn Fn() -> bool,
    ) -> Result<Arc<BTreeMap<u32, crate::font_unihex::Glyph>>, String> {
        if let Some(hex) = self.hex.get(file) {
            return Ok(Arc::clone(hex));
        }
        let binary = self
            .bank
            .font_resources()
            .binary(file)
            .ok_or_else(|| format!("缺少 unihex 字体 {file}"))?;
        let entries = self.bank.font_resources().unihex.load(
            file,
            binary,
            |bytes| self.budget(bytes),
            stop,
        )?;
        self.working_bytes += crate::font_unihex::bytes(&entries)?;
        self.hex.insert(file.into(), Arc::clone(&entries));
        Ok(entries)
    }
    pub(crate) fn glyph(
        &mut self,
        font: &str,
        ch: char,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Arc<Glyph>>, String> {
        self.resolve(font, ch, &mut BTreeSet::new(), stop)
    }
    pub(crate) fn glyph_or_missing(
        &mut self,
        font: &str,
        ch: char,
        stop: &dyn Fn() -> bool,
    ) -> Result<Arc<Glyph>, String> {
        if let Some(glyph) = self.glyph(font, ch, stop)? {
            return Ok(glyph);
        }
        if self.missing_logged.insert(font.to_owned()) {
            log::warn!(
                "[fonts] 字体 {font} 缺少 U+{:04X} 等字符；显示原版缺字方框",
                ch as u32
            );
        }
        static MISSING: std::sync::OnceLock<Arc<Glyph>> = std::sync::OnceLock::new();
        Ok(Arc::clone(MISSING.get_or_init(|| {
            let mut rgba = Vec::with_capacity(5 * 8 * 4);
            for y in 0..8 {
                for x in 0..5 {
                    rgba.extend_from_slice(&[
                        255,
                        255,
                        255,
                        if x == 0 || x == 4 || y == 0 || y == 7 {
                            255
                        } else {
                            0
                        },
                    ]);
                }
            }
            Arc::new(Glyph {
                legacy: None,
                width: 5,
                height: 8,
                rgba,
                scale: 1.,
                top: 0.,
                left: 0.,
                advance: 6.,
                bold_offset: 1.,
                shadow_offset: 1.,
            })
        })))
    }
    fn resolve(
        &mut self,
        font: &str,
        ch: char,
        stack: &mut BTreeSet<String>,
        stop: &dyn Fn() -> bool,
    ) -> Result<Option<Arc<Glyph>>, String> {
        if stop() {
            return Err("字形准备已取消".into());
        }
        let key = (font.to_owned(), ch);
        if let Some(glyph) = self.glyphs.get(&key) {
            return Ok(glyph.clone());
        }
        if !stack.insert(font.to_owned()) {
            return Err(format!("字体引用出现循环：{font}"));
        }
        let providers = self.providers(font)?;
        let mut found = None;
        for provider in providers.iter() {
            let glyph = match provider {
                Provider::Space(advances) => advances.get(&ch).map(|advance| {
                    Arc::new(Glyph {
                        legacy: None,
                        width: 0,
                        height: 0,
                        rgba: Vec::new(),
                        scale: 1.,
                        top: 0.,
                        left: 0.,
                        advance: *advance,
                        bold_offset: 1.,
                        shadow_offset: 1.,
                    })
                }),
                Provider::Reference(id) => self.resolve(id, ch, stack, stop)?,
                Provider::Bitmap {
                    file,
                    height,
                    ascent,
                    chars,
                    legacy,
                } => {
                    let position = if *legacy {
                        chars.iter().enumerate().find_map(|(row, chars)| {
                            chars
                                .iter()
                                .position(|code| *code == ch && ch != '\0')
                                .map(|col| (row, col))
                        })
                    } else {
                        chars.iter().enumerate().rev().find_map(|(row, chars)| {
                            chars
                                .iter()
                                .rposition(|code| *code == ch && ch != '\0')
                                .map(|col| (row, col))
                        })
                    };
                    if let Some((row, col)) = position {
                        let image = self
                            .bank
                            .font_resources()
                            .image(file)
                            .ok_or_else(|| format!("缺少字体位图 {file}"))?;
                        let cols = chars[0].len() as u32;
                        let rows = chars.len() as u32;
                        if image.width % cols != 0 || image.height % rows != 0 {
                            return Err("字体位图不能整除字符表".into());
                        }
                        let width = image.width / cols;
                        let pixel_height = image.height / rows;
                        if width == 0 || pixel_height == 0 {
                            return Err("字体位图单格为空".into());
                        }
                        let scale = *height / pixel_height as f32;
                        let mut occupied = 0;
                        let bytes = width as usize * pixel_height as usize * 4;
                        self.budget(bytes)?;
                        let mut rgba = Vec::with_capacity(bytes);
                        for y in 0..pixel_height {
                            for x in 0..width {
                                let at = (((row as u32 * pixel_height + y) * image.width
                                    + col as u32 * width
                                    + x)
                                    * 4) as usize;
                                let pixel = image
                                    .rgba
                                    .get(at..at + 4)
                                    .ok_or("字体位图 RGBA 尺寸不匹配")?;
                                if pixel[3] != 0 {
                                    occupied = occupied.max(x + 1);
                                }
                                rgba.extend_from_slice(pixel);
                            }
                        }
                        let advance = (occupied as f32
                            * if *legacy { 8. / width as f32 } else { scale }
                            + 0.5)
                            .floor()
                            + 1.;
                        let metrics = if *legacy {
                            Some(LegacyMetrics {
                                extent: [(advance - 1.).max(0.), 7.99],
                                uv: [0., 0., ((advance - 1.) / 8.).clamp(0., 1.), 0.99875],
                                measure: advance,
                            })
                        } else {
                            None
                        };
                        Some(Arc::new(Glyph {
                            legacy: metrics,
                            width,
                            height: pixel_height,
                            rgba,
                            scale,
                            top: 7. - *ascent,
                            left: 0.,
                            advance,
                            bold_offset: 1.,
                            shadow_offset: 1.,
                        }))
                    } else {
                        None
                    }
                }
                Provider::Unihex { file, overrides } => {
                    let hex = self.hex(file, stop)?;
                    if let Some(glyph) = hex.get(&(ch as u32)) {
                        let bounds = overrides
                            .iter()
                            .rev()
                            .find(|(from, to, _, _)| (*from..=*to).contains(&(ch as u32)))
                            .map(|(_, _, left, right)| (*left, *right));
                        let (left, right) = bounds.unwrap_or_else(|| {
                            let bits = glyph.rows.iter().fold(0u32, |a, b| a | b);
                            if bits == 0 {
                                (0, glyph.width - 1)
                            } else {
                                (
                                    (bits.leading_zeros() - (32 - u32::from(glyph.width))) as u8,
                                    (u32::from(glyph.width) - bits.trailing_zeros() - 1) as u8,
                                )
                            }
                        });
                        let width = u32::from(right - left + 1);
                        self.budget(width as usize * 16 * 4)?;
                        let mut rgba = Vec::with_capacity(width as usize * 16 * 4);
                        for row in glyph.rows {
                            for x in left..=right {
                                // size_overrides 可以为窄源字形补齐留白，不能将移位越界当作有效像素。
                                let alpha =
                                    if x < glyph.width && row & (1 << (glyph.width - x - 1)) != 0 {
                                        255
                                    } else {
                                        0
                                    };
                                rgba.extend_from_slice(&[255, 255, 255, alpha]);
                            }
                        }
                        Some(Arc::new(Glyph {
                            legacy: None,
                            width,
                            height: 16,
                            rgba,
                            scale: 0.5,
                            top: 0.,
                            left: 0.,
                            advance: (width / 2 + 1) as f32,
                            bold_offset: 0.5,
                            shadow_offset: 0.5,
                        }))
                    } else {
                        None
                    }
                }
                Provider::LegacyUnicode => {
                    let code = ch as u32;
                    if code > 0xffff {
                        continue;
                    }
                    let sizes = self
                        .bank
                        .font_resources()
                        .binary("minecraft:font/glyph_sizes.bin")
                        .ok_or("准确旧字体缺少 glyph_sizes.bin")?;
                    if sizes.len() != 65536 {
                        return Err("旧字体宽度表必须为 65536 字节".into());
                    }
                    let size = sizes[code as usize];
                    if size == 0 {
                        continue;
                    }
                    let left = size >> 4;
                    let right = (size & 15) + 1;
                    if left >= right {
                        return Err(format!("旧字体 U+{code:04X} 宽度边界无效"));
                    }
                    let image = self
                        .bank
                        .font_resources()
                        .image(&format!("minecraft:font/unicode_page_{:02x}", code >> 8))
                        .ok_or_else(|| format!("缺少旧 Unicode 字库页 {:02x}", code >> 8))?;
                    if image.width % 16 != 0
                        || image.height % 16 != 0
                        || image.width == 0
                        || image.height == 0
                    {
                        return Err("旧 Unicode 字库页不能整除 16×16 字符网格".into());
                    }
                    let width = image.width / 16;
                    let height = image.height / 16;
                    let bytes = (width as usize)
                        .checked_mul(height as usize)
                        .and_then(|n| n.checked_mul(4))
                        .ok_or("旧 Unicode 字形大小溢出")?;
                    self.budget(bytes)?;
                    let mut rgba = Vec::new();
                    rgba.try_reserve_exact(bytes).map_err(|e| e.to_string())?;
                    let x = (code & 15) * width;
                    let y = ((code & 255) >> 4) * height;
                    for row in 0..height {
                        let begin = (((y + row) * image.width + x) * 4) as usize;
                        rgba.extend_from_slice(
                            image
                                .rgba
                                .get(begin..begin + width as usize * 4)
                                .ok_or("旧字库 RGBA 数据不完整")?,
                        );
                    }
                    let measure = f32::from((right - left) / 2 + 1);
                    let metrics = LegacyMetrics {
                        extent: [(f32::from(right - left) - 0.02) / 2., 7.99],
                        uv: [
                            f32::from(left) / 16.,
                            0.,
                            (f32::from(right) - 0.02) / 16.,
                            0.99875,
                        ],
                        measure,
                    };
                    Some(Arc::new(Glyph {
                        legacy: Some(metrics),
                        width,
                        height,
                        rgba,
                        scale: 8. / height as f32,
                        top: 0.,
                        left: 0.,
                        advance: f32::from(right - left) / 2. + 1.,
                        bold_offset: 0.5,
                        shadow_offset: 0.5,
                    }))
                }
                Provider::TrueType {
                    file,
                    size,
                    oversample,
                    shift,
                    skip,
                } => {
                    if skip.contains(&ch) {
                        continue;
                    }
                    if !self.faces.contains_key(file) {
                        let bytes = self
                            .bank
                            .font_resources()
                            .binary(file)
                            .ok_or_else(|| format!("缺少 TTF 字体 {file}"))?;
                        self.budget(bytes.len())?;
                        let face = FontArc::try_from_vec(bytes.as_ref().clone())
                            .map_err(|e| format!("TTF 字体 {file} 无效：{e}"))?;
                        self.working_bytes += bytes.len();
                        self.faces.insert(file.clone(), face);
                    }
                    let font = self.faces.get(file).ok_or("TTF 字体实例缺失")?;
                    let id = font.glyph_id(ch);
                    if id.0 == 0 {
                        continue;
                    }
                    // ab_glyph 的缩放单位是 ascent-descent；先换算到原版使用的像素 EM。
                    let pixels = (*size * *oversample).round();
                    let units = font
                        .units_per_em()
                        .filter(|v| *v > 0.)
                        .ok_or("TTF units_per_em 无效")?;
                    let scale = pixels * font.height_unscaled() / units;
                    if !scale.is_finite() || scale <= 0. {
                        return Err("TTF 实际像素缩放无效".into());
                    }
                    let advance = font.as_scaled(scale).h_advance(id) / *oversample;
                    if !advance.is_finite() {
                        return Err("TTF 字形 advance 不是有限数值".into());
                    }
                    let glyph = ab_glyph::Glyph {
                        id,
                        scale: PxScale::from(scale),
                        position: ab_glyph::point(shift[0] * *oversample, shift[1] * *oversample),
                    };
                    if let Some(outline) = font.outline_glyph(glyph) {
                        let bounds = outline.px_bounds();
                        let width = bounds.width() as u32;
                        let height = bounds.height() as u32;
                        let bytes = (width as usize)
                            .checked_mul(height as usize)
                            .and_then(|n| n.checked_mul(4))
                            .ok_or("TTF 字形尺寸溢出")?;
                        self.budget(bytes)?;
                        let mut rgba = Vec::new();
                        rgba.try_reserve_exact(bytes).map_err(|e| e.to_string())?;
                        rgba.resize(bytes, 0);
                        outline.draw(|x, y, coverage| {
                            let at = (y as usize * width as usize + x as usize) * 4;
                            rgba[at..at + 4].copy_from_slice(&[
                                255,
                                255,
                                255,
                                (coverage.clamp(0., 1.) * 255.).round() as u8,
                            ]);
                        });
                        if stop() {
                            return Err("TTF 字形准备已取消".into());
                        }
                        Some(Arc::new(Glyph {
                            legacy: None,
                            width,
                            height,
                            rgba,
                            scale: 1. / *oversample,
                            top: 7. + bounds.min.y / *oversample,
                            left: bounds.min.x / *oversample,
                            advance,
                            bold_offset: 1.,
                            shadow_offset: 1.,
                        }))
                    } else if ch.is_whitespace() || advance == 0. {
                        Some(Arc::new(Glyph {
                            legacy: None,
                            width: 0,
                            height: 0,
                            rgba: Vec::new(),
                            scale: 1. / *oversample,
                            top: 0.,
                            left: 0.,
                            advance,
                            bold_offset: 1.,
                            shadow_offset: 1.,
                        }))
                    } else {
                        return Err(format!(
                            "TTF 字符 U+{:04X} 没有可解析的轮廓；彩色 / SVG 字形需要对应字体提供器",
                            ch as u32
                        ));
                    }
                }
                Provider::Unsupported(kind) => {
                    return Err(format!(
                        "字体提供器 {kind} 尚无法解释，不能用系统字形冒充资源字体"
                    ))
                }
            };
            if glyph.is_some() {
                found = glyph;
                break;
            }
        }
        stack.remove(font);
        let cost = found.as_ref().map_or(0, |glyph| glyph.rgba.len()) + key.0.len() + 128;
        // 只回收派生缓存，不截断字体来源或丢弃仍需绘制的字符。
        while self.glyph_bytes.saturating_add(cost) > 32 * 1024 * 1024 && !self.glyphs.is_empty() {
            if let Some((key, glyph)) = self.glyphs.pop_first() {
                self.glyph_bytes -=
                    glyph.as_ref().map_or(0, |glyph| glyph.rgba.len()) + key.0.len() + 128;
            }
        }
        self.budget(cost)?;
        self.glyph_bytes += cost;
        self.glyphs.insert(key, found.clone());
        Ok(found)
    }
}
