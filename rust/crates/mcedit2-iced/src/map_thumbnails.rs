//! 当前资料库页的真实地图缩略图；只在可见时读取，缓存不随重绘重复解码。
use super::{map_data, map_edit, work, Action as MapAction, App, Message};
use iced::{
    widget::{container, image, text, tooltip},
    Element, Task,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const EDGE: u32 = 48;
#[derive(Debug, Clone, PartialEq, Eq)]
struct Source {
    key: (usize, usize),
    root: PathBuf,
    version: Option<i32>,
    maps: Option<map_edit::View>,
    entries: Vec<(i32, String)>,
}
#[derive(Default)]
pub(super) struct State {
    source: Option<Source>,
    nonce: u64,
    cancel: Arc<AtomicBool>,
    items: BTreeMap<i32, Result<image::Handle, String>>,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Completed(
        u64,
        (usize, usize),
        Result<Vec<(i32, Result<image::Handle, String>)>, String>,
    ),
}
impl State {
    pub(super) fn invalidate(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.nonce = self.nonce.wrapping_add(1);
        self.source = None;
        self.items.clear();
    }
    pub(super) fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.nonce = self.nonce.wrapping_add(1);
        if let Some(source) = &self.source {
            for &(id, _) in &source.entries {
                self.items
                    .entry(id)
                    .or_insert_with(|| Err("缩略图读取已取消".into()));
            }
        }
    }
    pub(super) fn view(&self, id: i32) -> Element<'_, Message> {
        let content: Element<'_, Message> = match self.items.get(&id) {
            Some(Ok(handle)) => image(handle.clone())
                .width(EDGE as f32)
                .height(EDGE as f32)
                .content_fit(iced::ContentFit::Contain)
                .filter_method(image::FilterMethod::Nearest)
                .into(),
            Some(Err(error)) => {
                return tooltip(
                    container(text("读取失败").size(11)).center(EDGE as f32),
                    text(error),
                    tooltip::Position::FollowCursor,
                )
                .into()
            }
            None => text("读取中…").size(11).into(),
        };
        container(content).center(EDGE as f32).into()
    }
}
fn thumbnail(map: &map_data::MapImage, stop: &dyn Fn() -> bool) -> Result<image::Handle, String> {
    let edge = map.width.max(map.height);
    let width = (u64::from(map.width) * u64::from(EDGE) / u64::from(edge)).max(1) as u32;
    let height = (u64::from(map.height) * u64::from(EDGE) / u64::from(edge)).max(1) as u32;
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        if stop() {
            return Err("地图缩略图读取已取消".into());
        }
        let sy = u64::from(y) * u64::from(map.height) / u64::from(height);
        for x in 0..width {
            let sx = u64::from(x) * u64::from(map.width) / u64::from(width);
            let offset = (sy * u64::from(map.width) + sx) as usize * 4;
            rgba.extend_from_slice(&map.rgba[offset..offset + 4]);
        }
    }
    Ok(image::Handle::from_rgba(width, height, rgba))
}
pub(super) fn maintain(app: &mut App, visible: bool) -> Task<Message> {
    let key = app.current_key();
    let panel = &mut app.map_editor;
    let next = key
        .filter(|key| visible && panel.key == Some(*key))
        .and_then(|key| {
            panel.visible_catalog.as_ref().map(|catalog| {
                let root = app.worlds[key.0].root.clone();
                let page = panel
                    .catalog_page
                    .min(catalog.entries.len().div_ceil(64).max(1) - 1);
                Source {
                    key,
                    maps: panel.source_view(&root),
                    root,
                    version: app.worlds[key.0].data_version,
                    entries: catalog
                        .entries
                        .iter()
                        .skip(page * 64)
                        .take(64)
                        .map(|entry| (entry.id, entry.sha256.clone()))
                        .collect(),
                }
            })
        });
    let state = &mut panel.thumbnails;
    if state.source == next {
        return Task::none();
    }
    state.invalidate();
    let Some(source) = next else {
        return Task::none();
    };
    state.source = Some(source.clone());
    if source.entries.is_empty() {
        return Task::none();
    }
    state.cancel = Arc::new(AtomicBool::new(false));
    let cancel = Arc::clone(&state.cancel);
    let nonce = state.nonce;
    let key = source.key;
    work(
        move || {
            let stop = || cancel.load(Ordering::Acquire);
            let mut items = Vec::with_capacity(source.entries.len());
            for (id, expected) in source.entries {
                if stop() {
                    return Err("地图缩略图读取已取消".into());
                }
                let result = (|| {
                    let map = match &source.maps {
                        Some(maps) => maps.read(&source.root, id, source.version)?,
                        None => map_data::read(&source.root, id, source.version)?,
                    };
                    if map.sha256 != expected {
                        return Err("地图内容已改变，请重新扫描".into());
                    }
                    thumbnail(&map, &stop)
                })();
                if stop() {
                    return Err("地图缩略图读取已取消".into());
                }
                items.push((id, result));
            }
            Ok(items)
        },
        move |result| MapAction::Thumbnails(Action::Completed(nonce, key, result)),
    )
}
pub(super) fn accept(app: &mut App, action: Action) {
    let Action::Completed(nonce, key, result) = action;
    let current = app.current_key();
    let state = &mut app.map_editor.thumbnails;
    if current != Some(key) || nonce != state.nonce || state.cancel.load(Ordering::Acquire) {
        return;
    }
    match result {
        Ok(items) => {
            for (id, image) in items {
                if let Err(error) = &image {
                    log::error!("[map-thumbnail] #{id}: {error}");
                }
                state.items.insert(id, image);
            }
        }
        Err(error) => {
            log::error!("[map-thumbnail] {error}");
            for &(id, _) in &state
                .source
                .as_ref()
                .expect("缩略图完成请求仍有来源")
                .entries
            {
                state.items.insert(id, Err(error.clone()));
            }
        }
    }
}
