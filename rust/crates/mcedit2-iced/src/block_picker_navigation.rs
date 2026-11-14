//! 方块选择器的列表导航；输入框及下拉列表消费按键后才到达这里。
use super::{icons, select, Action, Panel, PAGE, RESULTS, ROW_HEIGHT, SEARCH};
use crate::Message;
use iced::{
    keyboard::{key::Named, Event, Key},
    Task,
};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Direction {
    Previous,
    Next,
    PagePrevious,
    PageNext,
    First,
    Last,
    Toggle,
    Search,
}

pub(super) fn key(epoch: u64, event: &Event) -> Option<Message> {
    let Event::KeyPressed {
        key,
        modifiers,
        repeat,
        ..
    } = event
    else {
        return None;
    };
    if modifiers.control()
        && !modifiers.shift()
        && !modifiers.alt()
        && !modifiers.logo()
        && !repeat
        && matches!(key, Key::Character(s) if s.eq_ignore_ascii_case("f"))
    {
        return Some(Message::BlockPicker(Action::Navigate(
            epoch,
            Direction::Search,
        )));
    }
    if !modifiers.is_empty() {
        return None;
    }
    let direction = match key {
        Key::Named(Named::Enter) if !repeat => {
            return Some(Message::BlockPicker(Action::Apply(epoch)));
        }
        Key::Named(Named::ArrowUp) => Direction::Previous,
        Key::Named(Named::ArrowDown) => Direction::Next,
        Key::Named(Named::PageUp) => Direction::PagePrevious,
        Key::Named(Named::PageDown) => Direction::PageNext,
        Key::Named(Named::Home) => Direction::First,
        Key::Named(Named::End) => Direction::Last,
        Key::Named(Named::Space) if !repeat => Direction::Toggle,
        _ => return None,
    };
    Some(Message::BlockPicker(Action::Navigate(epoch, direction)))
}

pub(super) fn scroll(position: usize) -> Task<Message> {
    // 条目有固定高度，跨页和键盘移动使用同一实际列表偏移。
    iced::widget::operation::scroll_to(
        RESULTS,
        iced::widget::operation::AbsoluteOffset {
            x: None,
            y: Some((position % PAGE) as f32 * (ROW_HEIGHT + 3.)),
        },
    )
}

pub(super) fn list_focus() -> Task<Message> {
    iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
}

pub(super) fn refresh(panel: &mut Panel) -> Task<Message> {
    let position = panel.catalog.as_ref().and_then(|catalog| {
        panel.focused.as_ref().and_then(|focused| {
            panel
                .order
                .iter()
                .position(|&index| &catalog.rows[index].state == focused)
        })
    });
    let position = position.unwrap_or(0);
    panel.page = position / PAGE;
    Task::batch([icons(panel), scroll(position)])
}

pub(super) fn page(panel: &mut Panel, page: usize) -> Result<Task<Message>, String> {
    let catalog = panel.catalog.as_ref().ok_or("请等待读取方块目录")?;
    panel.page = page.min(panel.order.len().saturating_sub(1) / PAGE);
    if let Some(&index) = panel.order.get(panel.page * PAGE) {
        if panel.target.multiple() {
            panel.focused = Some(catalog.rows[index].state.clone());
        } else {
            select(panel, index, false)?;
        }
    }
    Ok(Task::batch([icons(panel), scroll(0), list_focus()]))
}

pub(super) fn navigate(panel: &mut Panel, direction: Direction) -> Result<Task<Message>, String> {
    if matches!(direction, Direction::Search) {
        return Ok(iced::widget::operation::focus(SEARCH));
    }
    let catalog = panel.catalog.as_ref().ok_or("请等待读取方块目录")?;
    let position = panel.focused.as_ref().and_then(|focused| {
        panel
            .order
            .iter()
            .position(|&index| &catalog.rows[index].state == focused)
    });
    if matches!(direction, Direction::Toggle) {
        if let Some(position) = position {
            let index = panel.order[position];
            let multiple = panel.target.multiple();
            select(panel, index, multiple)?;
        }
        return Ok(Task::none());
    }
    if panel.order.is_empty() {
        return Ok(Task::none());
    }
    let last = panel.order.len() - 1;
    let next = match (position, direction) {
        (_, Direction::First) => 0,
        (_, Direction::Last) => last,
        (None, _) => 0,
        (Some(i), Direction::Previous) => i.saturating_sub(1),
        (Some(i), Direction::Next) => (i + 1).min(last),
        (Some(i), Direction::PagePrevious) => i.saturating_sub(PAGE),
        (Some(i), Direction::PageNext) => (i + PAGE).min(last),
        (_, Direction::Toggle | Direction::Search) => unreachable!(),
    };
    let index = panel.order[next];
    if panel.target.multiple() {
        panel.focused = Some(catalog.rows[index].state.clone());
    } else {
        select(panel, index, false)?;
    }
    let changed_page = panel.page != next / PAGE;
    panel.page = next / PAGE;
    Ok(if changed_page {
        Task::batch([icons(panel), scroll(next)])
    } else {
        scroll(next)
    })
}
