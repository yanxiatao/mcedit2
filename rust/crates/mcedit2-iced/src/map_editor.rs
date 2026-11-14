//! 世界地图列表、图片预览和可取消拼图转换；保存候选交由统一事务。
#[path = "map_mosaic.rs"]
mod mosaic_view;
#[path = "map_thumbnails.rs"]
mod thumbnails;

use crate::{App, Message};
use iced::{
    widget::{button, column, image, row, text, text_input},
    Element, Length, Task,
};
use mcedit2_world::{map_catalog, map_data, map_edit, map_history};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[derive(Default)]
pub(crate) struct Panel {
    key: Option<(usize, usize)>,
    scanned_for: Option<(usize, usize)>,
    version: Option<i32>,
    world_root: Option<PathBuf>,
    epoch: u64,
    cancel: Arc<AtomicBool>,
    busy: bool,
    catalog: Option<map_catalog::Catalog>,
    visible_catalog: Option<map_catalog::Catalog>,
    views: std::collections::BTreeMap<PathBuf, map_edit::View>,
    thumbnails: thumbnails::State,
    selected: Option<Arc<map_data::MapImage>>,
    base_preview: Option<image::Handle>,
    preview: Option<Result<image::Handle, String>>,
    preview_source: Option<((usize, usize), String, crate::textures::VisibilityIdentity)>,
    preview_cancel: Arc<AtomicBool>,
    preview_nonce: u64,
    image_path: String,
    mosaic: Option<Arc<map_edit::Mosaic>>,
    mosaic_preview: Option<mosaic_view::Preview>,
    source_sha: Option<String>,
    tile_index: String,
    columns: String,
    rows: String,
    stretch: bool,
    status: String,
    sessions: std::collections::BTreeMap<PathBuf, map_history::Session>,
    catalog_page: usize,
    pending_page: usize,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Scan,
    Scanned(u64, (usize, usize), Result<map_catalog::Catalog, String>),
    Select(i32),
    CopyItem,
    History(Arc<()>, usize),
    Selected(
        u64,
        (usize, usize),
        Result<(Arc<map_data::MapImage>, image::Handle), String>,
    ),
    Preview(u64, (usize, usize), Result<image::Handle, String>),
    Thumbnails(thumbnails::Action),
    Path(String),
    TileIndex(String),
    MosaicTile(Arc<map_edit::Mosaic>, usize),
    Columns(String),
    Rows(String),
    Stretch(bool),
    Convert,
    Converted(
        u64,
        (usize, usize),
        Result<(Arc<map_edit::Mosaic>, String, mosaic_view::Preview), String>,
    ),
    PrepareImport,
    PrepareDelete,
    CatalogPage(i32),
    PendingPage(i32),
    Prepared(u64, (usize, usize), Result<map_history::Session, String>),
    Undo,
    Redo,
    Cancel,
}
/// 已选地图随资源身份变化重绘，旧资源任务只取消，不向新画面发布。
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let visible = !app.desktop.show_recent
        && app.structures.active.is_none()
        && ((app.show_advanced && app.advanced_section == crate::AdvancedSection::MapsEditor)
            || crate::library::maps_visible(app));
    if visible
        && !app.map_editor.busy
        && app.current_key().is_some()
        && app.map_editor.scanned_for != app.current_key()
    {
        return update(app, Action::Scan);
    }
    let thumbnails = thumbnails::maintain(app, visible);
    Task::batch([thumbnails, maintain_preview(app)])
}
fn maintain_preview(app: &mut App) -> Task<Message> {
    let key = app.current_key();
    let panel = &mut app.map_editor;
    let (Some(image), Some(bank), Some(key)) =
        (panel.selected.clone(), app.texture_bank.clone(), key)
    else {
        if panel.preview_source.take().is_some() {
            panel.preview_cancel.store(true, Ordering::Release);
            panel.preview = None;
        }
        return Task::none();
    };
    let source = (key, image.texture_id(), bank.visibility_id());
    if panel.preview_source.as_ref() == Some(&source) {
        return Task::none();
    }
    panel.preview_cancel.store(true, Ordering::Release);
    panel.preview_cancel = Arc::new(AtomicBool::new(false));
    panel.preview_nonce = panel.preview_nonce.wrapping_add(1);
    let nonce = panel.preview_nonce;
    let cancel = Arc::clone(&panel.preview_cancel);
    panel.preview_source = Some(source);
    panel.preview = None;
    work(
        move || crate::map_geometry::preview(&image, &bank, &|| cancel.load(Ordering::Acquire)),
        move |result| Action::Preview(nonce, key, result),
    )
}
fn work<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl Fn(Result<T, String>) -> Action + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                if tx.send(f()).is_err() {
                    log::debug!("[map] 后台结果接收端已关闭");
                }
            });
            rx.await
                .unwrap_or_else(|e| Err(format!("地图后台任务异常：{e}")))
        },
        move |result| Message::MapEditor(done(result)),
    )
}
impl Panel {
    fn first_map(&self) -> Option<i32> {
        self.visible_catalog
            .as_ref()?
            .entries
            .first()
            .map(|entry| entry.id)
    }
    pub(crate) fn source_view(&self, root: &std::path::Path) -> Option<map_edit::View> {
        self.views.get(root).cloned()
    }
    /// 候选历史和磁盘基线分开维护，避免把新建地图误当成已有磁盘文件分配 ID。
    fn refresh_sources(&mut self, root: &std::path::Path) {
        if let Some(session) = self.sessions.get(root) {
            self.views.insert(root.to_owned(), session.view());
        } else {
            self.views.remove(root);
        }
        if self.world_root.as_deref() == Some(root) {
            self.refresh_catalog();
        }
    }
    fn refresh_catalog(&mut self) {
        self.visible_catalog = self.catalog.as_ref().map(|catalog| {
            match self
                .world_root
                .as_ref()
                .and_then(|root| self.views.get(root))
            {
                Some(view) => view.catalog(catalog),
                None => catalog.clone(),
            }
        });
        if self.selected.as_ref().is_some_and(|image| {
            !self.visible_catalog.as_ref().is_some_and(|catalog| {
                catalog
                    .entries
                    .iter()
                    .any(|entry| entry.id == image.id && entry.sha256 == image.sha256)
            })
        }) {
            self.selected = None;
            self.base_preview = None;
            self.preview_cancel.store(true, Ordering::Release);
            self.preview = None;
            self.preview_source = None;
        }
        let pages = self
            .visible_catalog
            .as_ref()
            .map_or(1, |c| c.entries.len().div_ceil(64).max(1));
        self.catalog_page = self.catalog_page.min(pages - 1);
    }
    pub(crate) fn image_path(&self) -> &str {
        &self.image_path
    }
    fn clear_mosaic(&mut self) {
        self.mosaic = None;
        self.mosaic_preview = None;
        self.source_sha = None;
    }
    pub(crate) fn has_candidates(&self, root: &std::path::Path) -> bool {
        self.sessions
            .get(root)
            .is_some_and(map_history::Session::has_changes)
    }
    pub(crate) fn change_count(&self, root: &std::path::Path) -> usize {
        self.sessions
            .get(root)
            .map_or(0, map_history::Session::change_count)
    }
    pub(crate) fn discard(&mut self, root: &std::path::Path) {
        self.sessions.remove(root);
        self.refresh_sources(root);
        if self.world_root.as_deref() == Some(root) {
            self.invalidate();
        }
    }
    pub(crate) fn invalidate(&mut self) {
        self.cancel();
        self.key = None;
        self.scanned_for = None;
        self.thumbnails.invalidate();
        self.world_root = None;
        self.catalog = None;
        self.visible_catalog = None;
        self.selected = None;
        self.base_preview = None;
        self.preview = None;
        self.clear_mosaic();
        self.status.clear();
    }
    pub(crate) fn session(&self, root: &std::path::Path) -> Option<map_history::Session> {
        self.sessions.get(root).cloned()
    }
    pub(crate) fn saved(
        &mut self,
        root: &std::path::Path,
        revision: Option<&Arc<()>>,
        result: Result<map_history::Session, String>,
    ) -> Result<(), String> {
        let matches = match (self.sessions.get(root), revision) {
            (Some(session), Some(revision)) => Arc::ptr_eq(revision, &session.revision()),
            (None, None) => true,
            _ => false,
        };
        if !matches {
            return Err("地图保存点交接来源已改变，当前编辑仍保留".into());
        }
        self.sessions.insert(root.to_owned(), result?);
        self.refresh_sources(root);
        if self.world_root.as_deref() != Some(root) {
            return Ok(());
        }
        self.cancel();
        self.scanned_for = None;
        self.thumbnails.invalidate();
        self.catalog = None;
        self.visible_catalog = None;
        self.selected = None;
        self.base_preview = None;
        self.clear_mosaic();
        self.status = "地图已保存，撤销与重做历史已保留".into();
        Ok(())
    }
    fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.preview_cancel.store(true, Ordering::Release);
        if self.preview.is_none() && self.preview_source.is_some() {
            self.preview = Some(Err("地图预览已取消".into()));
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.busy = false;
    }
    pub(crate) fn history(&self, root: &std::path::Path) -> mcedit2_world::history::Timeline {
        self.sessions.get(root).map_or_else(
            || mcedit2_world::history::Timeline {
                revision: Arc::new(()),
                cursor: 0,
                entries: Vec::new(),
            },
            map_history::Session::history,
        )
    }
    pub(crate) fn history_editable(&self) -> bool {
        !self.busy
    }
    pub(crate) fn history_report(&self) -> &str {
        &self.status
    }
    fn goto_history(
        &mut self,
        root: &std::path::Path,
        revision: &Arc<()>,
        target: usize,
    ) -> Result<(), String> {
        self.cancel();
        self.sessions
            .get_mut(root)
            .ok_or("地图历史会话已关闭")?
            .goto(revision, target)
            .map_err(|error| error.to_string())?;
        self.refresh_sources(root);
        self.status = format!("已切换到地图修订 {target}；磁盘未改变");
        Ok(())
    }
    pub fn library_view(
        &self,
        key: Option<(usize, usize)>,
        drag_drop: &crate::drag_drop::State,
    ) -> Element<'_, Message> {
        let mut body = column![
            text("世界地图管理").size(18),
            row![
                button("扫描地图").on_press(Message::MapEditor(Action::Scan)),
                button("取消后台任务").on_press(Message::MapEditor(Action::Cancel))
            ]
            .spacing(6),
        ];
        if self.key != key {
            return body.into();
        }
        body = body.push(text(&self.status));
        if let Some(catalog) = &self.visible_catalog {
            let pages = catalog.entries.len().div_ceil(64).max(1);
            let page = self.catalog_page.min(pages - 1);
            body = body.push(
                row![
                    button("地图上一页").on_press(Message::MapEditor(Action::CatalogPage(-1))),
                    text(format!("{}/{}", page + 1, pages)),
                    button("地图下一页").on_press(Message::MapEditor(Action::CatalogPage(1)))
                ]
                .spacing(6),
            );
            for entry in catalog.entries.iter().skip(page * 64).take(64) {
                let staged = self
                    .world_root
                    .as_ref()
                    .and_then(|root| self.views.get(root))
                    .is_some_and(|view| view.contains_created(entry.id));
                let label = format!(
                    "地图 #{} · {} 字节{}",
                    entry.id,
                    entry.bytes,
                    if staged { " · 尚未保存" } else { "" }
                );
                let selected = self.selected.as_ref().is_some_and(|map| map.id == entry.id);
                let content = row![
                    self.thumbnails.view(entry.id),
                    text(label).width(Length::Fill)
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center);
                let stack = self
                    .world_root
                    .as_ref()
                    .ok_or("地图列表没有实际世界来源".into())
                    .and_then(|root| map_stack(root, self.version, entry.id));
                match stack {
                    Ok(stack) => {
                        body = body.push(crate::drag_drop::source(
                            drag_drop,
                            crate::drag_drop::Payload::Item(stack),
                            iced::widget::container(content)
                                .padding(6)
                                .width(Length::Fill)
                                .style(if selected {
                                    iced::widget::container::primary
                                } else {
                                    iced::widget::container::rounded_box
                                })
                                .into(),
                            Message::MapEditor(Action::Select(entry.id)),
                            None,
                        ))
                    }
                    Err(error) => {
                        body = body.push(column![
                            button(content)
                                .width(Length::Fill)
                                .style(if selected {
                                    button::primary
                                } else {
                                    button::secondary
                                })
                                .on_press(Message::MapEditor(Action::Select(entry.id))),
                            text(error)
                        ])
                    }
                }
            }
            for diagnostic in &catalog.diagnostics {
                body = body.push(text(diagnostic));
            }
        }
        if let Some(map) = &self.selected {
            body = body.push(text(format!(
                "地图 #{} · {}×{} · 维度 {} · 缩放 {} · {}",
                map.id,
                map.width,
                map.height,
                map.dimension,
                map.scale,
                map.source.display()
            )));
            let handle = match &self.preview {
                Some(Ok(handle)) => Some(handle.clone()),
                Some(Err(error)) => {
                    body = body.push(text(format!("地图预览失败：{error}")));
                    None
                }
                None if self.preview_source.is_some() => {
                    body = body.push(text("正在绘制地图与标记…"));
                    None
                }
                None => self.base_preview.clone(),
            };
            if let Some(handle) = handle {
                body = body.push(
                    image(handle)
                        .width(Length::Fill)
                        .height(Length::Fixed(256.))
                        .content_fit(iced::ContentFit::Contain)
                        .filter_method(image::FilterMethod::Nearest),
                );
            }
            for warning in &map.warnings {
                body = body.push(text(warning));
            }
            body = body.push(button("复制地图物品").on_press(Message::MapEditor(Action::CopyItem)));
        }
        body.into()
    }
    pub fn view(&self, drag_drop: &crate::drag_drop::State) -> Element<'_, Message> {
        let mut body = column![self.library_view(self.key, drag_drop)];
        body = body.push(
            row![
                button("预检导入拼图").on_press_maybe(
                    (!self.busy && self.mosaic.is_some() && self.catalog.is_some())
                        .then_some(Message::MapEditor(Action::PrepareImport))
                ),
                button("预检删除选中地图").on_press_maybe(
                    (!self.busy && self.selected.is_some())
                        .then_some(Message::MapEditor(Action::PrepareDelete))
                ),
                button("撤销地图编辑").on_press_maybe(
                    (!self.busy
                        && self
                            .world_root
                            .as_ref()
                            .is_some_and(|root| self.history(root).cursor > 0))
                    .then_some(Message::MapEditor(Action::Undo))
                )
            ]
            .spacing(6),
        );
        let pending = self
            .world_root
            .as_ref()
            .and_then(|root| self.sessions.get(root))
            .map_or_else(Vec::new, map_history::Session::pending_summaries);
        body = body.push(text(format!("地图待保存变化：{} 项", pending.len())));
        let pages = pending.len().div_ceil(64).max(1);
        let page = self.pending_page.min(pages - 1);
        body = body.push(
            row![
                button("上一页").on_press(Message::MapEditor(Action::PendingPage(-1))),
                text(format!("{}/{}", page + 1, pages)),
                button("下一页").on_press(Message::MapEditor(Action::PendingPage(1))),
                button("重做地图编辑").on_press_maybe(
                    (!self.busy
                        && self.world_root.as_ref().is_some_and(|root| {
                            let history = self.history(root);
                            history.cursor < history.entries.len()
                        }))
                    .then_some(Message::MapEditor(Action::Redo))
                )
            ]
            .spacing(6),
        );
        for detail in pending.into_iter().skip(page * 64).take(64) {
            body = body.push(text(detail));
        }
        body = body.push(
            row![
                text_input("导入 PNG/JPEG/GIF/BMP 路径", &self.image_path)
                    .on_input(|s| Message::MapEditor(Action::Path(s)))
                    .width(Length::Fill),
                button("选择图片…").on_press(Message::ChooseAdvancedFile(
                    crate::AdvancedFile::MapImage,
                    false,
                )),
                button("预览 128×128 拼图")
                    .on_press_maybe((!self.busy).then_some(Message::MapEditor(Action::Convert)))
            ]
            .spacing(6),
        );
        body = body.push(
            row![
                text_input("拼图列数（空为原尺寸）", &self.columns)
                    .on_input(|s| Message::MapEditor(Action::Columns(s))),
                text_input("拼图行数（空为原尺寸）", &self.rows)
                    .on_input(|s| Message::MapEditor(Action::Rows(s))),
                iced::widget::checkbox(self.stretch)
                    .label("拉伸填满")
                    .on_toggle(|v| Message::MapEditor(Action::Stretch(v)))
            ]
            .spacing(6),
        );
        if let Some(mosaic) = &self.mosaic {
            if let Some(preview) = &self.mosaic_preview {
                body = body.push(mosaic_view::view(
                    mosaic,
                    preview,
                    self.tile_index.parse().ok(),
                ));
            }
            body = body.push(text(format!(
                "共 {} 张地图，导入后通过世界保存写入。",
                mosaic.tiles.len()
            )));
            body = body.push(
                text_input("预览块序号（从 0 开始）", &self.tile_index)
                    .on_input(|s| Message::MapEditor(Action::TileIndex(s))),
            );
            if let Some(tile) = self
                .tile_index
                .parse::<usize>()
                .ok()
                .and_then(|i| mosaic.tiles.get(i))
            {
                let pixels = tile
                    .colors
                    .iter()
                    .map(|v| map_data::color_for_version(mosaic.data_version, *v as u8))
                    .collect::<Option<Vec<_>>>();
                if let Some(pixels) = pixels {
                    let rgba = pixels.into_iter().flatten().collect::<Vec<_>>();
                    body = body.push(text(format!("拼图列 {}，行 {}", tile.column, tile.row)));
                    body = body.push(
                        image(image::Handle::from_rgba(128, 128, rgba))
                            .width(Length::Fixed(256.))
                            .height(Length::Fixed(256.)),
                    );
                } else {
                    body = body.push(text("拼图包含未支持的颜色，无法显示"));
                }
            } else {
                body = body.push(text(format!(
                    "请输入 0 到 {} 的块序号。",
                    mosaic.tiles.len() - 1
                )));
            }
        }
        body.into()
    }
}
fn map_stack(
    root: &std::path::Path,
    version: Option<i32>,
    id: i32,
) -> Result<Arc<mcedit2_world::inventory::Stack>, String> {
    let catalog = crate::item_presets::get(version).ok_or("此数据版本没有准确地图物品预设")?;
    let preset = catalog
        .default_item("minecraft:filled_map")
        .ok_or("当前版本缺少填充地图预设")?;
    let context = mcedit2_world::inventory::Context {
        root: root.to_path_buf(),
        version,
    };
    let stack = preset
        .stack(context)?
        .with_fields("minecraft:filled_map", 1, None)
        .map_err(|error| error.to_string())?
        .with_map_id(id)
        .map_err(|error| error.to_string())?;
    Ok(Arc::new(stack))
}
fn select_first_if_needed(app: &mut App) -> Task<Message> {
    if app.map_editor.selected.is_none() && app.map_editor.key == app.current_key() {
        if let Some(id) = app.map_editor.first_map() {
            return update(app, Action::Select(id));
        }
    }
    Task::none()
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if let Action::Prepared(epoch, key, _) = &action {
        if app.current_key() != Some(*key) || app.map_editor.epoch != *epoch {
            return Task::none();
        }
    }
    // 统一事务已经冻结来源时保持同一修订，提交完成后推进保存点并保留历史。
    let changes_history = matches!(
        &action,
        Action::Undo
            | Action::Redo
            | Action::History(..)
            | Action::PrepareImport
            | Action::PrepareDelete
            | Action::Prepared(..)
    );
    let frozen = app.save_worker
        || app.save_planning
        || app.mixed_busy
        || app.nbt_group_busy
        || app
            .mixed_key
            .is_some_and(|owner| app.current_key().is_some_and(|key| key.0 == owner.0));
    if changes_history && (app.readonly || frozen) {
        let error = if app.readonly {
            "只读模式禁止修改地图候选"
        } else {
            "地图来源正在保存或已由统一事务冻结，请提交或回滚后再编辑"
        };
        app.map_editor.cancel();
        log::error!("[map] {error}");
        app.map_editor.status = error.into();
        return Task::none();
    }
    match action {
        Action::Thumbnails(action) => thumbnails::accept(app, action),
        Action::History(revision, target) => {
            let result = (|| {
                let key = app
                    .current_key()
                    .filter(|_| app.structures.active.is_none() && !app.desktop.show_recent)
                    .ok_or("请切换到地图所属世界")?;
                if app.readonly
                    || !app.map_editor.history_editable()
                    || app.save_worker
                    || app.save_planning
                    || app.mixed_busy
                    || app.nbt_group_busy
                {
                    return Err("当前地图为只读或仍有后台 / 保存任务".into());
                }
                app.map_editor
                    .goto_history(&app.worlds[key.0].root, &revision, target)
            })();
            if let Err(error) = result {
                log::error!("[map-history] {error}");
                app.map_editor.status = error;
            } else {
                return select_first_if_needed(app);
            }
        }
        Action::CopyItem => {
            let result = (|| {
                let key = app
                    .current_key()
                    .filter(|_| app.structures.active.is_none() && !app.desktop.show_recent)
                    .ok_or("请切换到地图所属的世界标签")?;
                if app.map_editor.key != Some(key) {
                    return Err("地图列表属于其他世界或维度，请重新扫描".into());
                }
                let map = app.map_editor.selected.as_ref().ok_or("请先选择地图")?;
                let world = &app.worlds[key.0];
                Ok::<_, String>((map_stack(&world.root, world.data_version, map.id)?, map.id))
            })();
            match result {
                Ok((stack, id)) => {
                    app.item_clipboard = Some(stack);
                    app.map_editor.status =
                        format!("已复制地图 #{id} 的物品，可粘贴到该世界的库存槽位。");
                }
                Err(error) => {
                    log::error!("[map-item] {error}");
                    app.map_editor.status = error;
                }
            }
        }
        Action::CatalogPage(delta) => {
            let max = app
                .map_editor
                .visible_catalog
                .as_ref()
                .map_or(0, |c| c.entries.len().div_ceil(64).saturating_sub(1));
            app.map_editor.catalog_page = app
                .map_editor
                .catalog_page
                .saturating_add_signed(delta as isize)
                .min(max);
        }
        Action::PendingPage(delta) => {
            let max = app
                .map_editor
                .world_root
                .as_ref()
                .and_then(|root| app.map_editor.sessions.get(root))
                .map_or(0, |session| session.pending_summaries().len())
                .div_ceil(64)
                .saturating_sub(1);
            app.map_editor.pending_page = app
                .map_editor
                .pending_page
                .saturating_add_signed(delta as isize)
                .min(max);
        }
        Action::Undo | Action::Redo => {
            if let Some(root) = app.map_editor.world_root.clone() {
                let history = app.map_editor.history(&root);
                let target = if matches!(action, Action::Undo) {
                    history.cursor.checked_sub(1)
                } else {
                    (history.cursor < history.entries.len()).then_some(history.cursor + 1)
                };
                if let Some(target) = target {
                    match app
                        .map_editor
                        .goto_history(&root, &history.revision, target)
                    {
                        Ok(()) => return select_first_if_needed(app),
                        Err(error) => {
                            log::error!("[map-history] {error}");
                            app.map_editor.status = error;
                        }
                    }
                }
            }
        }
        action @ (Action::PrepareImport | Action::PrepareDelete) => {
            if app.readonly {
                app.map_editor.status = "只读模式禁止准备地图写入".into();
                return Task::none();
            }
            app.map_editor.cancel();
            let Some(key) = app.current_key() else {
                return Task::none();
            };
            let world = app.worlds[key.0].clone();
            let dimension = world.dimensions[key.1].name.clone();
            let session = app.map_editor.session(&world.root);
            let epoch = app.map_editor.epoch;
            let catalog = app.map_editor.catalog.clone();
            let visible_catalog = app.map_editor.visible_catalog.clone();
            let mosaic = app.map_editor.mosaic.clone();
            let selected = app.map_editor.selected.clone();
            let path = PathBuf::from(app.map_editor.image_path.trim());
            let expected_sha = app.map_editor.source_sha.clone();
            app.map_editor.cancel = Arc::new(AtomicBool::new(false));
            let cancel = Arc::clone(&app.map_editor.cancel);
            app.map_editor.busy = true;
            app.map_editor.status = "正在后台预检地图和源摘要…".into();
            return work(
                move || {
                    let session = match session {
                        Some(session) => session,
                        None => map_history::Session::new(world.clone())
                            .map_err(|error| error.to_string())?,
                    };
                    if matches!(action, Action::PrepareImport) {
                        use sha2::{Digest, Sha256};
                        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                        if meta.file_type().is_symlink()
                            || !meta.is_file()
                            || meta.len() > 64 * 1024 * 1024
                        {
                            return Err("图片源类型或大小已改变".into());
                        }
                        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                        if Some(format!("{:x}", Sha256::digest(&bytes))) != expected_sha {
                            return Err("原始图片已变化，请重新转换".into());
                        }
                        drop(bytes);
                        session
                            .prepare_import(
                                &dimension,
                                &catalog.ok_or("请先扫描地图")?,
                                mosaic.ok_or("请先转换图片")?.as_ref(),
                                (path, expected_sha.ok_or("缺少图片源摘要")?),
                                256 * 1024 * 1024,
                                || cancel.load(Ordering::Acquire),
                            )
                            .map_err(|error| error.to_string())
                    } else {
                        let image = selected.ok_or("请选择地图")?;
                        let catalog = visible_catalog.ok_or("请先扫描地图")?;
                        let entry = catalog
                            .entries
                            .iter()
                            .find(|e| e.id == image.id && e.sha256 == image.sha256)
                            .ok_or("地图源已过期")?;
                        if cancel.load(Ordering::Acquire) {
                            return Err("地图删除预检已取消".into());
                        }
                        session
                            .prepare_delete(entry, 256 * 1024 * 1024, || {
                                cancel.load(Ordering::Acquire)
                            })
                            .map_err(|error| error.to_string())
                    }
                },
                move |r| Action::Prepared(epoch, key, r),
            );
        }
        Action::Prepared(epoch, key, result) => {
            if app.current_key() != Some(key) || app.map_editor.epoch != epoch {
                return Task::none();
            }
            app.map_editor.busy = false;
            match result {
                Ok(session) => {
                    let root = app.worlds[key.0].root.clone();
                    let selected = session.last_created();
                    app.map_editor.sessions.insert(root.clone(), session);
                    app.map_editor.refresh_sources(&root);
                    app.map_editor.status = "地图编辑已暂存，历史与保存点分别保留".into();
                    if let Some(id) = selected.or_else(|| app.map_editor.first_map()) {
                        if let Some(catalog) = &app.map_editor.visible_catalog {
                            if let Some(index) =
                                catalog.entries.iter().position(|entry| entry.id == id)
                            {
                                app.map_editor.catalog_page = index / 64;
                            }
                        }
                        return update(app, Action::Select(id));
                    }
                }
                Err(e) => {
                    log::error!("[map] {e}");
                    app.map_editor.status = e;
                }
            }
        }
        Action::TileIndex(v) => app.map_editor.tile_index = v,
        Action::MosaicTile(source, index) => {
            if app.map_editor.key != app.current_key()
                || !app
                    .map_editor
                    .mosaic
                    .as_ref()
                    .is_some_and(|m| Arc::ptr_eq(m, &source))
            {
                log::info!("[map] 已忽略过期拼图选择");
                return Task::none();
            }
            app.map_editor.tile_index = index.to_string();
        }
        Action::Columns(v) => {
            app.map_editor.cancel();
            app.map_editor.columns = v;
            app.map_editor.clear_mosaic();
        }
        Action::Rows(v) => {
            app.map_editor.cancel();
            app.map_editor.rows = v;
            app.map_editor.clear_mosaic();
        }
        Action::Stretch(v) => {
            app.map_editor.cancel();
            app.map_editor.stretch = v;
            app.map_editor.clear_mosaic();
        }
        Action::Path(v) => {
            app.map_editor.cancel();
            app.map_editor.image_path = v;
            app.map_editor.clear_mosaic();
        }
        Action::Cancel => {
            app.map_editor.cancel();
            app.map_editor.thumbnails.cancel();
            app.map_editor.status = "地图后台任务已取消".into();
        }
        Action::Scan => {
            app.map_editor.cancel();
            let Some(key) = app.current_key() else {
                app.map_editor.status = "请先选择世界".into();
                return Task::none();
            };
            let root = app.worlds[key.0].root.clone();
            app.map_editor.world_root = Some(root.clone());
            app.map_editor.cancel = Arc::new(AtomicBool::new(false));
            let cancel = Arc::clone(&app.map_editor.cancel);
            let epoch = app.map_editor.epoch;
            app.map_editor.key = Some(key);
            app.map_editor.scanned_for = Some(key);
            app.map_editor.version = app.worlds[key.0].data_version;
            app.map_editor.busy = true;
            app.map_editor.status = "正在后台扫描地图…".into();
            return work(
                move || map_catalog::scan(&root, || cancel.load(Ordering::Acquire)),
                move |r| Action::Scanned(epoch, key, r),
            );
        }
        Action::Scanned(epoch, key, result) => {
            if app.current_key() != Some(key) || app.map_editor.epoch != epoch {
                return Task::none();
            }
            app.map_editor.busy = false;
            match result {
                Ok(c) => {
                    app.map_editor.catalog = Some(c);
                    app.map_editor.refresh_catalog();
                    app.map_editor.thumbnails.invalidate();
                    app.map_editor.status = format!(
                        "当前修订有 {} 张地图",
                        app.map_editor
                            .visible_catalog
                            .as_ref()
                            .expect("刚安装地图清单")
                            .entries
                            .len()
                    );
                    if app.map_editor.selected.is_none() {
                        if let Some(id) = app.map_editor.first_map() {
                            return update(app, Action::Select(id));
                        }
                    }
                }
                Err(e) => {
                    log::error!("[map] {e}");
                    app.map_editor.status = e;
                }
            }
        }
        Action::Select(id) => {
            app.map_editor.cancel();
            let Some(key) = app.current_key() else {
                return Task::none();
            };
            if app.map_editor.key != Some(key) {
                app.map_editor.status = "地图列表属于其他世界或维度，请重新扫描".into();
                return Task::none();
            }
            let version = app.worlds[key.0].data_version;
            if !matches!(version, Some(1343 | 3465 | 3955)) {
                app.map_editor.status = "此地图版本的色表尚未确认".into();
                return Task::none();
            }
            let Some(entry) = app
                .map_editor
                .visible_catalog
                .as_ref()
                .and_then(|c| c.entries.iter().find(|e| e.id == id))
                .cloned()
            else {
                app.map_editor.status = "地图清单已过期".into();
                return Task::none();
            };
            let root = app.worlds[key.0].root.clone();
            let view = app.map_editor.source_view(&root);
            let epoch = app.map_editor.epoch;
            app.map_editor.selected = None;
            app.map_editor.base_preview = None;
            app.map_editor.busy = true;
            return work(
                move || {
                    let image = match &view {
                        Some(view) => view.read(&root, id, version)?,
                        None => map_data::read(&root, id, version)?,
                    };
                    if image.sha256 != entry.sha256 {
                        return Err("地图文件已变化，请重新扫描".into());
                    }
                    let base =
                        image::Handle::from_rgba(image.width, image.height, image.rgba.clone());
                    Ok((Arc::new(image), base))
                },
                move |r| Action::Selected(epoch, key, r),
            );
        }
        Action::Selected(epoch, key, result) => {
            if app.current_key() != Some(key) || app.map_editor.epoch != epoch {
                return Task::none();
            }
            app.map_editor.busy = false;
            match result {
                Ok((v, base)) => {
                    app.map_editor.status = format!(
                        "已读取地图 #{} · {} 个保存标记{}",
                        v.id,
                        v.decorations.len(),
                        if app.texture_bank.is_none() {
                            "；尚未加载客户端资源，当前显示底图"
                        } else {
                            ""
                        }
                    );
                    app.map_editor.selected = Some(v);
                    app.map_editor.base_preview = Some(base);
                    app.map_editor.preview = None;
                    app.map_editor.preview_source = None;
                }
                Err(e) => {
                    log::error!("[map] {e}");
                    app.map_editor.status = e;
                }
            }
        }
        Action::Preview(nonce, key, result) => {
            if app.current_key() == Some(key)
                && app.map_editor.preview_nonce == nonce
                && !app.map_editor.preview_cancel.load(Ordering::Acquire)
            {
                if let Err(error) = &result {
                    log::error!("[map] {error}");
                }
                app.map_editor.preview = Some(result);
            }
        }
        Action::Convert => {
            app.map_editor.cancel();
            app.map_editor.clear_mosaic();
            let Some(key) = app.current_key() else {
                app.map_editor.status = "请先选择世界".into();
                return Task::none();
            };
            app.map_editor.key = Some(key);
            app.map_editor.world_root = Some(app.worlds[key.0].root.clone());
            let path = PathBuf::from(app.map_editor.image_path.trim());
            let version = app.worlds[key.0].data_version;
            if !matches!(version, Some(1343 | 3465 | 3955)) {
                app.map_editor.status = "此版本的地图色表尚未确认".into();
                return Task::none();
            }
            let columns = app.map_editor.columns.clone();
            let rows = app.map_editor.rows.clone();
            let stretch = app.map_editor.stretch;
            let epoch = app.map_editor.epoch;
            app.map_editor.cancel = Arc::new(AtomicBool::new(false));
            let cancel = Arc::clone(&app.map_editor.cancel);
            app.map_editor.busy = true;
            app.map_editor.status = "正在后台解码图片并计算地图颜色…".into();
            return work(
                move || {
                    let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                    if meta.file_type().is_symlink()
                        || !meta.is_file()
                        || meta.len() > 64 * 1024 * 1024
                    {
                        return Err("图片必须是 64 MiB 内的普通图片文件".into());
                    }
                    use sha2::{Digest, Sha256};
                    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                    let source_sha = format!("{:x}", Sha256::digest(&bytes));
                    let probe = ::image::ImageReader::new(std::io::Cursor::new(&bytes))
                        .with_guessed_format()
                        .map_err(|e| e.to_string())?;
                    if !matches!(
                        probe.format(),
                        Some(
                            ::image::ImageFormat::Png
                                | ::image::ImageFormat::Jpeg
                                | ::image::ImageFormat::Gif
                                | ::image::ImageFormat::Bmp
                        )
                    ) {
                        return Err("仅支持 PNG、JPEG、GIF、BMP 图片".into());
                    }
                    let (width, height) = probe.into_dimensions().map_err(|e| e.to_string())?;
                    let pixels = (width as usize)
                        .checked_mul(height as usize)
                        .ok_or("图片尺寸溢出")?;
                    let tiles = ((width as usize + 127) / 128)
                        .checked_mul((height as usize + 127) / 128)
                        .ok_or("拼图数量溢出")?;
                    let total = pixels
                        .checked_mul(8)
                        .and_then(|v| v.checked_add(tiles.checked_mul(16384)?))
                        .and_then(|v| v.checked_add(bytes.len()))
                        .and_then(|v| v.checked_add(mosaic_view::MEMORY))
                        .ok_or("图片预算溢出")?;
                    if total > 256 * 1024 * 1024 {
                        return Err("图片、解码和拼图超过 256 MiB 内存预算".into());
                    }
                    let mut reader = ::image::ImageReader::new(std::io::Cursor::new(&bytes))
                        .with_guessed_format()
                        .map_err(|e| e.to_string())?;
                    let mut limits = ::image::Limits::default();
                    limits.max_alloc = Some(128 * 1024 * 1024);
                    reader.limits(limits);
                    let decoded = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
                    if cancel.load(Ordering::Acquire) {
                        return Err("图片转换已取消".into());
                    }
                    let requested = if columns.trim().is_empty() && rows.trim().is_empty() {
                        None
                    } else {
                        let columns = columns.trim().parse::<u32>().map_err(|_| "拼图列数无效")?;
                        let rows = rows.trim().parse::<u32>().map_err(|_| "拼图行数无效")?;
                        if columns == 0 || rows == 0 {
                            return Err("拼图列数和行数必须大于零".into());
                        }
                        Some((
                            columns.checked_mul(128).ok_or("拼图宽度溢出")?,
                            rows.checked_mul(128).ok_or("拼图高度溢出")?,
                        ))
                    };
                    let original_size = [decoded.width(), decoded.height()];
                    let (mosaic, preview) = if let Some((w, h)) = requested {
                        let target = (w as usize)
                            .checked_mul(h as usize)
                            .and_then(|n| n.checked_mul(4))
                            .ok_or("目标图片尺寸溢出")?;
                        let total = bytes
                            .len()
                            .checked_add(
                                decoded
                                    .as_raw()
                                    .len()
                                    .checked_mul(2)
                                    .ok_or("图片预算溢出")?,
                            )
                            .and_then(|n| n.checked_add(target.checked_mul(2)?))
                            .and_then(|n| n.checked_add(target / 4))
                            .and_then(|n| n.checked_add(mosaic_view::MEMORY))
                            .ok_or("图片预算溢出")?;
                        if total > 256 * 1024 * 1024 {
                            return Err("缩放图片与拼图超过 256 MiB 内存预算".into());
                        }
                        let resized = map_edit::resize_rgba(
                            decoded.width(),
                            decoded.height(),
                            decoded.as_raw(),
                            w,
                            h,
                            stretch,
                            256 * 1024 * 1024,
                            || cancel.load(Ordering::Acquire),
                        )?;
                        let mosaic = map_edit::quantize_mosaic(
                            version,
                            w,
                            h,
                            &resized,
                            256 * 1024 * 1024,
                            || cancel.load(Ordering::Acquire),
                        )?;
                        let preview =
                            mosaic_view::Preview::new(&mosaic, &resized, original_size, &|| {
                                cancel.load(Ordering::Acquire)
                            })?;
                        (mosaic, preview)
                    } else {
                        let mosaic = map_edit::quantize_mosaic(
                            version,
                            decoded.width(),
                            decoded.height(),
                            decoded.as_raw(),
                            256 * 1024 * 1024,
                            || cancel.load(Ordering::Acquire),
                        )?;
                        let preview = mosaic_view::Preview::new(
                            &mosaic,
                            decoded.as_raw(),
                            original_size,
                            &|| cancel.load(Ordering::Acquire),
                        )?;
                        (mosaic, preview)
                    };
                    Ok((Arc::new(mosaic), source_sha, preview))
                },
                move |r| Action::Converted(epoch, key, r),
            );
        }
        Action::Converted(epoch, key, result) => {
            if app.current_key() != Some(key) || app.map_editor.epoch != epoch {
                return Task::none();
            }
            app.map_editor.busy = false;
            match result {
                Ok((m, sha, preview)) => {
                    app.map_editor.status = "拼图已转换；尚未修改世界".into();
                    app.map_editor.source_sha = Some(sha);
                    app.map_editor.mosaic = Some(m);
                    app.map_editor.mosaic_preview = Some(preview);
                    app.map_editor.tile_index = "0".into();
                }
                Err(e) => {
                    log::error!("[map] {e}");
                    app.map_editor.status = e;
                }
            }
        }
    }
    Task::none()
}

