//! 与选框共用已显示来源和射线的工具手势，不在空处创建空气或源状态。
use super::*;
use mcedit2_plugins::tool_input::{Gesture, Location, Phase, Ray as ToolRay, Sample};

#[derive(Clone, Debug)]
pub(crate) struct Finished {
    pub config: Config,
    pub source: Source,
    pub target: Target,
    pub gesture: Gesture,
    pub bytes: usize,
}
pub(super) struct Trace {
    pub(super) config: Config,
    source: Source,
    target: Target,
    gesture: Gesture,
    bytes: usize,
}
impl Trace {
    fn sample(
        &mut self,
        c: Config,
        target: Target,
        view: View,
        phase: Phase,
        p: Point,
        modifiers: keyboard::Modifiers,
        input: impl Fn(Point) -> Option<InputFrame>,
    ) -> Result<(), String> {
        if self.config != c || self.target != target {
            return Err("工具手势所属视图或参数已变化".into());
        }
        let frame = input(p).ok_or("当前工具没有可读取的来源视图")?;
        if !self.source.same(&frame.source) {
            return Err("工具手势的来源视图已变化".into());
        }
        let ray = view.ray(p).ok_or("当前工具没有可用的真实射线")?;
        let location = frame.sample?.map(|sample| {
            let position = sample.position();
            let normal: [i32; 3] = sample.face().normal().into();
            let axis = normal
                .iter()
                .position(|value| *value != 0)
                .expect("源面具有方向");
            let point = ray.plane(
                axis,
                f64::from(position[axis]) + f64::from(i32::from(normal[axis] > 0)),
            );
            Location {
                position,
                normal,
                point,
                state: sample.owner().map(|owner| (*owner.state).clone()),
            }
        });
        self.bytes = self
            .bytes
            .checked_add(
                std::mem::size_of::<Sample>()
                    + location
                        .as_ref()
                        .and_then(|location| location.state.as_ref())
                        .map_or(0, mcedit2_world::clipboard::state_bytes),
            )
            .ok_or("工具手势工作集溢出")?;
        if self.bytes.saturating_mul(4) > c.budget {
            return Err("工具手势超过当前工作预算".into());
        }
        self.gesture.samples.push(Sample {
            phase,
            screen: [p.x, p.y],
            ray: ToolRay {
                origin: ray.origin,
                direction: ray.direction,
            },
            location,
            shift: modifiers.shift(),
            control: modifiers.control(),
            alt: modifiers.alt(),
        });
        Ok(())
    }
}
pub(super) fn event(
    active: &mut Option<Trace>,
    c: Config,
    target: Target,
    view: View,
    event: &canvas::Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
    modifiers: keyboard::Modifiers,
    input: impl Fn(Point) -> Option<InputFrame>,
) -> Option<canvas::Action<Message>> {
    if active.as_ref().is_some_and(|trace| trace.config != c) {
        *active = None;
    }
    // 四视图逐窗格分发相同事件；只有按下时的窗格能推进或取消本次手势。
    if active.as_ref().is_some_and(|trace| trace.target != target) {
        return None;
    }
    if matches!(event, canvas::Event::Mouse(mouse::Event::CursorLeft)) {
        return active
            .take()
            .map(|_| canvas::Action::request_redraw().and_capture());
    }
    let point = cursor
        .position()
        .map(|point| Point::new(point.x - bounds.x, point.y - bounds.y));
    let result = match event {
        canvas::Event::Mouse(mouse::Event::CursorMoved { .. })
            if active.is_none() && matches!(c.mode, Mode::Plugin(_, true)) =>
        {
            let point =
                point.filter(|point| Rectangle::with_size(bounds.size()).contains(*point))?;
            let frame = input(point)?;
            let mut trace = Trace {
                config: c,
                source: frame.source.clone(),
                target,
                gesture: Gesture {
                    view: crate::structure_workspace::plugin::view_name(target),
                    samples: Vec::new(),
                },
                bytes: 0,
            };
            trace
                .sample(c, target, view, Phase::Move, point, modifiers, |_| {
                    Some(frame.clone())
                })
                .and_then(|_| trace.gesture.validate())
                .map(|_| Finished {
                    config: c,
                    source: trace.source,
                    target,
                    gesture: trace.gesture,
                    bytes: trace.bytes,
                })
        }
        canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            if !modifiers.alt() && !modifiers.control() =>
        {
            let point =
                point.filter(|point| Rectangle::with_size(bounds.size()).contains(*point))?;
            let frame = input(point)?;
            let mut trace = Trace {
                config: c,
                source: frame.source.clone(),
                target,
                gesture: Gesture {
                    view: crate::structure_workspace::plugin::view_name(target),
                    samples: Vec::new(),
                },
                bytes: 0,
            };
            match trace.sample(c, target, view, Phase::Press, point, modifiers, |_| {
                Some(frame.clone())
            }) {
                Ok(()) => {
                    *active = Some(trace);
                    return Some(canvas::Action::request_redraw().and_capture());
                }
                Err(error) => Err(error),
            }
        }
        canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if active.is_some() => {
            let Some(point) =
                point.filter(|point| Rectangle::with_size(bounds.size()).contains(*point))
            else {
                *active = None;
                return Some(canvas::Action::request_redraw().and_capture());
            };
            match active.as_mut().expect("手势正在进行").sample(
                c,
                target,
                view,
                Phase::Drag,
                point,
                modifiers,
                input,
            ) {
                Ok(()) => return Some(canvas::Action::request_redraw().and_capture()),
                Err(error) => Err(error),
            }
        }
        canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            if active.is_some() =>
        {
            let mut trace = active.take().expect("手势正在进行");
            match point
                .ok_or_else(|| "工具松开事件没有真实鼠标位置".to_owned())
                .and_then(|point| {
                    trace.sample(c, target, view, Phase::Release, point, modifiers, input)
                }) {
                Ok(()) => trace.gesture.validate().map(|_| Finished {
                    config: c,
                    source: trace.source,
                    target,
                    gesture: trace.gesture,
                    bytes: trace.bytes,
                }),
                Err(error) => Err(error),
            }
        }
        _ => return None,
    };
    *active = None;
    Some(
        canvas::Action::publish(Message::Plugin(crate::plugin_ui::Action::Gesture(result)))
            .and_capture(),
    )
}
pub(super) fn draw(active: &Option<Trace>, view: View, frame: &mut Frame) {
    let Some(trace) = active else {
        return;
    };
    let mut previous: Option<[f64; 3]> = None;
    for sample in &trace.gesture.samples {
        let Some(location) = &sample.location else {
            previous = None;
            continue;
        };
        let point = location.position.map(|value| f64::from(value) + 0.5);
        if let Some(from) = previous {
            if let Some((a, b)) = view.segment(
                from.map(|value| value - 0.5),
                point.map(|value| value - 0.5),
            ) {
                frame.stroke(
                    &Path::line(a, b),
                    Stroke::default()
                        .with_color(Color::from_rgb(1., 0.65, 0.1))
                        .with_width(2.),
                );
            }
        }
        previous = Some(point);
    }
}
