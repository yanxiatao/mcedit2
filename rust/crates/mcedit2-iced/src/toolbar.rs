//! 原版工具与面板图标；标签模式使用同一桌面配置队列，动作复用现有命令和菜单。
use crate::{App, Message};
use iced::{
    widget::{container, image, row, text},
    Element,
};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Icon {
    Select,
    Brush,
    Clone,
    Move,
    Flood,
    Block,
    Entity,
    Chunk,
    Generate,
}
fn handle(icon: Icon) -> image::Handle {
    static ICONS: OnceLock<BTreeMap<Icon, image::Handle>> = OnceLock::new();
    ICONS
        .get_or_init(|| {
            let sources = [
                (Icon::Select, "select_blocks"),
                (Icon::Brush, "brush"),
                (Icon::Clone, "clone"),
                (Icon::Move, "move"),
                (Icon::Flood, "flood_fill"),
                (Icon::Block, "edit_block"),
                (Icon::Entity, "edit_entity"),
                (Icon::Chunk, "edit_chunk"),
                (Icon::Generate, "generate"),
            ];
            sources
                .into_iter()
                .map(|(icon, name)| {
                    let source = mcedit2_plugins::resources::toolbar_icon(name)
                        .expect("原生工具栏声明的资源必须存在");
                    let rgba = ::image::load_from_memory(source)
                        .expect("内嵌原版工具栏 PNG 必须有效")
                        .to_rgba8();
                    (
                        icon,
                        image::Handle::from_rgba(rgba.width(), rgba.height(), rgba.into_raw()),
                    )
                })
                .collect()
        })
        .get(&icon)
        .expect("每个工具栏图标必须有实际资源")
        .clone()
}
pub(crate) fn default_text() -> bool {
    true
}

pub(crate) fn content<'a>(app: &App, icon: Icon, label: &'a str) -> Element<'a, Message> {
    let picture = image(handle(icon))
        .width(18)
        .height(18)
        .filter_method(iced::advanced::image::FilterMethod::Nearest);
    if app.desktop.saved.toolbar_text {
        row![picture, text(label).size(13)]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into()
    } else {
        container(picture).padding([2, 4]).into()
    }
}
