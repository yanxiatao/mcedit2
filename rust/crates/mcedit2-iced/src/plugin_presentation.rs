//! 动作的图标和按键元数据；图像按实际载荷缓存，控件复用同一原生动作入口。
use super::*;
use base64::Engine;
use iced::widget::{image, row, text};
use mcedit2_plugins::tool_pick::{ActionState, IconFile};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
    sync::OnceLock,
};

#[derive(Clone)]
struct Variant {
    handle: image::Handle,
    mode: u8,
    state: u8,
    size: [u32; 2],
    bytes: usize,
}
#[derive(Default)]
pub(super) struct State {
    icons: BTreeMap<String, Vec<Variant>>,
}
fn identity(files: &[IconFile]) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(files).map_err(|error| error.to_string())?)
    ))
}
pub(super) fn action(spec: &serde_json::Value) -> Result<Option<ActionState>, String> {
    spec.get("action")
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|error| format!("表单动作状态无效：{error}"))
        })
        .transpose()
}
impl State {
    pub(super) fn install(
        &mut self,
        commands: &[PluginCommand],
        budget: EditWorkBudget,
    ) -> Result<(), String> {
        let mut states = Vec::new();
        let mut icon_sets = Vec::new();
        for command in commands {
            if command.action.is_none() {
                if let Some(icon) = &command.icon {
                    icon_sets.push(icon.clone());
                }
            }
            if let Some(state) = &command.action {
                states.push(state.clone());
            }
            for spec in &command.options {
                if let Some(state) = action(spec)? {
                    states.push(state);
                }
            }
        }
        let mut wanted = BTreeSet::new();
        let mut added = BTreeMap::new();
        let mut retained = self
            .icons
            .values()
            .flatten()
            .map(|variant| variant.bytes)
            .sum::<usize>();
        for state in states {
            state.validate()?;
            for sequence in &state.shortcuts {
                for key in sequence {
                    crate::keyboard_bindings::Chord::parse_action(key)?;
                }
            }
            if let Some(files) = state.icon {
                icon_sets.push(files);
            }
        }
        for files in icon_sets {
            mcedit2_plugins::tool_pick::validate_icon(&files)?;
            let key = identity(&files)?;
            if !wanted.insert(key.clone()) || self.icons.contains_key(&key) {
                continue;
            }
            let mut variants = Vec::new();
            for file in files {
                let available = budget
                    .bytes()
                    .checked_sub(retained)
                    .ok_or("图标缓存超过实际工作预算")?;
                let variant = decode(&file, available)?;
                retained = retained
                    .checked_add(variant.bytes)
                    .ok_or("图标缓存工作集溢出")?;
                budget.check(retained).map_err(|error| error.to_string())?;
                variants.push(variant);
            }
            added.insert(key, variants);
        }
        self.icons.retain(|key, _| wanted.contains(key));
        self.icons.extend(added);
        Ok(())
    }
    fn icon(&self, action: &ActionState, selected: bool) -> Result<Option<image::Handle>, String> {
        let Some(files) = &action.icon else {
            return Ok(None);
        };
        let mode = if !action.enabled {
            1
        } else if selected {
            3
        } else {
            0
        };
        let state = u8::from(!action.checked);
        Ok(Some(self.variant(files, mode, state)?))
    }
    fn variant(&self, files: &[IconFile], mode: u8, state: u8) -> Result<image::Handle, String> {
        let key = identity(files)?;
        let variant = self
            .icons
            .get(&key)
            .ok_or("动作图标未接入当前命令目录")?
            .iter()
            .min_by_key(|variant| {
                (
                    u8::from(variant.state != state),
                    if variant.mode == mode {
                        0
                    } else if variant.mode == 0 {
                        1
                    } else {
                        2
                    },
                    variant.size[0].abs_diff(32) + variant.size[1].abs_diff(32),
                )
            })
            .ok_or("动作没有实际图标")?;
        Ok(variant.handle.clone())
    }
    pub(super) fn label<'a>(
        &self,
        label: String,
        action: Option<&ActionState>,
        selected: bool,
        declared: Option<&[IconFile]>,
    ) -> Result<Element<'a, Message>, String> {
        let mut content = row![].spacing(6).align_y(iced::Alignment::Center);
        if let Some(action) = action {
            if let Some(handle) = self.icon(action, selected)? {
                content = content.push(image(handle).width(20).height(20));
            }
        } else if let Some(files) = declared {
            content = content.push(
                image(self.variant(files, if selected { 3 } else { 0 }, 1)?)
                    .width(20)
                    .height(20),
            );
        }
        content = content.push(text(label));
        if let Some(action) = action {
            if !action.shortcuts.is_empty() {
                let hints = action
                    .shortcuts
                    .iter()
                    .map(|keys| {
                        keys.iter()
                            .map(|key| {
                                crate::keyboard_bindings::Chord::parse_action(key)
                                    .map(|key| key.label())
                            })
                            .collect::<Result<Vec<_>, _>>()
                            .map(|keys| keys.join(", "))
                    })
                    .collect::<Result<Vec<_>, String>>()?
                    .join(" / ");
                content = content.push(text(hints).size(12));
            }
        }
        Ok(content.into())
    }
}
fn decode(file: &IconFile, budget: usize) -> Result<Variant, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&file.data)
        .map_err(|error| format!("图标 {} 的编码无效：{error}", file.source))?;
    let budget = budget
        .checked_sub(bytes.len())
        .ok_or("图标载荷超过实际工作预算")?;
    let svg = file.source.to_ascii_lowercase().ends_with(".svg")
        || file.source.to_ascii_lowercase().ends_with(".svgz")
        || bytes
            .iter()
            .copied()
            .skip_while(u8::is_ascii_whitespace)
            .take(4)
            .eq(b"<svg".iter().copied())
        || bytes.starts_with(b"<?xml");
    let (width, height, rgba) = if svg {
        let xml = if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut xml = Vec::new();
            flate2::read::GzDecoder::new(bytes.as_slice())
                .take(budget.saturating_add(1) as u64)
                .read_to_end(&mut xml)
                .map_err(|error| format!("图标 {}：{error}", file.source))?;
            if xml.len().saturating_mul(16) > budget {
                return Err("SVG 图标解压后的工作集超过预算".into());
            }
            xml
        } else {
            if bytes.len().saturating_mul(16) > budget {
                return Err("SVG 图标解析工作集超过预算".into());
            }
            bytes.clone()
        };
        let mut options = resvg::usvg::Options::default();
        options.resources_dir = std::path::Path::new(&file.source)
            .parent()
            .map(std::path::Path::to_path_buf);
        static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
        options.fontdb = Arc::clone(FONTS.get_or_init(|| {
            let mut fonts = resvg::usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        }));
        let tree = resvg::usvg::Tree::from_data(&xml, &options)
            .map_err(|error| format!("图标 {}：{error}", file.source))?;
        let size = tree.size();
        let scale = 64.0 / size.width().max(size.height());
        let width = (size.width() * scale).ceil() as u32;
        let height = (size.height() * scale).ceil() as u32;
        if u64::from(width) * u64::from(height) * 4 > budget as u64 {
            return Err("SVG 图标超过实际工作预算".into());
        }
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or("SVG 图标尺寸无效")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let mut rgba = pixmap.take();
        for pixel in rgba.chunks_exact_mut(4) {
            let alpha = u16::from(pixel[3]);
            if alpha != 0 {
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha) as u8;
                }
            }
        }
        (width, height, rgba)
    } else {
        let mut reader = ::image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| error.to_string())?;
        let mut limits = ::image::Limits::default();
        limits.max_alloc = Some(budget as u64);
        reader.limits(limits);
        let decoded = reader
            .decode()
            .map_err(|error| format!("图标 {}：{error}", file.source))?;
        let image = decoded.thumbnail(64, 64).into_rgba8();
        if decoded
            .as_bytes()
            .len()
            .saturating_add(image.as_raw().len())
            > budget
        {
            return Err("图标解码超过实际工作预算".into());
        }
        (image.width(), image.height(), image.into_raw())
    };
    Ok(Variant {
        bytes: rgba.len(),
        handle: image::Handle::from_rgba(width, height, rgba),
        mode: file.mode,
        state: file.state,
        size: file.size.unwrap_or([width, height]),
    })
}