#[test]
fn background_behavior() {
    preview_handoff();
    use crate::{
        metadata_fixture::Fixture,
        metadata_tests::{self, background, deliver},
    };
    let a = Fixture::new(3955);
    let b = Fixture::new(3955);
    let mut app = metadata_tests::app(&a);
    app.worlds.push(b.world.clone());
    fn import(app: &mut App, path: &std::path::Path) {
        let _ = background(app, Message::MapEditor(Action::Scan));
        let _ = update(app, Action::Path(path.to_string_lossy().into_owned()));
        let _ = background(app, Message::MapEditor(Action::Convert));
        let _ = background(app, Message::MapEditor(Action::PrepareImport));
    }
    import(&mut app, &a.image("first.png"));
    let _ = background(&mut app, Message::MapEditor(Action::PrepareImport));
    assert_eq!(app.map_editor.history(&a.root).cursor, 2);
    let _ = crate::update(
        &mut app,
        Message::Workspace(crate::workspace_tabs::Action::Close(
            crate::workspace_tabs::CloseTarget::World(0),
        )),
    );
    assert_eq!(
        app.workspace.closing,
        Some(crate::workspace_tabs::CloseTarget::World(0))
    );
    let _ = crate::update(
        &mut app,
        Message::Workspace(crate::workspace_tabs::Action::CancelClose),
    );
    assert_eq!(app.map_editor.history(&a.root).cursor, 2);
    let ids = |panel: &Panel, root: &std::path::Path| {
        panel
            .source_view(root)
            .map(|view| view.catalog(&map_catalog::Catalog::default()))
            .map_or_else(Vec::new, |catalog| {
                catalog.entries.into_iter().map(|entry| entry.id).collect()
            })
    };
    assert_eq!(ids(&app.map_editor, &a.root), vec![0, 1]);
    let _ = update(&mut app, Action::Undo);
    let _ = update(&mut app, Action::Undo);
    assert!(!app.map_editor.has_candidates(&a.root));
    let _ = update(&mut app, Action::Redo);
    let _ = update(&mut app, Action::Redo);
    assert_eq!(ids(&app.map_editor, &a.root), vec![0, 1]);
    let _ = crate::update(&mut app, Message::Select(1));
    import(&mut app, &b.image("second.png"));
    assert_eq!(ids(&app.map_editor, &b.root), vec![0]);
    assert_eq!(ids(&app.map_editor, &a.root), vec![0, 1]);
    let _ = crate::update(&mut app, Message::Select(0));
    app.nbt_editable = true;
    let _ = background(&mut app, Message::MixedPrepare);
    assert!(a.root.join(".mcedit2-mixed.pending.json").is_file());
    let task = background(&mut app, Message::MixedFinish);
    let _ = deliver(&mut app, task);
    assert!(a.root.join("data/map_0.dat").is_file());
    assert!(a.root.join("data/map_1.dat").is_file());
    assert!(!b.root.join("data/map_0.dat").exists());
    assert!(!app.map_editor.has_candidates(&a.root));
    assert_eq!(ids(&app.map_editor, &b.root), vec![0]);
    let _ = crate::update(&mut app, Message::Select(1));
    app.nbt_editable = true;
    let _ = background(&mut app, Message::MixedPrepare);
    let task = background(&mut app, Message::MixedFinish);
    let _ = deliver(&mut app, task);
    assert!(b.root.join("data/map_0.dat").is_file());
    assert!(app
        .map_editor
        .sessions
        .values()
        .all(|session| !session.has_changes()));
}

