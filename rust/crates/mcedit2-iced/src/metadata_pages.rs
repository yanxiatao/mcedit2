//! 玩家 / 世界属性入口选择真实文件，记住各世界玩家来源，不借用其他 NBT 标签。
use crate::{nbt_editor::Page, AdvancedSection, App, Message};
use iced::Task;
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct State {
    visible: Option<((usize, usize), AdvancedSection)>,
    players: BTreeMap<usize, u64>,
}
pub(crate) fn entering(app: &mut App, section: AdvancedSection) {
    app.metadata_pages.visible = app.current_key().map(|key| (key, section));
}
fn activate(app: &mut App, id: u64, section: AdvancedSection, page: Page) -> Task<Message> {
    let task = crate::nbt_documents::update(app, crate::nbt_documents::Action::Select(id));
    app.nbt_documents
        .files
        .get_mut(&id)
        .expect("已选择真实文件")
        .panel
        .as_mut()
        .expect("已加载专用属性文档")
        .page = page;
    app.show_advanced = true;
    app.advanced_section = section;
    Task::batch([
        task,
        crate::tab_bar::reveal(app, crate::tab_bar::Id::File(id)),
    ])
}
pub(crate) fn show(app: &mut App, section: AdvancedSection) -> Task<Message> {
    let Some(key) = app
        .current_key()
        .filter(|_| !app.desktop.show_recent && app.structures.active.is_none())
    else {
        app.nbt_status = "请切换到世界标签后打开属性".into();
        return Task::none();
    };
    app.nbt_documents.cancel_queued_open();
    app.show_advanced = true;
    app.advanced_section = section;
    entering(app, section);
    match section {
        AdvancedSection::Players => {
            let eligible = |id: u64| {
                app.nbt_documents.files.get(&id).is_some_and(|document| {
                    document.key.0 == key.0
                        && document
                            .panel
                            .as_ref()
                            .is_some_and(|panel| panel.player_prefix().is_some())
                })
            };
            let preferred = app
                .metadata_pages
                .players
                .get(&key.0)
                .copied()
                .filter(|id| eligible(*id));
            let active = app
                .nbt_documents
                .active(key.0)
                .map(|(id, _)| id)
                .filter(|id| eligible(*id));
            let existing = preferred
                .or(active)
                .or_else(|| app.nbt_documents.ordered(key.0).find(|id| eligible(*id)));
            if let Some(id) = existing {
                let panel = app.nbt_documents.files[&id]
                    .panel
                    .as_ref()
                    .expect("已核对玩家文档");
                let page = if panel.page == Page::World {
                    Page::Player
                } else {
                    panel.page
                };
                app.metadata_pages.players.insert(key.0, id);
                return activate(app, id, section, page);
            }
            crate::player_browser::show_first(app)
        }
        AdvancedSection::WorldInfo => {
            let root = &app.worlds[key.0].root;
            let path = root.join(if root.join("level.dat").is_file() {
                "level.dat"
            } else {
                "level.dat_old"
            });
            let existing = app.nbt_documents.ordered(key.0).find(|id| {
                app.nbt_documents.files[id]
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.world_file() && panel.source.path() == path)
            });
            match existing {
                Some(id) => activate(app, id, section, Page::World),
                None => crate::update_inner(app, Message::WorldInfo),
            }
        }
        _ => unreachable!("专用属性入口只有玩家与世界属性"),
    }
}
pub(crate) fn maintain(app: &mut App) -> Task<Message> {
    app.metadata_pages.players.retain(|world, id| {
        app.nbt_documents
            .files
            .get(id)
            .is_some_and(|document| document.key.0 == *world)
    });
    let next = app
        .current_key()
        .filter(|_| {
            app.show_advanced
                && !app.desktop.show_recent
                && app.structures.active.is_none()
                && matches!(
                    app.advanced_section,
                    AdvancedSection::Players | AdvancedSection::WorldInfo
                )
        })
        .map(|key| (key, app.advanced_section));
    if app.metadata_pages.visible != next {
        app.metadata_pages.visible = next;
        app.player_browser.cancel_first();
        if let Some((_, section)) = next {
            return show(app, section);
        }
    }
    if let Some((key, AdvancedSection::Players)) = next {
        if let Some((id, document)) = app.nbt_documents.active(key.0) {
            if document
                .panel
                .as_ref()
                .is_some_and(|panel| panel.player_prefix().is_some())
            {
                app.metadata_pages.players.insert(key.0, id);
            }
        }
        return crate::player_browser::maintain_catalog(app);
    }
    Task::none()
}
