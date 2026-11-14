//! 只响应明确的树导航请求；普通重绘和手动滚动不会被旧选中行拉回。
use super::{Action, Tree as NbtTree};
use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{self, operation, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    widget::scrollable,
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
};
use std::sync::Arc;
const SCROLL: &str = "mcedit2-nbt-tree-scroll";
pub(super) const TARGET: &str = "mcedit2-nbt-tree-target";
pub(super) const RENAME: &str = "mcedit2-nbt-rename-input";
pub(super) const VALUE: &str = "mcedit2-nbt-value-input";
pub(super) const EDITOR: &str = "mcedit2-nbt-inline-editor";
#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    Name,
    Value,
}
impl Input {
    fn id(self) -> &'static str {
        match self {
            Self::Name => RENAME,
            Self::Value => VALUE,
        }
    }
    fn cancel(self) -> Action {
        match self {
            Self::Name => Action::CancelRename,
            Self::Value => Action::CancelInline,
        }
    }
}

pub(super) fn view<'a>(content: Element<'a, Action>) -> Element<'a, Action> {
    scrollable(content)
        .id(SCROLL)
        .direction(scrollable::Direction::Both {
            vertical: scrollable::Scrollbar::new().spacing(6),
            horizontal: scrollable::Scrollbar::new().spacing(6),
        })
        .height(320)
        .into()
}
pub(super) fn wrap<'a>(tree: &NbtTree, content: Element<'a, Action>) -> Element<'a, Action> {
    Element::new(Reveal {
        content,
        ticket: tree.reveal_ticket,
        target: tree.reveal_path.is_some(),
        owner: Arc::clone(&tree.identity),
        focus_input: if tree.renaming.is_some() {
            Some((tree.edit_ticket, Input::Name))
        } else if tree.editing.is_some() {
            Some((tree.edit_ticket, Input::Value))
        } else {
            None
        },
    })
}
#[derive(Default)]
struct Seen {
    ticket: Option<u64>,
    owner: Option<Arc<()>>,
    input: Option<(u64, Input)>,
}
struct Reveal<'a> {
    content: Element<'a, Action>,
    ticket: u64,
    target: bool,
    owner: Arc<()>,
    focus_input: Option<(u64, Input)>,
}
struct Locate {
    bounds: Option<Rectangle>,
    prefer_editor: bool,
    editor_found: bool,
}
impl widget::Operation for Locate {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        operate(self);
    }
    fn container(&mut self, id: Option<&widget::Id>, bounds: Rectangle) {
        if self.prefer_editor && id == Some(&widget::Id::new(EDITOR)) {
            self.bounds = Some(bounds);
            self.editor_found = true;
        } else if !self.editor_found && id == Some(&widget::Id::new(TARGET)) {
            self.bounds = Some(bounds);
        }
    }
}
struct Show(Rectangle);
impl widget::Operation for Show {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        id: Option<&widget::Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        if id != Some(&widget::Id::new(SCROLL)) {
            return;
        }
        let left = self.0.x - translation.x;
        let top = self.0.y - translation.y;
        let x = if left < bounds.x || self.0.width > bounds.width {
            left - bounds.x
        } else if left + self.0.width > bounds.x + bounds.width {
            left + self.0.width - bounds.x - bounds.width
        } else {
            0.
        };
        let y = if top < bounds.y {
            top - bounds.y
        } else if top + self.0.height > bounds.y + bounds.height {
            top + self.0.height - bounds.y - bounds.height
        } else {
            0.
        };
        state.scroll_by(
            operation::scrollable::AbsoluteOffset { x, y },
            bounds,
            content,
        );
    }
}
impl Widget<Action, Theme, Renderer> for Reveal<'_> {
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<Seen>()
    }
    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(Seen::default())
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let node = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let seen = tree.state.downcast_ref::<Seen>();
        let owner_changed = !seen
            .owner
            .as_ref()
            .is_some_and(|owner| Arc::ptr_eq(owner, &self.owner));
        let focus_input = self
            .focus_input
            .filter(|_| owner_changed || seen.input != self.focus_input);
        if seen.ticket != Some(self.ticket) || owner_changed {
            if self.target {
                let mut locate = Locate {
                    bounds: None,
                    prefer_editor: focus_input.is_some(),
                    editor_found: false,
                };
                self.content.as_widget_mut().operate(
                    &mut tree.children[0],
                    Layout::new(&node),
                    renderer,
                    &mut locate,
                );
                if let Some(target) = locate.bounds {
                    self.content.as_widget_mut().operate(
                        &mut tree.children[0],
                        Layout::new(&node),
                        renderer,
                        &mut Show(target),
                    );
                } else {
                    log::error!("[nbt-tree] 导航目标已不在当前树窗口，未移动到其他节点");
                }
            } else {
                self.content.as_widget_mut().operate(
                    &mut tree.children[0],
                    Layout::new(&node),
                    renderer,
                    &mut operation::scrollable::scroll_to::<()>(
                        widget::Id::new(SCROLL),
                        operation::scrollable::AbsoluteOffset {
                            x: Some(0.),
                            y: Some(0.),
                        },
                    ),
                );
            }
        }
        if let Some((_, input)) = focus_input {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                Layout::new(&node),
                renderer,
                &mut operation::focusable::focus::<()>(widget::Id::new(input.id())),
            );
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                Layout::new(&node),
                renderer,
                &mut operation::text_input::select_all::<()>(widget::Id::new(input.id())),
            );
        }
        let seen = tree.state.downcast_mut::<Seen>();
        seen.ticket = Some(self.ticket);
        seen.owner = Some(Arc::clone(&self.owner));
        seen.input = self.focus_input;
        node
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Action>,
        viewport: &Rectangle,
    ) {
        if !shell.is_event_captured()
            && self.focus_input.is_some()
            && matches!(
                event,
                Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                })
            )
        {
            shell.publish(self.focus_input.expect("存在活动行内输入").1.cancel());
            shell.capture_event();
            return;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Action, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
