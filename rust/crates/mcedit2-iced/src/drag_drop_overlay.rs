//! 在真实覆盖层的事件顺序中交接跨窗口释放；继续使用目标窗口的剪贴板及消息映射。
use super::*;
use iced::advanced::Overlay;

pub(super) struct Routed<'a, M> {
    pub content: overlay::Element<'a, M, Theme, Renderer>,
    pub shared: Arc<Mutex<Shared>>,
    pub window: Option<iced::window::Id>,
    pub wake: M,
    pub relay: fn(Proof, M) -> M,
}
impl<M: Clone> Overlay<M, Theme, Renderer> for Routed<'_, M> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.content.as_overlay_mut().layout(renderer, bounds)
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let cursor = routed_cursor(&self.shared, self.window, cursor);
        self.content
            .as_overlay()
            .draw(renderer, theme, style, layout, cursor);
    }
    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        self.content
            .as_overlay_mut()
            .operate(layout, renderer, operation);
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
    ) {
        if matches!(event, Event::Window(iced::window::Event::FileDropped(_))) {
            let point = {
                let shared = self.shared.lock().expect("拖放状态锁被毒化");
                self.window
                    .map(|id| native::file_point(&shared.native, id))
                    .transpose()
            };
            let blocked = match point {
                Ok(Some(Some(point))) => {
                    self.content.as_overlay().mouse_interaction(
                        layout,
                        mouse::Cursor::Available(point),
                        renderer,
                    ) != mouse::Interaction::None
                }
                Ok(_) => false,
                Err(error) => {
                    let mut shared = self.shared.lock().expect("拖放状态锁被毒化");
                    shared.errors.push(error);
                    wake(&mut shared, &self.wake, shell);
                    shell.capture_event();
                    return;
                }
            };
            if blocked {
                let mut shared = self.shared.lock().expect("拖放状态锁被毒化");
                shared
                    .errors
                    .push("请先关闭当前位置的弹出面板，再拖入文件".into());
                wake(&mut shared, &self.wake, shell);
                shell.capture_event();
                return;
            }
        }
        let point = if matches!(
            event,
            Event::Window(iced::window::Event::RedrawRequested(_))
        ) {
            begin_route(
                &mut self.shared.lock().expect("拖放状态锁被毒化"),
                self.window,
            )
        } else {
            None
        };
        if let Some((point, proof)) = point {
            let cursor = mouse::Cursor::Available(point);
            let interaction = self
                .content
                .as_overlay()
                .mouse_interaction(layout, cursor, renderer);
            let mut messages = Vec::new();
            let mut release = Shell::new(&mut messages);
            self.content.as_overlay_mut().update(
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                layout,
                cursor,
                renderer,
                clipboard,
                &mut release,
            );
            let captured = release.is_event_captured();
            shell.merge(release, |message| (self.relay)(proof.clone(), message));
            let mut shared = self.shared.lock().expect("拖放状态锁被毒化");
            if captured || interaction != mouse::Interaction::None {
                finish_route(&mut shared, &self.wake, shell);
            } else if let Some(route) = &mut shared.pending {
                route.dispatching = false;
            }
        }
        self.content
            .as_overlay_mut()
            .update(event, layout, cursor, renderer, clipboard, shell);
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_overlay().mouse_interaction(
            layout,
            routed_cursor(&self.shared, self.window, cursor),
            renderer,
        )
    }
    fn overlay<'a>(
        &'a mut self,
        layout: Layout<'a>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'a, M, Theme, Renderer>> {
        let shared = Arc::clone(&self.shared);
        let window = self.window;
        let wake = self.wake.clone();
        let relay = self.relay;
        self.content
            .as_overlay_mut()
            .overlay(layout, renderer)
            .map(|content| {
                overlay::Element::new(Box::new(Routed {
                    content,
                    shared,
                    window,
                    wake,
                    relay,
                }))
            })
    }
    fn index(&self) -> f32 {
        self.content.as_overlay().index()
    }
}
pub(super) fn routed_cursor(
    shared: &Arc<Mutex<Shared>>,
    window: Option<iced::window::Id>,
    cursor: mouse::Cursor,
) -> mouse::Cursor {
    let shared = shared.lock().expect("拖放状态锁被毒化");
    if shared.active().is_none() {
        return cursor;
    }
    if shared.window == window {
        shared
            .pointer
            .map_or(mouse::Cursor::Unavailable, mouse::Cursor::Available)
    } else {
        mouse::Cursor::Unavailable
    }
}
