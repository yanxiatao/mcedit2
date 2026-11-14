//! 槽位真实排列及命中区域；内容由库存面板提供，不把空白区域变成按钮。
use iced::{Element, Length, Padding};
use mcedit2_world::inventory::Layout;
pub(crate) fn extent(layout: Layout) -> (f32, f32) {
    layout
        .positions()
        .iter()
        .fold((0f32, 0f32), |(width, height), (_, x, y)| {
            (
                (width).max(*x as f32 * 44. / 18. + 42.),
                height.max(*y as f32 * 70. / 18. + 68.),
            )
        })
}
pub(crate) fn view<'a, M: 'a, R: iced::advanced::Renderer + 'a>(
    layout: Layout,
    mut cell: impl FnMut(i16) -> Element<'a, M, iced::Theme, R>,
) -> Element<'a, M, iced::Theme, R> {
    let (width, height) = extent(layout);
    let cells = layout
        .positions()
        .into_iter()
        .map(|(slot, x, y)| {
            iced::widget::container(cell(slot))
                .padding(Padding {
                    top: y as f32 * 70. / 18.,
                    left: x as f32 * 44. / 18.,
                    right: 0.,
                    bottom: 0.,
                })
                .into()
        })
        .collect::<Vec<_>>();
    iced::widget::stack(cells)
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .into()
}

#[cfg(test)]
pub(crate) fn verify() {
    use iced::{
        mouse,
        widget::{button, Space},
        Event, Point, Size,
    };
    use iced_runtime::{user_interface::Cache, UserInterface};
    for layout in [
        Layout::Player,
        Layout::Ender,
        Layout::Chest,
        Layout::Dispenser,
        Layout::Hopper,
        Layout::Furnace,
        Layout::Brewing,
        Layout::Bookshelf,
        Layout::Crafter,
        Layout::Campfire,
        Layout::Jukebox,
        Layout::Lectern,
        Layout::Pot,
        Layout::Brushable,
    ] {
        let content: Element<'_, i16, iced::Theme, ()> = view(layout, |slot| {
            button(Space::new())
                .padding(0)
                .width(42)
                .height(68)
                .on_press(slot)
                .into()
        });
        let mut ui = UserInterface::build(content, Size::new(800., 900.), Cache::new(), &mut ());
        for (slot, x, y) in layout.positions() {
            let point = Point::new(x as f32 * 44. / 18. + 21., y as f32 * 70. / 18. + 34.);
            let mut messages = Vec::new();
            ui.update(
                &[
                    Event::Mouse(mouse::Event::CursorMoved { position: point }),
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                ],
                mouse::Cursor::Available(point),
                &mut (),
                &mut iced::advanced::clipboard::Null,
                &mut messages,
            );
            assert_eq!(messages, vec![slot], "{layout:?} 槽 {slot} 命中错误");
        }
        if layout == Layout::Furnace {
            let point = Point::new(100., 34.);
            let mut messages = Vec::new();
            ui.update(
                &[
                    Event::Mouse(mouse::Event::CursorMoved { position: point }),
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                ],
                mouse::Cursor::Available(point),
                &mut (),
                &mut iced::advanced::clipboard::Null,
                &mut messages,
            );
            assert!(messages.is_empty(), "容器空白不能捕获为槽位");
        }
    }
}
