//! 当前显示资源的静态图集布局；总览与原像素裁片按需生成，不复制整张大页。
use crate::{
    textures::{AtlasPage, TextureAtlas, TextureBank},
    App, Message,
};
use iced::{
    widget::{
        button,
        canvas::{self, Frame, Path, Stroke},
        column, container, image, row, scrollable, text, text_input,
    },
    Color, Element, Length, Point, Rectangle, Size, Task,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};

const EDGE: u32 = 512;
const ROWS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Owner {
    World((usize, usize), std::path::PathBuf),
    Structure(u64),
}
#[derive(Default)]
pub(crate) struct State {
    open: bool,
    owner: Option<Owner>,
    bank: Weak<TextureBank>,
    epoch: u64,
    serial: u64,
    running: Option<u64>,
    pending: Option<Job>,
    cancel: Option<Arc<AtomicBool>>,
    page: usize,
    origin: [u32; 2],
    x: String,
    y: String,
    filter: String,
    matches: Vec<String>,
    list_page: usize,
    selected: Option<String>,
    zoom: Zoom,
    light: bool,
    preview: Option<Preview>,
    overview: Option<(usize, image::Handle)>,
    pixel: Option<([u32; 2], [u8; 4])>,
    error: Option<String>,
}
impl Drop for State {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Zoom {
    #[default]
    One,
    Two,
    Four,
    Eight,
}
impl Zoom {
    const ALL: [Self; 4] = [Self::One, Self::Two, Self::Four, Self::Eight];
    fn scale(self) -> f32 {
        match self {
            Self::One => 1.,
            Self::Two => 2.,
            Self::Four => 4.,
            Self::Eight => 8.,
        }
    }
}
impl std::fmt::Display for Zoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}×", self.scale())
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Preview {
    serial: u64,
    page: usize,
    size: [u32; 2],
    origin: [u32; 2],
    extent: [u32; 2],
    overview: image::Handle,
    detail: image::Handle,
}
struct Job {
    serial: u64,
    page_index: usize,
    page: Arc<AtlasPage>,
    overview: Option<image::Handle>,
    origin: [u32; 2],
    cancel: Arc<AtomicBool>,
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Toggle,
    Page(u64, usize),
    Filter(u64, String),
    ListPage(u64, usize),
    Select(u64, String),
    X(u64, String),
    Y(u64, String),
    Apply(u64),
    Overview(u64, u64, [u32; 2]),
    Pixel(u64, u64, [u32; 2]),
    Zoom(u64, Zoom),
    Background(u64),
    Copy(u64),
    Done(u64, Result<Preview, String>),
}
fn message(action: Action) -> Message {
    Message::AtlasInspector(action)
}
fn context(app: &App) -> Option<(Owner, Arc<TextureBank>)> {
    if let Some(id) = crate::structure_workspace::active(app) {
        let entry = &app.structures.files[&id];
        return Some((Owner::Structure(id), entry.definition_bank()?));
    }
    if app.desktop.show_recent {
        return None;
    }
    let key = app.current_key()?;
    Some((
        Owner::World(key, app.worlds[key.0].root.clone()),
        app.scene_texture_bank()?.clone(),
    ))
}
fn visible(app: &App) -> bool {
    if app
        .floating_panels
        .contains(crate::floating_panels::Panel::Atlas)
    {
        return true;
    }
    if app
        .workspace
        .layout
        .collapsed(crate::workspace_ui::layout::active(app))
    {
        return false;
    }
    if let Some(id) = crate::structure_workspace::active(app) {
        app.structures.files[&id].resources_visible()
    } else {
        !app.desktop.show_recent
            && app.current_key().is_some()
            && (!app.show_advanced || crate::workspace_ui::scene_workspace(app))
            && app.workspace_pane == crate::workspace_ui::Pane::Resources
    }
}
impl State {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }
    pub(crate) fn set_open(&mut self, open: bool) {
        self.open = open;
        if !open {
            self.clear_source();
        }
    }
    fn clear_source(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.serial = self.serial.wrapping_add(1);
        self.owner = None;
        self.bank = Weak::new();
        self.pending = None;
        self.preview = None;
        self.overview = None;
        self.pixel = None;
        self.error = None;
        self.matches.clear();
        self.selected = None;
        self.page = 0;
        self.list_page = 0;
        self.origin = [0, 0];
        self.x = "0".into();
        self.y = "0".into();
        // running 只在真实后台结果返回时释放，切源不额外启动第二个线程。
    }
    fn search(&mut self, atlas: &TextureAtlas) {
        let query = self.filter.trim().to_lowercase();
        self.matches = atlas
            .entries
            .keys()
            .filter(|id| id.to_lowercase().contains(&query))
            .cloned()
            .collect();
        self.list_page = 0;
    }
    fn queue(&mut self, atlas: &TextureAtlas) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        self.serial = self.serial.wrapping_add(1);
        self.preview = None;
        self.pixel = None;
        self.error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.pending = Some(Job {
            serial: self.serial,
            page_index: self.page,
            page: atlas.pages[self.page].clone(),
            overview: self
                .overview
                .as_ref()
                .filter(|(page, _)| *page == self.page)
                .map(|(_, image)| image.clone()),
            origin: self.origin,
            cancel: cancel.clone(),
        });
        self.cancel = Some(cancel);
        self.x = self.origin[0].to_string();
        self.y = self.origin[1].to_string();
    }
    fn report(&mut self, error: String) {
        log::error!("[atlas-view] {error}");
        self.error = Some(error);
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    let context = (app.atlas_inspector.open && visible(app))
        .then(|| context(app))
        .flatten();
    let state = &mut app.atlas_inspector;
    let Some((owner, bank)) = context else {
        if state.owner.is_some() {
            state.clear_source();
        }
        return Task::none();
    };
    if state.owner.as_ref() != Some(&owner) || !state.bank.ptr_eq(&Arc::downgrade(&bank)) {
        state.clear_source();
        state.owner = Some(owner);
        state.bank = Arc::downgrade(&bank);
        match bank.atlas() {
            Some(atlas) if !atlas.pages.is_empty() => {
                state.search(atlas);
                state.queue(atlas);
            }
            _ => state.report("当前资源没有已生成的图集页".into()),
        }
    }
    if state.running.is_some() {
        return Task::none();
    }
    let Some(job) = state.pending.take() else {
        return Task::none();
    };
    let serial = job.serial;
    state.running = Some(serial);
    Task::perform(
        async move {
            let (send, receive) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let result = prepare(job);
                if send.send(result).is_err() {
                    log::debug!("[atlas-view] 图集预览接收端已关闭");
                }
            });
            receive
                .await
                .map_err(|error| format!("图集预览任务未返回：{error}"))?
        },
        move |result| message(Action::Done(serial, result)),
    )
}
fn prepare(job: Job) -> Result<Preview, String> {
    let page = &job.page;
    let overview = if let Some(overview) = job.overview {
        overview
    } else {
        let largest = page.width.max(page.height);
        let overview_size = [
            (u64::from(page.width) * u64::from(EDGE) / u64::from(largest))
                .max(1)
                .min(u64::from(page.width)) as u32,
            (u64::from(page.height) * u64::from(EDGE) / u64::from(largest))
                .max(1)
                .min(u64::from(page.height)) as u32,
        ];
        let mut overview = Vec::with_capacity((overview_size[0] * overview_size[1] * 4) as usize);
        for y in 0..overview_size[1] {
            if job.cancel.load(Ordering::Acquire) {
                return Err("图集预览已取消".into());
            }
            let sy = u64::from(y) * u64::from(page.height) / u64::from(overview_size[1]);
            for x in 0..overview_size[0] {
                let sx = u64::from(x) * u64::from(page.width) / u64::from(overview_size[0]);
                let offset = ((sy * u64::from(page.width) + sx) * 4) as usize;
                overview.extend_from_slice(&page.rgba[offset..offset + 4]);
            }
        }
        image::Handle::from_rgba(overview_size[0], overview_size[1], overview)
    };
    let extent = [
        (page.width - job.origin[0]).min(EDGE),
        (page.height - job.origin[1]).min(EDGE),
    ];
    let mut detail = Vec::with_capacity((extent[0] * extent[1] * 4) as usize);
    for y in job.origin[1]..job.origin[1] + extent[1] {
        if job.cancel.load(Ordering::Acquire) {
            return Err("图集预览已取消".into());
        }
        let offset =
            ((u64::from(y) * u64::from(page.width) + u64::from(job.origin[0])) * 4) as usize;
        detail.extend_from_slice(&page.rgba[offset..offset + (extent[0] * 4) as usize]);
    }
    Ok(Preview {
        serial: job.serial,
        page: job.page_index,
        size: [page.width, page.height],
        origin: job.origin,
        extent,
        overview,
        detail: image::Handle::from_rgba(extent[0], extent[1], detail),
    })
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    if let Action::Done(serial, result) = action {
        let state = &mut app.atlas_inspector;
        if state.running == Some(serial) {
            state.running = None;
            if state.owner.is_some() && serial == state.serial && state.pending.is_none() {
                match result {
                    Ok(preview) => {
                        state.overview = Some((preview.page, preview.overview.clone()));
                        state.preview = Some(preview);
                        return crate::workspace_ui::scroll_start("mcedit2-atlas-detail");
                    }
                    Err(error) => state.report(error),
                }
            }
        }
        return Task::none();
    }
    if let Action::Toggle = action {
        app.atlas_inspector.set_open(!app.atlas_inspector.open);
        return Task::none();
    }
    let Some((owner, bank)) = context(app).filter(|_| visible(app)) else {
        return Task::none();
    };
    let state = &mut app.atlas_inspector;
    if state.owner.as_ref() != Some(&owner) || !state.bank.ptr_eq(&Arc::downgrade(&bank)) {
        return Task::none();
    }
    let epoch = match &action {
        Action::Page(epoch, _)
        | Action::Filter(epoch, _)
        | Action::ListPage(epoch, _)
        | Action::Select(epoch, _)
        | Action::X(epoch, _)
        | Action::Y(epoch, _)
        | Action::Apply(epoch)
        | Action::Overview(epoch, ..)
        | Action::Pixel(epoch, ..)
        | Action::Zoom(epoch, _)
        | Action::Background(epoch)
        | Action::Copy(epoch) => *epoch,
        Action::Toggle | Action::Done(..) => unreachable!(),
    };
    if !state.open || state.epoch != epoch {
        return Task::none();
    }
    let Some(atlas) = bank.atlas() else {
        return Task::none();
    };
    let copy = matches!(&action, Action::Copy(_));
    let result: Result<(), String> = (|| {
        match action {
            Action::Page(_, page) => {
                if page >= atlas.pages.len() {
                    return Err("图集页不存在".into());
                }
                state.page = page;
                state.origin = [0, 0];
                state.selected = None;
                state.queue(atlas);
            }
            Action::Filter(_, value) => {
                state.filter = value;
                state.search(atlas);
            }
            Action::ListPage(_, page) => {
                if page >= state.matches.len().div_ceil(ROWS).max(1) {
                    return Err("资源列表页已变化".into());
                }
                state.list_page = page;
            }
            Action::Select(_, id) => {
                let uv = atlas.entries.get(&id).ok_or("图集中没有所选资源")?;
                state.page = uv.page as usize;
                let page = &atlas.pages[state.page];
                state.origin = [
                    ((uv.start[0] * page.width as f32).floor() as u32)
                        .min(page.width.saturating_sub(EDGE)),
                    ((uv.start[1] * page.height as f32).floor() as u32)
                        .min(page.height.saturating_sub(EDGE)),
                ];
                state.selected = Some(id);
                state.queue(atlas);
            }
            Action::X(_, value) => state.x = value,
            Action::Y(_, value) => state.y = value,
            Action::Apply(_) => {
                let x = state
                    .x
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "图集 X 必须为非负整数")?;
                let y = state
                    .y
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "图集 Y 必须为非负整数")?;
                let page = &atlas.pages[state.page];
                if x >= page.width || y >= page.height {
                    return Err("局部查看起点不在当前图集页内".into());
                }
                state.origin = [x, y];
                state.queue(atlas);
            }
            Action::Overview(_, serial, point) => {
                if !state
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.serial == serial)
                {
                    return Ok(());
                }
                let page = &atlas.pages[state.page];
                state.origin = [
                    point[0]
                        .saturating_sub(EDGE / 2)
                        .min(page.width.saturating_sub(EDGE)),
                    point[1]
                        .saturating_sub(EDGE / 2)
                        .min(page.height.saturating_sub(EDGE)),
                ];
                state.queue(atlas);
            }
            Action::Pixel(_, serial, point) => {
                if !state
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.serial == serial)
                {
                    return Ok(());
                }
                let page = &atlas.pages[state.page];
                let at = ((u64::from(point[1]) * u64::from(page.width) + u64::from(point[0])) * 4)
                    as usize;
                state.pixel = Some((point, page.rgba[at..at + 4].try_into().unwrap()));
            }
            Action::Zoom(_, zoom) => state.zoom = zoom,
            Action::Background(_) => state.light = !state.light,
            Action::Copy(_) | Action::Toggle | Action::Done(..) => (),
        }
        Ok(())
    })();
    if let Err(error) = result {
        state.report(error);
    }
    if copy {
        if let Some(id) = &state.selected {
            return iced::clipboard::write(id.clone());
        }
    }
    Task::none()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Page(usize);
