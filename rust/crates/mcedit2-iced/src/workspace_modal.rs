//! 对话框和不可中断的保存等待独占输入，后台消息仍按正式任务流程完成。
use crate::{workspace_tabs::Action, Message};
use iced::{
    advanced::{layout, mouse, overlay, renderer, widget::Tree, Clipboard, Layout, Shell, Widget},
    Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
};
struct Barrier<'a> {
    content: Element<'a, Message>,
    cancel: Option<Message>,
    keyboard: Option<Box<dyn Fn(&iced::keyboard::Event) -> Option<Message> + 'a>>,
}
pub(super) fn block(content: Element<'_, Message>) -> Element<'_, Message> {
    block_with(content, Message::Workspace(Action::CancelClose))
}
pub(super) fn block_with(content: Element<'_, Message>, cancel: Message) -> Element<'_, Message> {
    Element::new(Barrier {
        content,
        cancel: Some(cancel),
        keyboard: None,
    })
}
/// 子控件优先消费文本及下拉框操作，剩余按键交给所属对话框。
pub(super) fn block_with_keys<'a>(
    content: Element<'a, Message>,
    cancel: Message,
    keyboard: impl Fn(&iced::keyboard::Event) -> Option<Message> + 'a,
) -> Element<'a, Message> {
    Element::new(Barrier {
        content,
        cancel: Some(cancel),
        keyboard: Some(Box::new(keyboard)),
    })
}
pub(super) fn wait(content: Element<'_, Message>) -> Element<'_, Message> {
    Element::new(Barrier {
        content,
        cancel: None,
        keyboard: None,
    })
}
impl Widget<Message, Theme, Renderer> for Barrier<'_> {
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
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
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
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if matches!(
            event,
            Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                ..
            })
        ) {
            if let Some(cancel) = &self.cancel {
                shell.publish(cancel.clone());
            }
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
        if !shell.is_event_captured() {
            if let (Some(keyboard), Event::Keyboard(event)) = (&self.keyboard, event) {
                if let Some(message) = keyboard(event) {
                    shell.publish(message);
                }
            }
        }
        if matches!(
            event,
            Event::Keyboard(_) | Event::Mouse(_) | Event::Touch(_)
        ) {
            shell.capture_event();
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let interaction = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );
        if interaction == mouse::Interaction::None {
            mouse::Interaction::Idle
        } else {
            interaction
        }
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