#[cfg(test)]
fn preview_handoff() {
    use crate::{
        map_fixture as f,
        metadata_tests::{app, deliver},
        picking_integration_tests::task_output,
    };
    let fixture = crate::metadata_fixture::Fixture::new(3955);
    f::write(&fixture.root, 3955, f::data(3955));
    let assets = crate::entity_geometry::map_tests::assets(3955);
    let mut app = app(&fixture);
    app.map_editor.selected = Some(Arc::new(
        map_data::read(&fixture.root, 7, Some(3955)).unwrap(),
    ));
    app.texture_bank = Some(assets.open());
    let old = maintain(&mut app);
    app.texture_bank = Some(assets.open());
    let new = maintain(&mut app);
    let _ = deliver(&mut app, old);
    assert!(app.map_editor.preview.is_none());
    let _ = deliver(&mut app, new);
    assert!(matches!(app.map_editor.preview, Some(Ok(_))));
    assert!(task_output(maintain(&mut app)).is_none());
    let identity = app.map_editor.preview_source.clone();
    let _ = app.map_editor.view(&app.drag_drop);
    assert_eq!(app.map_editor.preview_source, identity);
    app.texture_bank = Some(assets.open());
    let cancelled = maintain(&mut app);
    app.map_editor.cancel();
    let _ = deliver(&mut app, cancelled);
    assert!(matches!(&app.map_editor.preview,Some(Err(e)) if e.contains("取消")));
    assert!(task_output(maintain(&mut app)).is_none());
    app.map_editor.invalidate();
    assert!(task_output(maintain(&mut app)).is_none());
}
