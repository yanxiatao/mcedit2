//! 自由视角的源方块交互与投影辅助线，读取 GPU 已提交的不可变网格。
use crate::{
    picking::{Hit, Intent, Owner, Request},
    spectator::{Camera, StreamMesh},
    Message,
};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{keyboard, mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use mcedit2_world::editing::Bounds;
use std::sync::{Arc, Mutex};
pub(crate) struct Controls {
    pub workplane: Option<crate::work_plane::Plane>,
    pub selection_handle: Option<crate::selection_handle::Layer>,
    pub presented: Arc<Mutex<Option<Arc<StreamMesh>>>>,
    pub presented_view: crate::spectator::PresentedViewCache,
    pub key: Option<(usize, usize)>,
    pub selected: Option<(i32, i32, i32)>,
    pub selection: Option<Bounds>,
    pub flying: bool,
    pub brush: Option<crate::brush_stroke::Config>,
}
/// 准星独立绘制在材质光标上方；事件仍仅由 Controls 处理。
pub(crate) struct Crosshair {
    pub anchor: Option<Point>,
}
impl canvas::Program<Message> for Crosshair {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let center = self
            .anchor
            .unwrap_or(Point::new(bounds.width * 0.5, bounds.height * 0.5));
        let (cx, cy) = (center.x, center.y);
        let stroke = Stroke::default().with_width(1.5).with_color(Color::WHITE);
        frame.stroke(
            &Path::line(Point::new(cx - 8., cy), Point::new(cx + 8., cy)),
            stroke,
        );
        frame.stroke(
            &Path::line(Point::new(cx, cy - 8.), Point::new(cx, cy + 8.)),
            stroke,
        );
        vec![frame.into_geometry()]
    }
}
struct Drag {
    start: Point,
    owner: Owner,
    intent: Intent,
    context: Arc<Mutex<Option<Arc<StreamMesh>>>>,
}
struct PlaneDrag {
    key: (usize, usize),
    start: Point,
    anchor: crate::work_plane::Anchor,
    intent: crate::work_plane::Intent,
    source: crate::brush_stroke::Source,
    plane: crate::work_plane::Plane,
}
#[derive(Default)]
pub(crate) struct ControlState {
    workplane_seen: Option<crate::work_plane::Plane>,
    plane_hover: Option<crate::work_plane::Anchor>,
    plane_drag: Option<PlaneDrag>,
    navigation: crate::view_navigation::Gesture,
    /// 事件中的窗口坐标；iced 同批事件传入的 cursor 已可能位于最终位置。
    looking: bool,
    last: Option<Point>,
    hover: Option<Hit>,
    drag: Option<Drag>,
    brush: Option<crate::brush_stroke::Drag>,
    modifiers: keyboard::Modifiers,
    hover_query: crate::hover_query::Gate<Option<crate::hover_query::WorldKey>>,
}
fn position(hit: Option<&Hit>) -> Option<(i32, i32, i32)> {
    hit?.owner.as_ref().map(|owner| {
        let [x, y, z] = owner.position;
        (x, y, z)
    })
}
impl Controls {
    fn selection_view(&self, size: Size) -> Option<crate::selection_handle::View> {
        Some(crate::selection_handle::View::World(
            self.frame(size)?.camera,
            size,
        ))
    }
    fn frame(&self, size: Size) -> Option<crate::spectator::PresentedView> {
        let mesh = self
            .presented
            .lock()
            .expect("相机拾取状态锁被毒化")
            .clone()?;
        let view = self
            .presented_view
            .lock()
            .expect("相机拾取状态锁被毒化")
            .clone()?;
        if !Arc::ptr_eq(&view.mesh, &mesh)
            || view.viewport.logical != size
            || !matches!(view.camera, crate::world_camera::WorldCamera::Free(_))
        {
            return None;
        }
        Some(view)
    }
    fn hover(
        &self,
        state: &mut ControlState,
        point: Option<Point>,
        size: Size,
        now: std::time::Instant,
    ) -> Option<canvas::Action<Message>> {
        let shown = point.and_then(|point| self.frame(size).map(|frame| (frame, point)));
        let key = shown
            .as_ref()
            .map(|(frame, point)| crate::hover_query::WorldKey::new(frame, *point));
        match state.hover_query.request(key, now) {
            crate::hover_query::Decision::Unchanged => return None,
            crate::hover_query::Decision::At(at) => {
                return Some(if self.flying {
                    canvas::Action::request_redraw()
                } else {
                    canvas::Action::request_redraw_at(at)
                })
            }
            crate::hover_query::Decision::Run => {}
        }
        let before = position(state.hover.as_ref())
            .or_else(|| state.plane_hover.map(|anchor| anchor.position.into()));
        let before_plane = state.plane_hover;
        let outline = state
            .hover
            .as_ref()
            .filter(|h| h.owner.is_some() || h.marker.is_some())
            .map(|h| h.outline);
        state.plane_hover = None;
        state.hover = match shown
            .map(|(view, point)| crate::work_plane::pick(&view, point, self.workplane))
            .transpose()
        {
            Ok(Some(crate::work_plane::Picked::Plane(anchor))) => {
                state.plane_hover = Some(anchor);
                None
            }
            Ok(Some(crate::work_plane::Picked::Surface(hit))) => hit,
            Ok(None) => None,
            Err(error) => {
                return Some(canvas::Action::publish(Message::WorkPlane(
                    crate::work_plane::Action::Error(error),
                )))
            }
        };
        let next = position(state.hover.as_ref())
            .or_else(|| state.plane_hover.map(|anchor| anchor.position.into()));
        if before != next || before_plane != state.plane_hover || next.is_some() {
            Some(canvas::Action::publish(if state.plane_hover.is_some() {
                Message::WorldHover(crate::world_hover::Action::Plane(next))
            } else {
                Message::HoverSurface(crate::view_navigation::Target::Perspective, next)
            }))
        } else if outline
            != state
                .hover
                .as_ref()
                .filter(|h| h.owner.is_some() || h.marker.is_some())
                .map(|h| h.outline)
        {
            Some(canvas::Action::request_redraw())
        } else {
            None
        }
    }
}
/// 裁剪到相同相机的近远平面，再投影；跨近裁剪面的选框边不整条消失。
fn line(
    frame: &mut Frame,
    camera: Camera,
    size: Size,
    a: [f64; 3],
    b: [f64; 3],
    color: Color,
    width: f32,
) {
    if let Some((a, b)) = crate::world_camera::WorldCamera::Free(camera).segment(a, b, size) {
        frame.stroke(
            &Path::line(a, b),
            Stroke::default().with_color(color).with_width(width),
        );
    }
}

