//! 各投影独立保存屏幕变换；导航只修改观察状态，不读取或改写源方块。
use iced::{mouse, widget::canvas, Point, Rectangle, Vector};

pub(crate) const MIN_ZOOM: f32 = 1. / 64.;
pub(crate) const MAX_ZOOM: f32 = 64.;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuadPane {
    Horizontal,
    Front,
    Side,
    Isometric,
}
impl QuadPane {
    pub(crate) fn index(self) -> usize {
        self as usize
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Isometric,
    TopDown,
    Horizontal,
    Vertical,
    Perspective,
    Quad(QuadPane),
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Action {
    Pan(Vector),
    Zoom {
        steps: f32,
        anchor: Point,
        center: Point,
    },
    Reset,
}
impl Action {
    pub(crate) fn centered_zoom(steps: f32) -> Self {
        Self::Zoom {
            steps,
            anchor: Point::ORIGIN,
            center: Point::ORIGIN,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Viewport {
    pub zoom: f32,
    pub pan: Vector,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            zoom: 1.,
            pan: Vector::ZERO,
        }
    }
}
pub(crate) fn zoom_after(zoom: f32, steps: f32) -> Result<f32, String> {
    if !zoom.is_finite() || !(MIN_ZOOM..=MAX_ZOOM).contains(&zoom) || !steps.is_finite() {
        return Err("观察缩放必须是有效范围内的有限数值".into());
    }
    Ok((f64::from(zoom) * 1.15_f64.powf(f64::from(steps)))
        .clamp(f64::from(MIN_ZOOM), f64::from(MAX_ZOOM)) as f32)
}
impl Viewport {
    /// 光标所在源投影点保持不动，包括达到缩放边界的最后一步。
    pub(crate) fn apply(&mut self, action: Action) -> Result<bool, String> {
        let mut next = *self;
        match action {
            Action::Reset => next = Self::default(),
            Action::Pan(delta) => next.pan = self.pan + delta,
            Action::Zoom {
                steps,
                anchor,
                center,
            } => {
                if ![anchor.x, anchor.y, center.x, center.y]
                    .into_iter()
                    .all(f32::is_finite)
                {
                    return Err("观察缩放锚点必须是有限坐标".into());
                }
                next.zoom = zoom_after(self.zoom, steps)?;
                let ratio = f64::from(next.zoom) / f64::from(self.zoom);
                let anchored = |a: f32, c: f32, p: f32| {
                    let offset = f64::from(a) - f64::from(c);
                    (offset - (offset - f64::from(p)) * ratio) as f32
                };
                next.pan = Vector::new(
                    anchored(anchor.x, center.x, self.pan.x),
                    anchored(anchor.y, center.y, self.pan.y),
                );
            }
        }
        if !next.pan.x.is_finite() || !next.pan.y.is_finite() {
            return Err("观察平移超出有限屏幕坐标范围".into());
        }
        let changed = next != *self;
        *self = next;
        Ok(changed)
    }
}

pub(crate) fn wheel(
    target: Target,
    event: &iced::Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
) -> Option<canvas::Action<crate::Message>> {
    let iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) = event else {
        return None;
    };
    let anchor = cursor.position_in(bounds)?;
    let steps = match *delta {
        mouse::ScrollDelta::Lines { y, .. } => y,
        mouse::ScrollDelta::Pixels { y, .. } => y / 80.,
    };
    if !steps.is_finite() {
        log::error!("[view] 拒绝非有限滚轮输入：{steps}");
        return Some(canvas::Action::capture());
    }
    if steps == 0. {
        return None;
    }
    Some(
        canvas::Action::publish(crate::Message::Navigate(
            target,
            Action::Zoom {
                steps,
                anchor,
                center: Point::new(bounds.width * 0.5, bounds.height * 0.5),
            },
        ))
        .and_capture(),
    )
}

#[derive(Default)]
pub(crate) struct Gesture {
    pointer: Option<Point>,
    dragging: bool,
    motion: Option<Vector>,
}
impl Gesture {
    pub(crate) fn dragging(&self) -> bool {
        self.dragging
    }
    pub(crate) fn motion(&self) -> Option<Vector> {
        self.motion
    }
    pub(crate) fn update(
        &mut self,
        target: Target,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<crate::Message>> {
        // cursor 已经过 Scrollable 的坐标变换。原始事件位置仅用于计算位移，
        // 不得覆盖传入光标，否则滚动页面中的点选和滚轮会落到画布之外。
        self.motion = match event {
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => self
                .pointer
                .replace(*position)
                .map(|previous| *position - previous),
            _ => None,
        };
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                cursor.position_in(bounds)?;
                self.dragging = true;
                Some(canvas::Action::capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle))
                if self.dragging =>
            {
                self.dragging = false;
                Some(canvas::Action::capture())
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) if self.dragging => {
                Some(match self.motion {
                    Some(delta) => canvas::Action::publish(crate::Message::Navigate(
                        target,
                        Action::Pan(delta),
                    ))
                    .and_capture(),
                    None => canvas::Action::capture(),
                })
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.dragging =>
            {
                Some(canvas::Action::capture())
            }
            iced::Event::Mouse(mouse::Event::CursorLeft)
            | iced::Event::Window(iced::window::Event::Unfocused) => {
                self.pointer = None;
                self.motion = None;
                let dragging = self.dragging;
                self.dragging = false;
                dragging.then(canvas::Action::capture)
            }
            _ => wheel(target, event, bounds, cursor),
        }
    }
}

impl crate::App {
    pub(crate) fn active_view(&self) -> Target {
        if self.show_top_down {
            Target::TopDown
        } else if self.spectator {
            Target::Perspective
        } else {
            Target::Isometric
        }
    }
    pub(crate) fn navigate(&mut self, target: Target, action: Action) {
        let result = match target {
            Target::Isometric if self.uses_world_mesh() => self.navigate_world_orthographic(action),
            Target::Isometric => self.surface_navigation.apply(action),
            Target::TopDown => self.top_down_navigation.apply(action),
            Target::Horizontal => self.slice_navigation.apply(action),
            Target::Vertical => self.vertical_navigation.apply(action),
            Target::Quad(pane) => {
                crate::quad_view::navigate(self, pane, action);
                return;
            }
            Target::Perspective => match action {
                Action::Zoom { steps, .. } => self.spectator_camera.zoom_by(steps),
                Action::Reset => {
                    let changed = self.spectator_camera.zoom != 1.;
                    self.spectator_camera.zoom = 1.;
                    Ok(changed)
                }
                Action::Pan(delta) => {
                    let result = self.spectator_camera.pan(delta);
                    if matches!(result, Ok(true)) {
                        self.recenter_spectator();
                    }
                    result
                }
            },
        };
        if let Err(error) = result {
            log::error!("[view] {target:?} 导航被拒绝：{error}");
            self.surface_report = format!("视图导航失败：{error}");
        }
    }
}

pub(crate) fn pan_controls(target: Target) -> iced::widget::Row<'static, crate::Message> {
    use iced::widget::{button, row};
    let pan = |label, x, y| {
        button(label).on_press(crate::Message::Navigate(
            target,
            Action::Pan(Vector::new(x, y)),
        ))
    };
    row![
        pan("←", -32., 0.),
        pan("→", 32., 0.),
        pan("↑", 0., -32.),
        pan("↓", 0., 32.)
    ]
    .spacing(4)
}
pub(crate) fn controls(target: Target) -> iced::widget::Row<'static, crate::Message> {
    use iced::widget::{button, row};
    row![
        button("−").on_press(crate::Message::Navigate(target, Action::centered_zoom(-1.))),
        button("+").on_press(crate::Message::Navigate(target, Action::centered_zoom(1.))),
        pan_controls(target),
        button("重置此视图").on_press(crate::Message::Navigate(target, Action::Reset))
    ]
    .spacing(4)
}

#[cfg(test)]
#[path = "view_navigation_tests.rs"]
mod tests;
