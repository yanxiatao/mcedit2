//! 相机工作平面：只提供工具几何落点，真实方块状态仍由编辑会话读取。
use crate::{
    brush_stroke::Source, isometric::Face, picking::Hit, spectator::PresentedView, App, Message,
};
use iced::{
    widget::{
        button,
        canvas::{Frame, Path, Stroke},
        checkbox, column, row, text, text_input,
    },
    Color, Element, Point, Task,
};
use mcedit2_world::editing::Bounds;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Saved {
    pub enabled: bool,
    pub height: i32,
}
impl Default for Saved {
    fn default() -> Self {
        Self {
            enabled: false,
            height: 64,
        }
    }
}
impl Saved {
    pub fn validate(self) -> Result<(), String> {
        Bounds::spatial((0, self.height, 0), (0, self.height, 0)).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn validate_source(
        self,
        world: &mcedit2_world::WorldSummary,
        dimension: &str,
    ) -> Result<(), String> {
        self.validate()?;
        if self.enabled {
            let geometry = world
                .world_data
                .geometry(dimension)
                .map_err(|e| e.to_string())?;
            if self.height < geometry.min_y || self.height > geometry.max_y() {
                return Err(format!(
                    "工作平面高度应在当前维度 {}..{} 内",
                    geometry.min_y,
                    geometry.max_y()
                ));
            }
        }
        Ok(())
    }
}
pub(crate) struct State {
    pub saved: Saved,
    draft: String,
    report: String,
}
impl Default for State {
    fn default() -> Self {
        Self::restore(Saved::default())
    }
}
impl State {
    pub fn restore(saved: Saved) -> Self {
        Self {
            saved,
            draft: saved.height.to_string(),
            report: String::new(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Plane {
    pub height: i32,
    pub epoch: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub position: [i32; 3],
    pub face: Face,
}
#[derive(Debug, Clone)]
pub(crate) enum Picked {
    Plane(Anchor),
    Surface(Option<Hit>),
}
pub(crate) fn config(app: &App) -> Option<Plane> {
    (app.work_plane.saved.enabled && crate::navigation_bindings::perspective_active(app)).then_some(
        Plane {
            height: app.work_plane.saved.height,
            epoch: app.input_epoch,
        },
    )
}
/// 只在工作平面确实位于最近可见表面之前时使用几何落点。
pub(crate) fn pick(
    frame: &PresentedView,
    point: Point,
    plane: Option<Plane>,
) -> Result<Picked, String> {
    let hit = frame.pick(point);
    let Some(plane) = plane else {
        return Ok(Picked::Surface(hit));
    };
    if !matches!(frame.camera, crate::world_camera::WorldCamera::Free(_)) {
        return Err("相机工作平面需要实际自由相机画面".into());
    }
    let Some(ray) = crate::picking::Ray::new(frame.camera, [0; 3], point, frame.viewport.logical)
    else {
        return Ok(Picked::Surface(hit));
    };
    let Some((world, depth, down)) = ray.at_height(f64::from(plane.height)) else {
        return Ok(Picked::Surface(hit));
    };
    if hit.as_ref().is_some_and(|hit| hit.depth <= depth) {
        return Ok(Picked::Surface(hit));
    }
    if !world.into_iter().all(f64::is_finite)
        || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&(world[0] + 0.5).floor())
        || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&(world[2] + 0.5).floor())
    {
        return Err("工作平面交点坐标溢出".into());
    }
    let position = [
        (world[0] + 0.5).floor() as i32,
        plane.height,
        (world[2] + 0.5).floor() as i32,
    ];
    Bounds::spatial(position.into(), position.into()).map_err(|e| e.to_string())?;
    Ok(Picked::Plane(Anchor {
        position,
        face: if down { Face::Top } else { Face::Bottom },
    }))
}
pub(crate) fn draw(frame: &mut Frame, shown: &PresentedView, plane: Plane, hover: Option<Anchor>) {
    let crate::world_camera::WorldCamera::Free(camera) = shown.camera else {
        return;
    };
    let forward = camera.basis().2;
    let x = (camera.position[0] + f64::from(forward[0]) * 40.).floor();
    let z = (camera.position[2] + f64::from(forward[2]) * 40.).floor();
    let y = f64::from(plane.height);
    let stroke = Stroke::default()
        .with_width(1.)
        .with_color(Color::from_rgba(1., 1., 1., 100. / 255.));
    for index in -32..32 {
        for (a, b) in [
            (
                [x + f64::from(index) - 0.5, y, z - 32.5],
                [x + f64::from(index) - 0.5, y, z + 31.5],
            ),
            (
                [x - 32.5, y, z + f64::from(index) - 0.5],
                [x + 31.5, y, z + f64::from(index) - 0.5],
            ),
        ] {
            if let Some((a, b)) = crate::scene_layers::segment(shown.camera, frame.size(), a, b) {
                frame.stroke(&Path::line(a, b), stroke);
            }
        }
    }
    if let Some(anchor) = hover {
        let [x, _, z] = anchor.position.map(f64::from);
        let points = [
            [x - 0.5, y, z - 0.5],
            [x + 0.5, y, z - 0.5],
            [x + 0.5, y, z + 0.5],
            [x - 0.5, y, z + 0.5],
        ];
        let stroke = Stroke::default()
            .with_width(2.)
            .with_color(Color::from_rgb(0.2, 1., 1.));
        for index in 0..4 {
            if let Some((a, b)) = crate::scene_layers::segment(
                shown.camera,
                frame.size(),
                points[index],
                points[(index + 1) % 4],
            ) {
                frame.stroke(&Path::line(a, b), stroke);
            }
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Intent {
    Point,
    CornerA,
    CornerB,
    Box([i32; 3]),
}
#[derive(Debug, Clone)]
pub(crate) struct Selection {
    pub key: (usize, usize),
    pub source: Source,
    pub plane: Plane,
    pub anchor: Anchor,
    pub intent: Intent,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    Toggle(bool),
    Height(String),
    Apply,
    Step(i32),
    Pick(Selection),
    Error(String),
}
fn apply(app: &mut App, saved: Saved) -> Result<(), String> {
    let (wi, di) = app.current_key().ok_or("请先打开存档")?;
    saved.validate_source(&app.worlds[wi], &app.worlds[wi].dimensions[di].name)?;
    crate::keyboard_bindings::interrupt_view_gesture(app);
    app.work_plane = State::restore(saved);
    Ok(())
}
fn choose(app: &mut App, selection: Selection) -> Result<Task<Message>, String> {
    if app.reviewing_block_preview() {
        return Err("请先确认或取消当前候选，再点选工作平面".into());
    }
    if app.current_key() != Some(selection.key) || config(app) != Some(selection.plane) {
        return Err("工作平面或来源已经改变，请重新点选".into());
    }
    selection.source.verify(app, selection.key)?;
    let [x, y, z] = selection.anchor.position;
    Bounds::spatial((x, y, z), (x, y, z)).map_err(|e| e.to_string())?;
    app.query_coordinates = format!("{x}, {y}, {z}");
    app.edit_area = format!("{x},{y},{z};{x},{y},{z}");
    app.clear_block_preview();
    app.be_prepared = None;
    app.save_preview = None;
    app.selected_block = None;
    app.selected_surface = None;
    app.selected_surface_face = None;
    match selection.intent {
        Intent::Point => {
            app.corner_a = None;
            app.corner_b = None;
        }
        Intent::CornerA => app.corner_a = Some((x, y, z)),
        Intent::CornerB => app.corner_b = Some((x, y, z)),
        Intent::Box(start) => {
            app.corner_a = Some(start.into());
            app.corner_b = Some((x, y, z));
        }
    }
    if app.corner_a.is_some() && app.corner_b.is_some() {
        app.use_3d_box();
    }
    app.surface_report = format!("工作平面坐标 ({x},{y},{z})；编辑仍需预览与暂存。");
    if app.object_inspector.picking_chunks {
        let changed = crate::object_inspector::update(
            app,
            crate::object_inspector::Action::ChunkCoordinates(format!(
                "{},{}",
                x.div_euclid(16),
                z.div_euclid(16)
            )),
        );
        let opened =
            crate::object_inspector::update(app, crate::object_inspector::Action::OpenChunk);
        return Ok(Task::batch([changed, opened]));
    }
    if matches!(selection.intent, Intent::Point) {
        let source = app
            .current_session(selection.key)?
            .visible_block((x, y, z))
            .map_err(|e| e.to_string())?;
        app.block_result = source.description();
        if let mcedit2_world::block_lookup::Lookup::Present(block) = source {
            app.selected_block = Some(crate::block_names::Selection {
                key: selection.key,
                position: (x, y, z),
                state: block.state,
            });
            app.selected_surface = Some((x, y, z));
            app.selected_surface_face = Some(selection.anchor.face);
            return Ok(crate::plugin_ui::tool_click(app, "perspective"));
        }
    }
    Ok(Task::none())
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let result = (|| -> Result<Task<Message>, String> {
        match action {
            Action::Height(value) => {
                app.work_plane.draft = value;
                app.work_plane.report.clear();
            }
            Action::Toggle(enabled) => {
                let saved = Saved {
                    enabled,
                    ..app.work_plane.saved
                };
                apply(app, saved)?;
            }
            Action::Apply => {
                let height = app
                    .work_plane
                    .draft
                    .trim()
                    .parse::<i32>()
                    .map_err(|e| format!("工作平面高度不是有效整数：{e}"))?;
                let saved = Saved {
                    height,
                    ..app.work_plane.saved
                };
                apply(app, saved)?;
            }
            Action::Step(delta) => {
                let height = app
                    .work_plane
                    .draft
                    .trim()
                    .parse::<i32>()
                    .map_err(|e| format!("工作平面高度不是有效整数：{e}"))?
                    .checked_add(delta)
                    .ok_or("工作平面高度溢出")?;
                let saved = Saved {
                    height,
                    ..app.work_plane.saved
                };
                apply(app, saved)?;
            }
            Action::Pick(selection) => return choose(app, selection),
            Action::Error(error) => return Err(error),
        }
        Ok(Task::none())
    })();
    match result {
        Ok(task) => task,
        Err(error) => {
            log::error!("[work-plane] {error}");
            app.work_plane.report = error.clone();
            app.surface_report = error;
            Task::none()
        }
    }
}
pub(crate) fn controls(app: &App) -> Element<'_, Message> {
    if !app.spectator || app.show_top_down {
        return column![].into();
    }
    let l = app.language;
    column![
        checkbox(app.work_plane.saved.enabled).label(l.pick("相机工作平面","Camera work plane")).on_toggle(|enabled|Message::WorkPlane(Action::Toggle(enabled))),
        row![button("−").on_press(Message::WorkPlane(Action::Step(-1))),text_input("Y",&app.work_plane.draft).on_input(|value|Message::WorkPlane(Action::Height(value))).on_submit(Message::WorkPlane(Action::Apply)),button("+").on_press(Message::WorkPlane(Action::Step(1))),button(l.pick("应用高度","Set height")).on_press(Message::WorkPlane(Action::Apply))].spacing(5),
        text(l.pick("射线先遇到地形时选择地形，否则落到工作平面；笔刷、选框与点选共用此位置。","Use the nearest terrain hit, otherwise the work plane; brushes and selection share this position.")).size(12),
        text(&app.work_plane.report).size(12),
    ].spacing(6).into()
}