pub(crate) fn draw_box(
    frame: &mut Frame,
    camera: Camera,
    size: Size,
    bounds: Bounds,
    color: Color,
) {
    let corners: [[f64; 3]; 8] = std::array::from_fn(|mask| {
        std::array::from_fn(|i| {
            if mask & (1 << i) == 0 {
                bounds.minimum[i] as f64 - 0.5
            } else {
                bounds.maximum[i] as f64 + 0.5
            }
        })
    });
    for mask in 0..8 {
        for axis in 0..3 {
            if mask & (1 << axis) == 0 {
                line(
                    frame,
                    camera,
                    size,
                    corners[mask],
                    corners[mask | (1 << axis)],
                    color,
                    1.6,
                );
            }
        }
    }
}
impl canvas::Program<Message> for Controls {
    type State = ControlState;
    fn draw(
        &self,
        state: &ControlState,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let size = bounds.size();
        let mut frame = Frame::new(renderer, size);
        let Some(crate::spectator::PresentedView {
            camera: crate::world_camera::WorldCamera::Free(camera),
            ..
        }) = self.frame(size)
        else {
            return vec![frame.into_geometry()];
        };
        if let (Some(plane), Some(shown)) = (self.workplane, self.frame(size)) {
            crate::work_plane::draw(&mut frame, &shown, plane, state.plane_hover);
        }
        let input = if !state.looking && !state.modifiers.alt() && !state.modifiers.control() {
            cursor.position_in(bounds).and_then(|point| {
                self.frame(size).map(|shown| {
                    crate::brush_stroke::InputFrame::world_with_plane(shown, point, self.workplane)
                })
            })
        } else {
            None
        };
        crate::brush_cursor::draw(
            &mut frame,
            self.brush.as_ref(),
            crate::view_navigation::Target::Perspective,
            input,
            crate::brush_cursor::Projection::World(camera.into()),
            size,
        );
        if let Some(b) = self.selection.filter(|_| self.selection_handle.is_none()) {
            draw_box(&mut frame, camera, size, b, Color::from_rgb(0.1, 1., 0.9));
        }
        if let Some((x, y, z)) = self.selected {
            draw_box(
                &mut frame,
                camera,
                size,
                Bounds {
                    minimum: [x, y, z],
                    maximum: [x, y, z],
                },
                Color::from_rgb(0.1, 1., 0.9),
            );
        }
        if let Some(hit) = state
            .hover
            .as_ref()
            .filter(|h| h.owner.is_some() || h.marker.is_some())
        {
            let outline = hit.display_outline(camera);
            for i in 0..4 {
                line(
                    &mut frame,
                    camera,
                    size,
                    outline[i],
                    outline[(i + 1) % 4],
                    Color::from_rgb(1., 0.9, 0.2),
                    2.,
                );
            }
        }
        if let Some(drag) = &state.drag {
            if let Some(owner) = state.hover.as_ref().and_then(|h| h.owner.as_ref()) {
                let bounds = Bounds {
                    minimum: std::array::from_fn(|i| drag.owner.position[i].min(owner.position[i])),
                    maximum: std::array::from_fn(|i| drag.owner.position[i].max(owner.position[i])),
                };
                draw_box(
                    &mut frame,
                    camera,
                    size,
                    bounds,
                    Color::from_rgb(0.9, 0.7, 0.1),
                );
            }
        }
        if let Some(drag) = &state.plane_drag {
            if let Some(end) = state.plane_hover.map(|anchor| anchor.position).or_else(|| {
                state
                    .hover
                    .as_ref()
                    .and_then(|hit| hit.owner.as_ref())
                    .map(|owner| owner.position)
            }) {
                let start = drag.anchor.position;
                draw_box(
                    &mut frame,
                    camera,
                    size,
                    Bounds {
                        minimum: std::array::from_fn(|i| start[i].min(end[i])),
                        maximum: std::array::from_fn(|i| start[i].max(end[i])),
                    },
                    Color::from_rgb(1., 0.7, 0.15),
                );
            }
        }
        if let Some(brush) = &state.brush {
            for &(a, b) in &brush.lines {
                line(
                    &mut frame,
                    camera,
                    size,
                    a.map(f64::from),
                    b.map(f64::from),
                    Color::from_rgb(1., 0.65, 0.1),
                    2.,
                );
            }
            if let Some(bounds) = brush.cursor {
                draw_box(
                    &mut frame,
                    camera,
                    size,
                    bounds,
                    Color::from_rgb(1., 0.65, 0.1),
                );
            }
        } else if let (Some(config), Some(hit)) = (
            &self.brush,
            state.hover.as_ref().filter(|h| h.owner.is_some()),
        ) {
            match crate::brush_stroke::cursor(config, hit) {
                Ok(bounds) => draw_box(
                    &mut frame,
                    camera,
                    size,
                    bounds,
                    Color::from_rgb(1., 0.65, 0.1),
                ),
                Err(error) => frame.fill_text(canvas::Text {
                    content: error,
                    color: Color::from_rgb(1., 0.35, 0.35),
                    ..Default::default()
                }),
            }
        }
        if let Some(layer) = &self.selection_handle {
            layer.draw(
                crate::selection_handle::View::World(camera.into(), size),
                &mut frame,
                cursor.position_in(bounds),
            );
        }
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        state: &mut ControlState,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if state.workplane_seen != self.workplane {
            state.workplane_seen = self.workplane;
            state.drag = None;
            state.plane_drag = None;
            state.hover = None;
            state.plane_hover = None;
            state.hover_query.reset();
        }
        let result = (|| {
            if let Some(layer) = &self.selection_handle {
                if let Some(action) = layer.event(
                    crate::view_navigation::Target::Perspective,
                    self.selection_view(bounds.size()),
                    event,
                    bounds,
                    cursor,
                    |p| {
                        self.frame(bounds.size()).map(|frame| {
                            if matches!(
                                layer.config.mode,
                                crate::selection_handle::Mode::BlockInspect(_)
                            ) {
                                return crate::brush_stroke::InputFrame::inspect_block(frame, p);
                            }
                            crate::brush_stroke::InputFrame::world_with_plane(
                                frame,
                                p,
                                self.workplane,
                            )
                        })
                    },
                ) {
                    return Some(action);
                }
            }

            let event_cursor = cursor;
            if let Some(action) = crate::brush_stroke::event(
                self.brush
                    .as_ref()
                    .filter(|_| !state.looking && !state.navigation.dragging()),
                &mut state.brush,
                self.key,
                event,
                event_cursor.position_in(bounds),
                state.modifiers,
                || {
                    let point = event_cursor.position_in(bounds)?;
                    self.frame(bounds.size()).map(|frame| {
                        crate::brush_stroke::InputFrame::world_with_plane(
                            frame,
                            point,
                            self.workplane,
                        )
                    })
                },
            ) {
                state.drag = None;
                if matches!(
                    event,
                    canvas::Event::Mouse(mouse::Event::CursorLeft)
                        | canvas::Event::Window(iced::window::Event::Unfocused)
                ) {
                    state.hover = None;
                    state.plane_hover = None;
                    state.plane_drag = None;
                    state.modifiers = keyboard::Modifiers::default();
                    state.hover_query.reset();
                }
                return Some(action);
            }
            if let Some(action) = state.navigation.update(
                crate::view_navigation::Target::Perspective,
                event,
                bounds,
                cursor,
            ) {
                state.drag = None;
                state.plane_drag = None;
                return Some(action);
            }
            match event {
                canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => {
                    state.modifiers = *m;
                    None
                }
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    let point = event_cursor.position_in(bounds)?;
                    let shown = self.frame(bounds.size())?;
                    let picked = match crate::work_plane::pick(&shown, point, self.workplane) {
                        Ok(picked) => picked,
                        Err(error) => {
                            return Some(
                                canvas::Action::publish(Message::WorkPlane(
                                    crate::work_plane::Action::Error(error),
                                ))
                                .and_capture(),
                            )
                        }
                    };
                    if let Some(plane) = self.workplane {
                        let anchor = match &picked {
                            crate::work_plane::Picked::Plane(anchor) => Some(*anchor),
                            crate::work_plane::Picked::Surface(Some(hit)) => hit
                                .owner
                                .as_ref()
                                .filter(|_| hit.marker.is_none())
                                .map(|owner| crate::work_plane::Anchor {
                                    position: owner.position,
                                    face: hit.face,
                                }),
                            crate::work_plane::Picked::Surface(None) => None,
                        };
                        if let Some(anchor) = anchor {
                            state.drag = None;
                            state.plane_drag = Some(PlaneDrag {
                                key: self.key?,
                                start: point,
                                anchor,
                                intent: if state.modifiers.alt() {
                                    crate::work_plane::Intent::CornerA
                                } else if state.modifiers.control() {
                                    crate::work_plane::Intent::CornerB
                                } else {
                                    crate::work_plane::Intent::Point
                                },
                                source: crate::brush_stroke::Source::WorldPlane(shown.mesh, plane),
                                plane,
                            });
                            return Some(canvas::Action::capture());
                        }
                    }
                    let crate::work_plane::Picked::Surface(Some(hit)) = picked else {
                        return None;
                    };
                    let mesh = shown.mesh;
                    if let Some(marker) = &hit.marker {
                        state.drag = None;
                        return Some(
                            canvas::Action::publish(Message::EntityModelSelected(
                                mesh,
                                marker.clone(),
                            ))
                            .and_capture(),
                        );
                    }

                    let owner = hit.owner.clone()?;
                    let intent = if state.modifiers.alt() {
                        Intent::CornerA
                    } else if state.modifiers.control() {
                        Intent::CornerB
                    } else {
                        Intent::Point
                    };
                    state.drag = Some(Drag {
                        start: point,
                        owner,
                        intent,
                        context: Arc::clone(&self.presented),
                    });
                    state.hover = Some(hit);
                    Some(canvas::Action::capture())
                }
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    if let Some(drag) = state.plane_drag.take() {
                        let point = event_cursor.position_in(bounds)?;
                        let shown = self.frame(bounds.size())?;
                        if self.workplane != Some(drag.plane)
                            || !drag.source.same(&crate::brush_stroke::Source::WorldPlane(
                                Arc::clone(&shown.mesh),
                                drag.plane,
                            ))
                        {
                            return Some(
                                canvas::Action::publish(Message::WorkPlane(
                                    crate::work_plane::Action::Error(
                                        "工作平面或已显示来源改变，已取消点选".into(),
                                    ),
                                ))
                                .and_capture(),
                            );
                        }
                        let input = crate::brush_stroke::InputFrame::world_with_plane(
                            shown,
                            point,
                            self.workplane,
                        );
                        let sample = match input.sample {
                            Ok(Some(sample)) => sample,
                            Ok(None) => return Some(canvas::Action::capture()),
                            Err(error) => {
                                return Some(
                                    canvas::Action::publish(Message::WorkPlane(
                                        crate::work_plane::Action::Error(error),
                                    ))
                                    .and_capture(),
                                )
                            }
                        };
                        let intent = if matches!(drag.intent, crate::work_plane::Intent::Point)
                            && (point.x - drag.start.x).hypot(point.y - drag.start.y) >= 4.
                        {
                            crate::work_plane::Intent::Box(drag.anchor.position)
                        } else {
                            drag.intent
                        };
                        return Some(
                            canvas::Action::publish(Message::WorkPlane(
                                crate::work_plane::Action::Pick(crate::work_plane::Selection {
                                    key: drag.key,
                                    source: drag.source,
                                    plane: drag.plane,
                                    anchor: crate::work_plane::Anchor {
                                        position: sample.position(),
                                        face: sample.face(),
                                    },
                                    intent,
                                }),
                            ))
                            .and_capture(),
                        );
                    }
                    let drag = state.drag.take()?;
                    if !Arc::ptr_eq(&drag.context, &self.presented) {
                        return Some(canvas::Action::capture());
                    }
                    let point = event_cursor.position_in(bounds)?;
                    let shown = self.frame(bounds.size())?;
                    let hit = shown.pick(point)?;
                    let mesh = shown.mesh;
                    hit.owner.as_ref()?;
                    let moved = (point.x - drag.start.x).hypot(point.y - drag.start.y) >= 4.;
                    let intent = if moved && matches!(drag.intent, Intent::Point) {
                        Intent::Box(drag.owner)
                    } else {
                        drag.intent
                    };
                    Some(
                        canvas::Action::publish(Message::SpectatorPick(Request {
                            key: self.key?,
                            mesh,
                            hit,
                            intent,
                        }))
                        .and_capture(),
                    )
                }
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                    let point = event_cursor.position_in(bounds)?;
                    state.looking = true;
                    state.last = Some(point);
                    state.drag = None;
                    state.plane_drag = None;
                    Some(canvas::Action::capture())
                }
                canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) => {
                    state.looking = false;
                    state.last = None;
                    Some(canvas::Action::capture())
                }
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if state.looking => {
                    let point = event_cursor.position_in(bounds)?;
                    let last = state.last.replace(point)?;
                    Some(
                        canvas::Action::publish(Message::SpectatorLook(
                            point.x - last.x,
                            point.y - last.y,
                        ))
                        .and_capture(),
                    )
                }
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => self.hover(
                    state,
                    event_cursor.position_in(bounds),
                    bounds.size(),
                    std::time::Instant::now(),
                ),
                canvas::Event::Window(iced::window::Event::RedrawRequested(now)) => {
                    if state.looking {
                        return self.flying.then(canvas::Action::request_redraw);
                    }
                    self.hover(state, cursor.position_in(bounds), bounds.size(), *now)
                        .or_else(|| self.flying.then(canvas::Action::request_redraw))
                }
                canvas::Event::Mouse(mouse::Event::CursorLeft)
                | canvas::Event::Window(iced::window::Event::Unfocused) => {
                    state.looking = false;
                    state.last = None;
                    state.drag = None;
                    state.plane_drag = None;
                    state.plane_hover = None;
                    state.modifiers = keyboard::Modifiers::default();
                    state.hover = None;
                    state.hover_query.reset();
                    Some(canvas::Action::publish(Message::HoverSurface(
                        crate::view_navigation::Target::Perspective,
                        None,
                    )))
                }
                _ => None,
            }
        })();
        result.or_else(|| {
            self.selection_handle
                .as_ref()
                .and_then(|layer| layer.hover_redraw(event))
        })
    }
    fn mouse_interaction(
        &self,
        state: &ControlState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(interaction) = self.selection_handle.as_ref().and_then(|layer| {
            layer.interaction(
                self.selection_view(bounds.size()),
                cursor.position_in(bounds),
            )
        }) {
            return interaction;
        }

        if state.looking
            || state.drag.is_some()
            || state.plane_drag.is_some()
            || state.brush.is_some()
        {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Crosshair
        }
    }
}

