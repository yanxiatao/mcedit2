//! 原版独立小地图：只读区块图像、当前视图范围及单一桌面设置队列。
use crate::{
    view_navigation::{QuadPane, Target},
    App, Message,
};
use iced::{
    mouse,
    widget::{
        button,
        canvas::{self, Frame, Path, Stroke},
        column, container, row, text,
    },
    Color, Element, Length, Point, Rectangle, Size, Task,
};
use mcedit2_world::{
    edit_sources::{SourceChanges, SourceRevision},
    editing::Session,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
#[path = "minimap_view.rs"]
mod observation;
#[path = "minimap_tiles.rs"]
mod tiles;
pub(crate) use observation::{observe, Capture};
type Key = (usize, usize);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub visible: bool,
    scale: f32,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            visible: true,
            scale: 1.,
        }
    }
}
impl Saved {
    pub fn validate(&self) -> Result<(), String> {
        if !self.scale.is_finite() || !(1. ..=16.).contains(&self.scale) {
            return Err("小地图比例必须在每像素 1..16 格之间".into());
        }
        Ok(())
    }
}
#[derive(Default)]
struct Ready {
    nonce: u64,
    result: Option<Result<Option<tiles::Output>, String>>,
}
pub(crate) struct State {
    pub saved: Saved,
    views: Arc<Mutex<observation::Views>>,
    ready: Arc<Mutex<Ready>>,
    key: Option<Key>,
    preferred: Option<Target>,
    current: Option<observation::Observation>,
    source: Option<SourceRevision>,
    session: Option<Arc<Session>>,
    style: Option<(
        crate::source_visibility::Identity,
        crate::gpu_mesh::Quality,
        crate::lighting::Key,
    )>,
    grid: Option<tiles::Grid>,
    plan: Vec<(i32, i32)>,
    tiles: Arc<tiles::Tiles>,
    image: Option<tiles::Image>,
    cancelled: Arc<AtomicBool>,
    pending: Option<crate::render_work::Task>,
    busy: bool,
    error: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self::restore(Saved::default())
    }
}
impl State {
    pub(crate) fn set_visible(&mut self, visible: bool) {
        self.saved.visible = visible;
        if !visible {
            self.cancel();
        }
    }
    pub fn restore(saved: Saved) -> Self {
        Self {
            saved,
            views: Arc::new(Mutex::new(observation::Views::default())),
            ready: Arc::new(Mutex::new(Ready::default())),
            key: None,
            preferred: None,
            current: None,
            source: None,
            session: None,
            style: None,
            grid: None,
            plan: Vec::new(),
            tiles: Arc::new(BTreeMap::new()),
            image: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            pending: None,
            busy: false,
            error: None,
        }
    }
    pub fn capture(&self, key: Option<Key>, target: Target) -> Option<Capture> {
        if !self.saved.visible {
            return None;
        }
        Some(Capture::new(key?, target, Arc::clone(&self.views)))
    }
    fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.pending = None;
        self.busy = false;
        let mut ready = self.ready.lock().expect("小地图结果锁被毒化");
        ready.nonce = ready.nonce.checked_add(1).expect("小地图任务代次溢出");
        ready.result = None;
    }
    fn refresh(&mut self, app: &mut App) -> Result<(), String> {
        let Some(key) = app
            .current_key()
            .filter(|_| self.saved.visible && available(app))
        else {
            if self.busy {
                self.cancel();
            }
            self.current = None;
            if app.current_key().is_none() {
                self.key = None;
                self.source = None;
                self.session = None;
                self.grid = None;
                self.plan.clear();
                self.tiles = Arc::new(BTreeMap::new());
                self.image = None;
                self.error = None;
            }
            return Ok(());
        };
        if self.key != Some(key) {
            self.cancel();
            self.key = Some(key);
            self.current = None;
            self.preferred = None;
            self.source = None;
            self.session = None;
            self.grid = None;
            self.plan.clear();
            self.tiles = Arc::new(BTreeMap::new());
            self.image = None;
            self.error = None;
        }
        let active_target = app.active_view();
        let targets: &[Target] =
            if app.show_advanced && app.advanced_section == crate::AdvancedSection::Maps {
                &[Target::Horizontal, Target::Vertical]
            } else if app.show_advanced && app.advanced_section == crate::AdvancedSection::Quad {
                &[
                    Target::Quad(QuadPane::Horizontal),
                    Target::Quad(QuadPane::Front),
                    Target::Quad(QuadPane::Side),
                    Target::Quad(QuadPane::Isometric),
                ]
            } else {
                std::slice::from_ref(&active_target)
            };
        let hover = crate::navigation_bindings::hovered_target(app)
            .filter(|target| targets.contains(target));
        let (preferred, current, size) = {
            let views = self.views.lock().expect("小地图观察状态锁被毒化");
            let captured = |target: Target| {
                views.observations[observation::slot(target)]
                    .as_ref()
                    .is_some_and(|(source, _)| *source == key)
            };
            let preferred = hover
                .filter(|target| captured(*target))
                .or(self
                    .preferred
                    .filter(|target| targets.contains(target) && captured(*target)))
                .or_else(|| targets.iter().copied().find(|target| captured(*target)))
                .unwrap_or(targets[0]);
            let current = views.observations[observation::slot(preferred)]
                .as_ref()
                .filter(|(source, _)| *source == key)
                .map(|(_, observation)| observation.clone())
                .transpose()?;
            (
                preferred,
                current.filter(|view| view.key == key),
                views.size,
            )
        };
        self.preferred = Some(preferred);
        self.current = current;
        let (Some(current), Some(size)) = (self.current.as_ref(), size) else {
            if self.busy {
                self.cancel();
            }
            return Ok(());
        };
        let grid = tiles::Grid::new(current.center, size, self.saved.scale)?;
        let mut changed = false;
        if self.grid != Some(grid) {
            self.cancel();
            let old_span = self.grid.map(|grid| grid.span);
            Arc::make_mut(&mut self.tiles)
                .retain(|&chunk, _| old_span == Some(grid.span) && grid.contains(chunk));
            self.plan = grid.chunks(self.current.as_ref().expect("观察仍存在").center);
            self.grid = Some(grid);
            self.error = None;
            changed = true;
        }
        if self.error.is_some() && self.session.is_none() {
            return Ok(());
        }
        let session = app.display_session(key)?;
        let revision = session.display_source();
        let changes = self
            .source
            .as_ref()
            .map_or(SourceChanges::NewSource, |before| {
                revision.changes_since(before)
            });
        if changes != SourceChanges::Unchanged {
            self.session = Some(Arc::new(session.clone()));
            self.source = Some(revision);
            match changes {
                SourceChanges::NewSource => {
                    self.cancel();
                    self.tiles = Arc::new(BTreeMap::new());
                    self.image = None;
                    changed = true;
                }
                SourceChanges::Chunks(chunks) => {
                    let chunks = chunks
                        .iter()
                        .flat_map(|&(cx, cz)| {
                            (-1..=1).flat_map(move |dx| {
                                (-1..=1).filter_map(move |dz| {
                                    Some((cx.checked_add(dx)?, cz.checked_add(dz)?))
                                })
                            })
                        })
                        .collect::<std::collections::BTreeSet<_>>();
                    if chunks.iter().any(|chunk| grid.contains(*chunk)) {
                        self.cancel();
                        Arc::make_mut(&mut self.tiles).retain(|chunk, _| !chunks.contains(chunk));
                        self.image = None;
                        changed = true;
                    }
                }
                SourceChanges::Unchanged => unreachable!("来源变化分支不会接收未修改来源"),
            }
            self.error = None;
        }
        let assets = app.scene_texture_bank().cloned();
        let lighting = crate::lighting::Table::unlit();
        let show_textures = app.scene_show_textures();
        let block_layers = app.scene_block_layers();
        let style = (
            crate::source_visibility::identity(assets.as_ref(), show_textures, block_layers),
            app.gpu_quality,
            lighting.key(),
        );
        if self.style.as_ref() != Some(&style) {
            self.cancel();
            self.style = Some(style);
            self.image = None;
            self.error = None;
            changed = true;
        }
        if let Some(result) = self.ready.lock().expect("小地图结果锁被毒化").result.take()
        {
            self.busy = false;
            self.pending = None;
            match result? {
                Some(output) => {
                    self.tiles = output.tiles;
                    self.image = Some(output.image);
                }
                None => (),
            }
        }
        if self.error.is_some() {
            return Ok(());
        }
        if self.busy {
            if let Some(pending) = &mut self.pending {
                pending.submit()?;
            }
            return Ok(());
        }
        let chunks = self
            .plan
            .iter()
            .filter(|chunk| !self.tiles.contains_key(*chunk))
            .take(mcedit2_world::far_view::batch_size_for(grid.span))
            .copied()
            .collect::<Vec<_>>();
        let expired = self
            .image
            .as_ref()
            .and_then(|image| image.atlas.as_ref())
            .is_some_and(|atlas| !atlas.animation_is_current());
        if chunks.is_empty()
            && !changed
            && self.image.as_ref().is_some_and(|image| image.grid == grid)
            && !expired
        {
            return Ok(());
        }
        let source = Arc::clone(self.session.as_ref().expect("小地图读源已捕获会话"));
        let before = Arc::clone(&self.tiles);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancelled = Arc::clone(&cancelled);
        let ready = Arc::clone(&self.ready);
        let nonce = ready.lock().expect("小地图结果锁被毒化").nonce;
        let quality = app.gpu_quality;
        let token = Arc::clone(&cancelled);
        self.pending = Some(crate::render_work::spawn(
            "小地图",
            cancelled,
            move || {
                let result = tiles::build(
                    source,
                    before,
                    chunks,
                    grid,
                    assets,
                    show_textures,
                    block_layers,
                    quality,
                    lighting,
                    &token,
                );
                if let Err(error) = &result {
                    log::error!("[minimap] {error}");
                }
                let mut ready = ready.lock().expect("小地图结果锁被毒化");
                if ready.nonce == nonce {
                    ready.result = Some(result);
                }
            },
        )?);
        self.busy = true;
        Ok(())
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
fn available(app: &App) -> bool {
    !app.desktop.show_recent
        && (app
            .floating_panels
            .contains(crate::floating_panels::Panel::Minimap)
            || !app.show_advanced
            || matches!(
                app.advanced_section,
                crate::AdvancedSection::Viewer
                    | crate::AdvancedSection::Maps
                    | crate::AdvancedSection::Quad
            ))
}
pub(crate) fn maintain(app: &mut App) {
    let mut state = std::mem::take(&mut app.minimap);
    if let Err(error) = state.refresh(app) {
        if state.error.as_ref() != Some(&error) {
            log::error!("[minimap] {error}");
        }
        state.cancel();
        state.image = None;
        state.error = Some(error);
    }
    app.minimap = state;
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Toggle,
    Zoom(i32),
    Refresh,
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Toggle => {
            app.minimap.set_visible(!app.minimap.saved.visible);
        }
        Action::Zoom(steps) => {
            app.minimap.saved.scale = (app.minimap.saved.scale * 2_f32.powi(-steps)).clamp(1., 16.);
        }
        Action::Refresh => {
            app.minimap.cancel();
            app.minimap.source = None;
            app.minimap.error = None;
        }
    }
    Task::none()
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    if app
        .floating_panels
        .contains(crate::floating_panels::Panel::Minimap)
    {
        return column![].into();
    }
    panel(app, true)
}
pub(crate) fn floating_view(app: &App) -> Element<'_, Message> {
    panel(app, false)
}
/// 停靠区使用紧凑预览，把空间留给编辑参数；独立窗口保留完整图例。
fn panel(app: &App, compact: bool) -> Element<'_, Message> {
    let state = &app.minimap;
    let l = app.language;
    if !available(app) {
        return column![].into();
    }
    let title = if compact {
        l.pick(
            if state.saved.visible {
                "小地图 ▾"
            } else {
                "小地图 ▸"
            },
            "Minimap",
        )
    } else if state.saved.visible {
        l.pick("收起小地图", "Hide minimap")
    } else {
        l.pick("显示小地图", "Show minimap")
    };
    let legend = l.pick(
        "N 北 · E 东 · 紫：缺区块 · 黄纹：缺区段 · 方格：待读取",
        "N north · E east · purple: missing chunk · amber: missing section · checker: loading",
    );
    let mut header = row![crate::workspace_ui::hint(
        button(text(title).size(12))
            .style(button::text)
            .padding(4)
            .on_press(Message::Minimap(Action::Toggle)),
        text(format!(
            "{} {}\n{legend}",
            state.saved.scale,
            l.pick("格 / 像素", "blocks / pixel")
        )),
        iced::widget::tooltip::Position::Bottom
    )]
    .spacing(3);
    if compact && state.saved.visible {
        for (label, enabled, action, zh, en) in [
            (
                "−",
                state.saved.scale < 16.,
                Action::Zoom(-1),
                "缩小小地图",
                "Zoom minimap out",
            ),
            (
                "+",
                state.saved.scale > 1.,
                Action::Zoom(1),
                "放大小地图",
                "Zoom minimap in",
            ),
            ("↻", true, Action::Refresh, "刷新小地图", "Refresh minimap"),
        ] {
            header = header.push(crate::workspace_ui::hint(
                button(text(label).size(12))
                    .style(button::secondary)
                    .padding(4)
                    .on_press_maybe(enabled.then_some(Message::Minimap(action))),
                l.pick(zh, en),
                iced::widget::tooltip::Position::Bottom,
            ));
        }
    }
    let mut body = column![header].spacing(5);
    if state.saved.visible {
        if !compact {
            body = body.push(
                row![
                    button("−").on_press_maybe(
                        (state.saved.scale < 16.).then_some(Message::Minimap(Action::Zoom(-1)))
                    ),
                    text(format!(
                        "{} {}",
                        state.saved.scale,
                        l.pick("格 / 像素", "blocks / pixel")
                    )),
                    button("+").on_press_maybe(
                        (state.saved.scale > 1.).then_some(Message::Minimap(Action::Zoom(1)))
                    ),
                    button(l.pick("刷新", "Refresh")).on_press(Message::Minimap(Action::Refresh)),
                ]
                .spacing(4),
            );
        }
        if state.error.is_none() || state.image.is_some() {
            body = body.push(
                iced::widget::canvas(Map { state, language: l })
                    .width(Length::Fill)
                    .height(Length::Fixed(if compact { 104. } else { 192. })),
            );
        }
        if let Some(error) = &state.error {
            body = body.push(crate::workspace_ui::hint(
                text(l.pick(
                    "小地图读取失败 · 悬停查看详情",
                    "Minimap failed · hover for details",
                ))
                .size(12),
                text(error).size(12).width(Length::Fixed(360.)),
                iced::widget::tooltip::Position::Right,
            ));
        } else if state.current.is_none() {
            body = body.push(text(l.pick("等待观察画面…", "Waiting for the viewport…")).size(12));
        } else if state.busy || state.tiles.len() < state.plan.len() {
            body = body.push(
                text(format!(
                    "{} {}/{}",
                    l.pick("读取区块", "Reading chunks"),
                    state.tiles.len(),
                    state.plan.len()
                ))
                .size(12),
            );
        }
        if !compact && state.grid.is_some_and(|grid| grid.span > 1) {
            body = body.push(
                text(l.pick(
                    "远处高度采样，仅供观察",
                    "Distant height sampling; overview only",
                ))
                .size(11),
            );
        }
        if !compact && (!app.scene_show_textures() || app.scene_texture_bank().is_none()) {
            body = body.push(text(l.pick("调色板预览", "Palette preview")).size(11));
        }
        if !compact {
            body = body.push(text(legend).size(11));
        }
    }
    container(body)
        .max_width(256)
        .width(Length::Fill)
        .padding(6)
        .style(container::rounded_box)
        .into()
}
struct Map<'a> {
    state: &'a State,
    language: crate::language::Language,
}
impl canvas::Program<Message> for Map<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        theme: &iced::Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut views = self.state.views.lock().expect("小地图观察状态锁被毒化");
        let size = (bounds.width > 0. && bounds.height > 0.).then_some(bounds.size());
        let resized = views.size != size;
        views.size = size;
        drop(views);
        if resized {
            crate::render_wake::notify();
        }
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            theme.extended_palette().background.strong.color,
        );
        if let Some(current) = &self.state.current {
            let scale = f64::from(self.state.saved.scale);
            let project = |world: [f64; 3]| {
                Point::new(
                    (f64::from(bounds.width) * 0.5 + (world[0] - current.center[0]) / scale) as f32,
                    (f64::from(bounds.height) * 0.5 + (world[2] - current.center[2]) / scale)
                        as f32,
                )
            };
            frame.with_clip(Rectangle::with_size(bounds.size()), |frame| {
                if let Some(image) = &self.state.image {
                    frame.draw_image(
                        Rectangle::new(
                            project([
                                f64::from(image.grid.x) - 0.5,
                                0.,
                                f64::from(image.grid.z) - 0.5,
                            ]),
                            Size::new(
                                f32::from(image.grid.width) / self.state.saved.scale,
                                f32::from(image.grid.depth) / self.state.saved.scale,
                            ),
                        ),
                        iced::advanced::image::Image::new(image.handle.clone())
                            .filter_method(iced::advanced::image::FilterMethod::Nearest),
                    );
                }
                if !current.outline.is_empty() {
                    let path = Path::new(|builder| {
                        builder.move_to(project(current.outline[0]));
                        for &point in &current.outline[1..] {
                            builder.line_to(project(point));
                        }
                        if current.outline.len() > 2 {
                            builder.close();
                        }
                    });
                    frame.stroke(
                        &path,
                        Stroke::default()
                            .with_width(2.)
                            .with_color(Color::from_rgb(1., 0.9, 0.15)),
                    );
                }
                let center = Point::new(bounds.width * 0.5, bounds.height * 0.5);
                frame.fill(&Path::circle(center, 3.), Color::WHITE);
                if let Some(forward) = current.forward {
                    let length = (forward[0] * forward[0] + forward[2] * forward[2]).sqrt();
                    if length > 0. {
                        frame.stroke(
                            &Path::line(
                                center,
                                Point::new(
                                    center.x + (forward[0] / length * 14.) as f32,
                                    center.y + (forward[2] / length * 14.) as f32,
                                ),
                            ),
                            Stroke::default().with_width(2.).with_color(Color::WHITE),
                        );
                    }
                }
            });
        }
        frame.fill_text(canvas::Text {
            content: self.language.pick("N 北", "N").into(),
            position: Point::new(5., 4.),
            color: Color::WHITE,
            size: 12.into(),
            ..Default::default()
        });
        frame.fill_text(canvas::Text {
            content: "E".into(),
            position: Point::new(bounds.width - 16., bounds.height * 0.5),
            color: Color::WHITE,
            size: 12.into(),
            ..Default::default()
        });
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        _: &mut (),
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                        *y
                    }
                };
                (y != 0.).then(|| {
                    canvas::Action::publish(Message::Minimap(Action::Zoom(if y > 0. {
                        1
                    } else {
                        -1
                    })))
                    .and_capture()
                })
            }
            iced::Event::Window(iced::window::Event::RedrawRequested(_)) => {
                let atlas = self.state.image.as_ref()?.atlas.as_ref()?;
                let deadline = atlas.animation_deadline()?;
                if !atlas.animation_is_current() {
                    crate::render_wake::notify();
                }
                Some(canvas::Action::request_redraw_at(deadline))
            }
            _ => None,
        }
    }
}