impl std::fmt::Display for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0 + 1)
    }
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    if app
        .floating_panels
        .contains(crate::floating_panels::Panel::Atlas)
    {
        return crate::floating_panels::placeholder(app, crate::floating_panels::Panel::Atlas);
    }
    floating_view(app)
}
pub(crate) fn floating_view(app: &App) -> Element<'_, Message> {
    let state = &app.atlas_inspector;
    let l = app.language;
    let mut body = column![button(l.pick(
        if state.open {
            "收起图集查看"
        } else {
            "查看资源图集…"
        },
        if state.open {
            "Hide atlas viewer"
        } else {
            "View texture atlas…"
        },
    ))
    .on_press(message(Action::Toggle))]
    .spacing(6)
    .width(Length::Fill);
    if !state.open {
        return body.into();
    }
    let Some((owner, bank)) = context(app) else {
        return body
            .push(text(l.pick("当前没有显示资源。", "No display resources loaded.")).size(12))
            .into();
    };
    if state.owner.as_ref() != Some(&owner) || !state.bank.ptr_eq(&Arc::downgrade(&bank)) {
        return body
            .push(text(l.pick("正在切换资源来源…", "Changing resource source…")).size(12))
            .into();
    }
    if let Some(error) = &state.error {
        body = body.push(text(error).size(12));
    }
    let Some(atlas) = bank.atlas().filter(|atlas| !atlas.pages.is_empty()) else {
        return body.into();
    };
    let epoch = state.epoch;
    let page = &atlas.pages[state.page];
    body = body.push(text(l.pick(
        "基础资源图集布局，动画显示初始帧。总览用于定位，局部查看保留原始像素。",
        "Base resource atlas layout; animations show their initial frame. Use the overview to locate original-pixel details.",
    )).size(12));
    body = body.push(
        row![
            text(l.pick("图集页", "Atlas page")).size(12),
            iced::widget::pick_list(
                (0..atlas.pages.len()).map(Page).collect::<Vec<_>>(),
                Some(Page(state.page)),
                move |Page(page)| message(Action::Page(epoch, page))
            ),
            text(format!(
                "{}×{} · {}",
                page.width,
                page.height,
                atlas.pages.len()
            ))
            .size(12),
        ]
        .spacing(5)
        .align_y(iced::Alignment::Center)
        .wrap(),
    );
    body = body.push(
        text_input(l.pick("筛选资源 ID", "Filter resource IDs"), &state.filter)
            .on_input(move |value| message(Action::Filter(epoch, value))),
    );
    let list_pages = state.matches.len().div_ceil(ROWS).max(1);
    body = body.push(
        row![
            button("‹").on_press_maybe(
                (state.list_page > 0)
                    .then(|| message(Action::ListPage(epoch, state.list_page - 1)))
            ),
            text(format!(
                "{} / {list_pages} · {}",
                state.list_page + 1,
                state.matches.len()
            ))
            .size(12),
            button("›").on_press_maybe(
                (state.list_page + 1 < list_pages)
                    .then(|| message(Action::ListPage(epoch, state.list_page + 1)))
            ),
        ]
        .spacing(5)
        .align_y(iced::Alignment::Center),
    );
    for id in state.matches.iter().skip(state.list_page * ROWS).take(ROWS) {
        body = body.push(
            button(text(id).size(12))
                .style(if state.selected.as_ref() == Some(id) {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(message(Action::Select(epoch, id.clone())))
                .width(Length::Fill),
        );
    }
    let selected = state.selected.as_ref().and_then(|id| atlas.entries.get(id));
    let mut selected_rect = None;
    if let (Some(id), Some(uv)) = (&state.selected, selected) {
        let source_page = &atlas.pages[uv.page as usize];
        let minimum = [
            (uv.start[0] * source_page.width as f32).floor() as u32,
            (uv.start[1] * source_page.height as f32).floor() as u32,
        ];
        let maximum = [
            (uv.end[0] * source_page.width as f32).floor() as u32 + 1,
            (uv.end[1] * source_page.height as f32).floor() as u32 + 1,
        ];
        if uv.page as usize == state.page {
            selected_rect = Some([minimum[0], minimum[1], maximum[0], maximum[1]]);
        }
        body = body.push(
            text(format!(
                "{id}\n{} · [{}, {}) × [{}, {})\nUV {:?} → {:?}",
                uv.page + 1,
                minimum[0],
                maximum[0],
                minimum[1],
                maximum[1],
                uv.start,
                uv.end
            ))
            .size(12),
        );
        body = body.push(
            button(l.pick("复制资源 ID", "Copy resource ID"))
                .on_press(message(Action::Copy(epoch))),
        );
    }
    body = body.push(
        row![
            text_input("X", &state.x)
                .width(90)
                .on_input(move |value| message(Action::X(epoch, value)))
                .on_submit(message(Action::Apply(epoch))),
            text_input("Y", &state.y)
                .width(90)
                .on_input(move |value| message(Action::Y(epoch, value)))
                .on_submit(message(Action::Apply(epoch))),
            button(l.pick("查看坐标", "Go to pixel")).on_press(message(Action::Apply(epoch))),
        ]
        .spacing(4)
        .wrap(),
    );
    body = body.push(
        row![
            iced::widget::pick_list(Zoom::ALL, Some(state.zoom), move |zoom| message(
                Action::Zoom(epoch, zoom)
            )),
            button(l.pick("切换明暗底色", "Toggle background"))
                .on_press(message(Action::Background(epoch))),
        ]
        .spacing(4)
        .wrap(),
    );
    if let Some(preview) = &state.preview {
        body = body.push(
            canvas::Canvas::new(Overview {
                epoch,
                preview,
                selected: selected_rect,
                light: state.light,
            })
            .width(Length::Fill)
            .height(Length::Fixed(220.)),
        );
        body = body.push(
            text(format!(
                "X {}..{} · Y {}..{}",
                preview.origin[0],
                preview.origin[0] + preview.extent[0] - 1,
                preview.origin[1],
                preview.origin[1] + preview.extent[1] - 1
            ))
            .size(12),
        );
        body = body.push(
            scrollable(
                canvas::Canvas::new(Detail {
                    epoch,
                    preview,
                    zoom: state.zoom.scale(),
                    light: state.light,
                })
                .width(Length::Fixed(preview.extent[0] as f32 * state.zoom.scale()))
                .height(Length::Fixed(preview.extent[1] as f32 * state.zoom.scale())),
            )
            .id("mcedit2-atlas-detail")
            .direction(iced::widget::scrollable::Direction::Both {
                vertical: iced::widget::scrollable::Scrollbar::new(),
                horizontal: iced::widget::scrollable::Scrollbar::new(),
            })
            .width(Length::Fill)
            .height(Length::Fixed(288.)),
        );
    } else if state.pending.is_some() || state.running.is_some() {
        body = body.push(text(l.pick("正在准备图集预览…", "Preparing atlas preview…")).size(12));
    }
    if let Some((point, rgba)) = state.pixel {
        body = body.push(text(format!("X {} · Y {} · RGBA {rgba:?}", point[0], point[1])).size(12));
    }
    container(body)
        .padding(6)
        .width(Length::Fill)
        .style(container::rounded_box)
        .into()
}

fn fit(bounds: Size, size: [u32; 2]) -> Rectangle {
    let scale = (bounds.width / size[0] as f32).min(bounds.height / size[1] as f32);
    let size = Size::new(size[0] as f32 * scale, size[1] as f32 * scale);
    Rectangle::new(
        Point::new(
            (bounds.width - size.width) * 0.5,
            (bounds.height - size.height) * 0.5,
        ),
        size,
    )
}
fn background(light: bool) -> Color {
    if light {
        Color::WHITE
    } else {
        Color::from_rgb(0.12, 0.12, 0.12)
    }
}
struct Overview<'a> {
    epoch: u64,
    preview: &'a Preview,
    selected: Option<[u32; 4]>,
    light: bool,
}
impl canvas::Program<Message> for Overview<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &iced::Theme,
        bounds: Rectangle,
        _: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let image_bounds = fit(bounds.size(), self.preview.size);
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), background(self.light));
        frame.draw_image(
            image_bounds,
            iced::advanced::image::Image::new(self.preview.overview.clone())
                .filter_method(iced::advanced::image::FilterMethod::Nearest),
        );
        let scale = [
            image_bounds.width / self.preview.size[0] as f32,
            image_bounds.height / self.preview.size[1] as f32,
        ];
        let project = |point: [u32; 2]| {
            Point::new(
                image_bounds.x + point[0] as f32 * scale[0],
                image_bounds.y + point[1] as f32 * scale[1],
            )
        };
        frame.stroke(
            &Path::rectangle(
                project(self.preview.origin),
                Size::new(
                    self.preview.extent[0] as f32 * scale[0],
                    self.preview.extent[1] as f32 * scale[1],
                ),
            ),
            Stroke::default()
                .with_width(2.)
                .with_color(Color::from_rgb(0., 1., 1.)),
        );
        if let Some([x, y, xmax, ymax]) = self.selected {
            frame.stroke(
                &Path::rectangle(
                    project([x, y]),
                    Size::new((xmax - x) as f32 * scale[0], (ymax - y) as f32 * scale[1]),
                ),
                Stroke::default()
                    .with_width(2.)
                    .with_color(Color::from_rgb(1., 0.6, 0.)),
            );
        }
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        _: &mut (),
        event: &iced::Event,
        bounds: Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if matches!(
            event,
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
        ) {
            let point = cursor.position_in(bounds)?;
            let image_bounds = fit(bounds.size(), self.preview.size);
            if !image_bounds.contains(point) {
                return None;
            }
            let source = [
                (((point.x - image_bounds.x) / image_bounds.width * self.preview.size[0] as f32)
                    .floor() as u32)
                    .min(self.preview.size[0] - 1),
                (((point.y - image_bounds.y) / image_bounds.height * self.preview.size[1] as f32)
                    .floor() as u32)
                    .min(self.preview.size[1] - 1),
            ];
            return Some(
                canvas::Action::publish(message(Action::Overview(
                    self.epoch,
                    self.preview.serial,
                    source,
                )))
                .and_capture(),
            );
        }
        None
    }
    fn mouse_interaction(
        &self,
        _: &(),
        bounds: Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> iced::mouse::Interaction {
        if cursor
            .position_in(bounds)
            .is_some_and(|point| fit(bounds.size(), self.preview.size).contains(point))
        {
            iced::mouse::Interaction::Pointer
        } else {
            iced::mouse::Interaction::default()
        }
    }
}
struct Detail<'a> {
    epoch: u64,
    preview: &'a Preview,
    zoom: f32,
    light: bool,
}
impl canvas::Program<Message> for Detail<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &iced::Theme,
        bounds: Rectangle,
        _: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), background(self.light));
        frame.draw_image(
            Rectangle::with_size(bounds.size()),
            iced::advanced::image::Image::new(self.preview.detail.clone())
                .filter_method(iced::advanced::image::FilterMethod::Nearest),
        );
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        _: &mut (),
        event: &iced::Event,
        bounds: Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if matches!(
            event,
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
        ) {
            let point = cursor.position_in(bounds)?;
            let source = [
                self.preview.origin[0]
                    + ((point.x / self.zoom).floor() as u32).min(self.preview.extent[0] - 1),
                self.preview.origin[1]
                    + ((point.y / self.zoom).floor() as u32).min(self.preview.extent[1] - 1),
            ];
            return Some(
                canvas::Action::publish(message(Action::Pixel(
                    self.epoch,
                    self.preview.serial,
                    source,
                )))
                .and_capture(),
            );
        }
        None
    }
}