/// 真实源场景的事件开销诊断；不设时间门槛，不创建窗口，不执行编辑消息。
#[cfg(test)]
pub(super) fn profile_controls(mesh: Arc<StreamMesh>, camera: Camera) -> serde_json::Value {
    use canvas::Program;
    let size = Size::new(694., 555.);
    let points = (0..1000)
        .map(|i| {
            Point::new(
                size.width * (0.25 + 0.5 * (i % 32) as f32 / 31.),
                size.height * (0.25 + 0.5 * (i / 32) as f32 / 31.),
            )
        })
        .collect::<Vec<_>>();
    let quality = crate::gpu_mesh::Quality::Fast;
    let mut pick = None;
    let mut hits = 0;
    let start = std::time::Instant::now();
    for point in &points {
        if let Some(hit) = mesh.pick(
            camera,
            *point,
            crate::render_viewport::RenderViewport::one_to_one(size),
            quality,
        ) {
            hits += 1;
            if pick.is_none() && hit.marker.is_none() {
                pick = hit.owner.map(|owner| (*point, owner));
            }
        }
    }
    let raw_ms = start.elapsed().as_secs_f64() * 1000.;
    let shown = crate::spectator::PresentedView {
        atlas: mesh.atlas.clone(),
        mesh: Arc::clone(&mesh),
        camera: camera.into(),
        viewport: crate::render_viewport::RenderViewport::one_to_one(size),
        quality,
    };
    let controls = Controls {
        workplane: None,
        selection_handle: None,
        presented: Arc::new(Mutex::new(Some(Arc::clone(&mesh)))),
        presented_view: Arc::new(Mutex::new(Some(shown))),
        key: Some((0, 0)),
        selected: None,
        selection: None,
        brush: None,
        flying: false,
    };
    let mut state = ControlState::default();
    let bounds = Rectangle::with_size(size);
    let start = std::time::Instant::now();
    let mut deadline = None;
    for point in &points {
        if let Some(action) = controls.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::CursorMoved { position: *point }),
            bounds,
            mouse::Cursor::Available(*point),
        ) {
            if let iced::window::RedrawRequest::At(at) = action.into_inner().1 {
                deadline = Some(at);
            }
        }
    }
    let event_ms = start.elapsed().as_secs_f64() * 1000.;
    let queries = state.hover_query.runs;
    if let Some(at) = deadline {
        controls.update(
            &mut state,
            &canvas::Event::Window(iced::window::Event::RedrawRequested(at)),
            bounds,
            mouse::Cursor::Available(*points.last().unwrap()),
        );
    }
    let mut exact_click = false;
    if let Some((point, owner)) = pick {
        controls.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::CursorMoved { position: point }),
            bounds,
            mouse::Cursor::Available(point),
        );
        controls.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(point),
        );
        let action = controls
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Available(point),
            )
            .unwrap();
        let Some(Message::SpectatorPick(request)) = action.into_inner().0 else {
            panic!("点击应直接返回真实源方块");
        };
        assert_eq!(request.hit.owner.as_ref(), Some(&owner));
        assert!(Arc::ptr_eq(&request.mesh, &mesh));
        exact_click = true;
    }
    serde_json::json!({"events":points.len(),"raw_pick_ms":raw_ms,"raw_hits":hits,
        "coalesced_events_ms":event_ms,"coalesced_queries":queries,"exact_click":exact_click})
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::Program;

    #[test]
    #[cfg(debug_assertions)]
    #[cfg(debug_assertions)]
    fn queued_mouse_events_keep_drag_start_when_runtime_cursor_is_already_at_end() {
        let (app, _scene, _, original) = crate::picking_integration_tests::fixture();
        let controls = Controls {
            workplane: None,
            selection_handle: None,
            presented: Arc::clone(&app.spectator_presented),
            presented_view: Arc::clone(&app.spectator_presented_view),
            key: Some((0, 0)),
            selected: None,
            selection: None,
            brush: None,
            flying: false,
        };
        let mut state = ControlState::default();
        let bounds = Rectangle {
            x: 20.,
            y: 30.,
            width: 200.,
            height: 200.,
        };
        let start = Point::new(120., 130.);
        let end = Point::new(130., 130.);
        let cursor = mouse::Cursor::Available(end);
        let events = [
            mouse::Event::CursorMoved { position: start },
            mouse::Event::ButtonPressed(mouse::Button::Left),
            mouse::Event::CursorMoved { position: end },
        ]
        .map(canvas::Event::Mouse);
        let cursors = crate::input_batches::cursors(&events, end);
        for (event, actual) in events.iter().zip(cursors) {
            controls.update(&mut state, event, bounds, actual);
        }
        let action = controls
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                bounds,
                cursor,
            )
            .unwrap();
        let Some(Message::SpectatorPick(request)) = action.into_inner().0 else {
            panic!("拖动应提交选区请求");
        };
        assert!(
            matches!(request.intent,Intent::Box(ref owner) if Some(owner)==original.hit.owner.as_ref()),
            "批量分发事件仍应保留真实按下位置"
        );
    }
    fn click(
        controls: &Controls,
        state: &mut ControlState,
        from: Point,
        to: Point,
    ) -> Option<Request> {
        let bounds = Rectangle::with_size(Size::new(200., 200.));
        controls.update(
            state,
            &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(from),
        );
        let action = controls.update(
            state,
            &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(to),
        )?;
        match action.into_inner().0 {
            Some(Message::SpectatorPick(request)) => Some(request),
            _ => None,
        }
    }

    #[test]
    fn actual_canvas_events_publish_point_drag_and_corner_requests() {
        let (app, _scene, _, original) = crate::picking_integration_tests::fixture();
        let controls = Controls {
            workplane: None,
            selection_handle: None,
            presented: Arc::clone(&app.spectator_presented),
            presented_view: Arc::clone(&app.spectator_presented_view),
            key: Some((0, 0)),
            selected: None,
            selection: None,
            brush: None,
            flying: false,
        };
        let mut state = ControlState::default();
        let center = Point::new(100., 100.);
        let point = click(&controls, &mut state, center, center).unwrap();
        assert!(matches!(point.intent, Intent::Point));
        assert_eq!(point.hit.owner, original.hit.owner);
        let drag = click(&controls, &mut state, center, Point::new(110., 100.)).unwrap();
        assert!(
            matches!(drag.intent,Intent::Box(ref anchor) if Some(anchor)==original.hit.owner.as_ref())
        );
        let bounds = Rectangle::with_size(Size::new(200., 200.));
        for (modifiers, first) in [
            (keyboard::Modifiers::ALT, true),
            (keyboard::Modifiers::CTRL, false),
        ] {
            controls.update(
                &mut state,
                &canvas::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)),
                bounds,
                mouse::Cursor::Available(center),
            );
            let request = click(&controls, &mut state, center, center).unwrap();
            assert!(if first {
                matches!(request.intent, Intent::CornerA)
            } else {
                matches!(request.intent, Intent::CornerB)
            });
        }
    }

    #[test]
    fn source_context_switch_and_focus_loss_cancel_in_progress_drag() {
        let (app, _scene, mesh, _) = crate::picking_integration_tests::fixture();
        let mut controls = Controls {
            workplane: None,
            selection_handle: None,
            presented: Arc::clone(&app.spectator_presented),
            presented_view: Arc::clone(&app.spectator_presented_view),
            key: Some((0, 0)),
            selected: None,
            selection: None,
            brush: None,
            flying: false,
        };
        let mut state = ControlState::default();
        let center = Point::new(100., 100.);
        let bounds = Rectangle::with_size(Size::new(200., 200.));
        controls.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(center),
        );
        controls.presented = Arc::new(Mutex::new(Some(mesh)));
        let release = controls
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Available(center),
            )
            .unwrap();
        assert!(release.into_inner().0.is_none());
        assert!(state.drag.is_none());
        controls.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(center),
        );
        assert!(state.drag.is_some());
        controls.update(
            &mut state,
            &canvas::Event::Window(iced::window::Event::Unfocused),
            bounds,
            mouse::Cursor::Available(center),
        );
        assert!(state.drag.is_none());
        assert!(controls
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Available(center)
            )
            .is_none());
    }
}
