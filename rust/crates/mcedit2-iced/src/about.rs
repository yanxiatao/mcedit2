//! 原版关于窗口：真实构建信息、原项目鸣谢和随程序内嵌的许可全文。
use crate::{App, Message};
use iced::{
    widget::{button, column, container, image, row, scrollable, stack, text},
    Element, Length, Task,
};
use std::sync::OnceLock;

const BODY: &str = "mcedit2-about-body";
const ORIGINAL_LICENSE: &str = include_str!("../../../../LICENSE.md");
// 来源：7c13c16^:src/mcedit2/editorapp.py，MCEditApp.showAbout 的原始鸣谢名单。
const SUPPORTERS: &str = "Saurik Works\nGamer2313\nAndrew Devillez\nAlek Poyato\nJosh Mann\nNodeCraft Hosting\nDrew L\nCapt_World\nAdrian Brightmoore\nMarcel C\nTim G\nOwen C\nJulian C\nAusstan L\nLeonard P\nGregory M\nJoseph P\nLance R\nJohn B\nAaron J\nA.M.P.\nDaniel B\nZachary B\nGeoffrey C\nDiane W\nKyle H\nNathan M\nRoss C\nThomas H\nJordan S\nMicael L\nTodd A\nJohn C\nElisabeth F\nChris L\nS Spurlock\nPaul H\nJack T";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    Guide,
    #[default]
    About,
    Credits,
    License,
}
impl Page {
    const ALL: [Self; 4] = [Self::Guide, Self::About, Self::Credits, Self::License];
    fn label(self, language: crate::language::Language) -> &'static str {
        match self {
            Self::Guide => language.pick("操作指南", "Quick start"),
            Self::About => language.pick("关于", "About"),
            Self::Credits => language.pick("鸣谢", "Credits"),
            Self::License => language.pick("原项目许可", "Original license"),
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
    page: Page,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Open,
    Guide,
    Close,
    Page(Page),
    Step(bool),
    Copy,
    Keys,
}
fn message(action: Action) -> Message {
    Message::About(action)
}
fn build_info() -> String {
    format!(
        "MCEdit2 Rust {}\n{} / {}\n{}\nCargo license: {}\nRust · iced · wgpu · RustPython",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        env!("CARGO_PKG_LICENSE")
    )
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::Open | Action::Guide => {
            app.about.open = true;
            crate::keyboard_bindings::interrupt(app);
            app.workspace.main_menu = None;
            app.workspace.menu = None;
            app.about.page = if action == Action::Guide {
                Page::Guide
            } else {
                Page::About
            };
        }
        Action::Close => app.about.open = false,
        Action::Page(page) => app.about.page = page,
        Action::Step(forward) => {
            let at = Page::ALL
                .iter()
                .position(|page| *page == app.about.page)
                .expect("关于页来自固定目录");
            app.about.page =
                Page::ALL[(at + if forward { 1 } else { Page::ALL.len() - 1 }) % Page::ALL.len()];
        }
        Action::Copy => return iced::clipboard::write(build_info()),
        Action::Keys => {
            app.about.open = false;
            return crate::keyboard_bindings::update(app, crate::keyboard_bindings::Action::Open);
        }
    }
    if app.about.open {
        crate::workspace_ui::scroll_start(BODY)
    } else {
        Task::none()
    }
}
pub(crate) fn keyboard(
    event: iced::Event,
    _: iced::event::Status,
    _: iced::window::Id,
) -> Option<Message> {
    use iced::keyboard::{key::Named, Event, Key};
    match event {
        iced::Event::Keyboard(Event::KeyPressed {
            key: Key::Named(key),
            modifiers,
            repeat: false,
            ..
        }) if modifiers.is_empty() => match key {
            Named::Enter => Some(message(Action::Close)),
            Named::ArrowLeft => Some(message(Action::Step(false))),
            Named::ArrowRight => Some(message(Action::Step(true))),
            _ => None,
        },
        _ => None,
    }
}
fn icons() -> &'static (iced::widget::image::Handle, iced::window::Icon) {
    static ICON: OnceLock<(iced::widget::image::Handle, iced::window::Icon)> = OnceLock::new();
    ICON.get_or_init(|| {
        let image = ::image::load_from_memory(include_bytes!("../assets/mcediticon.png"))
            .expect("内嵌原版应用图标必须有效")
            .to_rgba8();
        let window = iced::window::icon::from_rgba(image.to_vec(), image.width(), image.height())
            .expect("应用图标 RGBA 尺寸必须有效");
        (
            iced::widget::image::Handle::from_rgba(image.width(), image.height(), image.into_raw()),
            window,
        )
    })
}
pub(crate) fn window_icon() -> iced::window::Icon {
    icons().1.clone()
}
pub(crate) fn modal<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    if !app.about.open {
        return base;
    }
    let language = app.language;
    let mut tabs = row![].spacing(8);
    for page in Page::ALL {
        tabs = tabs.push(
            button(page.label(language))
                .style(if app.about.page == page {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(message(Action::Page(page))),
        );
    }
    let content: Element<'_, Message> = match app.about.page {
        Page::Guide => guide(app),
        Page::About => column![
            text(language.pick(
                "Minecraft Java 世界与结构编辑器",
                "Minecraft Java world and structure editor"
            ))
            .size(20),
            text(language.pick(
                "原生 Rust 重写，源自 MCEdit 与 MCEditLib。",
                "A native Rust rewrite based on MCEdit and MCEditLib."
            )),
            text("David Rio Vierra · MCEdit / MCEditLib"),
            text(build_info()),
        ]
        .spacing(14)
        .into(),
        Page::Credits => column![
            text(language.pick("原项目支持者", "Original project supporters")).size(20),
            text(SUPPORTERS),
            text(language.pick("原版使用的技术", "Original technologies")).size(18),
            text("Python · Qt · PySide · PyOpenGL · NumPy · Cython · PyCharm"),
            text(language.pick(
                "原项目的图标与工具栏素材保留其作者和许可。",
                "Original application and toolbar artwork retains its authorship and license."
            )),
        ]
        .spacing(12)
        .into(),
        Page::License => column![
            text(language.pick(
                "MCEdit / MCEditLib 原项目许可",
                "Original MCEdit / MCEditLib license"
            ))
            .size(20),
            text(ORIGINAL_LICENSE).wrapping(iced::widget::text::Wrapping::WordOrGlyph),
        ]
        .spacing(12)
        .into(),
    };
    let dialog = container(
        column![
            row![
                image(icons().0.clone()).width(48).height(48),
                text(if app.about.page == Page::Guide {
                    language
                        .pick("MCEdit2 操作指南", "MCEdit2 Quick start")
                        .to_owned()
                } else {
                    format!("MCEdit2 Rust {}", env!("CARGO_PKG_VERSION"))
                })
                .size(25)
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            tabs.wrap(),
            scrollable(content)
                .id(BODY)
                .direction(iced::widget::scrollable::Direction::Vertical(
                    iced::widget::scrollable::Scrollbar::new().spacing(12)
                ))
                .height(Length::Fill)
                .width(Length::Fill),
            row![
                if app.about.page == Page::Guide {
                    button(language.pick("按键与导航设置…", "Keyboard & navigation…"))
                        .style(button::secondary)
                        .on_press(message(Action::Keys))
                } else {
                    button(language.pick("复制版本信息", "Copy version information"))
                        .on_press(message(Action::Copy))
                },
                button(language.pick("关闭", "Close")).on_press(message(Action::Close))
            ]
            .spacing(12)
        ]
        .spacing(16)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .padding(20)
    .max_width(720)
    .max_height(600)
    .width(Length::Fill)
    .height(Length::Fill)
    .style(container::rounded_box);
    let shade = container(dialog)
        .padding(24)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_| container::Style {
            background: Some(iced::Color::from_rgba(0., 0., 0., 0.72).into()),
            ..Default::default()
        });
    stack![
        base,
        crate::workspace_tabs::block_dialog(shade.into(), message(Action::Close))
    ]
    .into()
}

/// 指南展示当前实际绑定，修改按键后不会继续提示旧快捷键。
fn guide(app: &App) -> Element<'_, Message> {
    use crate::keyboard_bindings::Command;
    let l = app.language;
    let mut body = column![
        text(l.pick("从打开存档到保存修改", "From opening a world to saving changes")).size(21),
        text(l.pick("1  打开文档", "1  Open a document")).size(17),
        text(l.pick("打开包含 level.dat 的世界文件夹，或直接打开结构 / NBT 文件。顶部标签用于切换文档。", "Open a world folder containing level.dat, or a structure / NBT file. Switch documents using the tabs at the top.")),
        text(l.pick("2  选择工具并修改", "2  Choose a tool and edit")).size(17),
        text(l.pick("工具栏选择选框、笔刷、克隆等操作，侧栏设置参数。更多功能可在“工具与文件”中按用途搜索。", "Choose selection, brush, clone or another tool from the toolbar. Set options in the side panel, or search Tools & files for more tasks.")),
        text(l.pick("3  复查并保存", "3  Review and save")).size(17),
        text(l.pick("预览用于检查操作结果；“暂存”表示修改已进入当前文档，但尚未写入存档。确认保存后才写入磁盘；世界保存页还提供备份和未完成事务的处理入口。", "A preview lets you review a result. Staged changes belong to the current document but have not been written to disk. Confirm Save to write them. The world Save page also provides backups and interrupted-save recovery.")),
        text(l.pick("常用快捷键", "Common shortcuts")).size(17),
    ].spacing(12);
    for (command, zh, en) in [
        (Command::Open, "打开文档", "Open document"),
        (Command::Save, "保存当前文档", "Save current document"),
        (Command::Undo, "撤销", "Undo"),
        (Command::Redo, "重做", "Redo"),
        (Command::ToolSelect, "选框工具", "Selection tool"),
        (Command::Copy, "复制选区", "Copy selection"),
        (Command::Paste, "粘贴", "Paste"),
        (Command::TogglePanel, "显示 / 隐藏侧栏", "Show / hide panel"),
    ] {
        let binding = app.shortcuts.hint(command);
        body = body.push(
            row![
                text(l.pick(zh, en)).width(Length::Fill),
                text(if binding.is_empty() {
                    l.pick("未绑定", "Unbound").to_owned()
                } else {
                    binding
                })
            ]
            .spacing(16),
        );
    }
    body = body.push(text(l.pick("画布导航（当前视图）", "Scene navigation (current view)")).size(17))
        .push(text(crate::navigation_bindings::hint(app, app.active_view())))
        .push(text(l.pick("顶部视图按钮切换自由相机、等轴、俯视、切片和四视图。侧栏标题可拖动停靠，分隔条可调整宽度。", "Use the view buttons for free camera, isometric, overhead, slices or four views. Drag the panel title to dock it, or its divider to resize.")))
        .push(button(l.pick("按键与导航设置…", "Keyboard & navigation settings…"))
            .on_press(message(Action::Keys)));
    body.into()
}
