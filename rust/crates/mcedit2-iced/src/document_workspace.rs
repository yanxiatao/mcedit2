//! 文件标签固定在顶部，专用表单、库存和 NBT 各自使用一页及独立滚动树。
use crate::nbt_editor::Page;
use crate::*;
use iced::widget::column;
pub(super) fn view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let mut header = column![text(app.advanced_section.label(l)).size(24)].spacing(7);
    if app.advanced_section == AdvancedSection::Players {
        header = header.push(app.player_browser.view(l, app.entity_models.online_skins));
    }
    if app.advanced_section == AdvancedSection::WorldInfo {
        header = header.push(
            button(l.pick("读取世界属性", "Load world properties")).on_press(Message::WorldInfo),
        );
    }
    if app.advanced_section == AdvancedSection::Files {
        header = header.push(
            row![
                text_input(
                    l.pick("世界内 .dat 或 .nbt 路径", "World .dat or .nbt path"),
                    &app.nbt_path
                )
                .on_input(Message::NbtPath)
                .on_submit(Message::NbtOpen)
                .width(Length::Fill),
                button(l.pick("打开", "Open")).on_press(Message::NbtOpen)
            ]
            .spacing(6),
        );
    }
    header = header.push(crate::nbt_documents::view(app));
    let mut body = column![].spacing(10).width(Length::Fill);
    if !app.nbt_status.is_empty() {
        body = body.push(text(&app.nbt_status));
    }
    if !app.metadata_refresh.status.is_empty() {
        body = body.push(text(&app.metadata_refresh.status));
    }
    if let Some(panel) = super::document(app) {
        let epoch = app.nbt_document_id();
        let mut tabs = row![].spacing(4);
        for page in panel.pages() {
            tabs = tabs.push(
                button(page.label(l))
                    .style(if panel.page == page {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press(Message::Nbt(epoch, nbt_editor::Action::Page(page))),
            );
        }
        header = header.push(tabs.wrap());
        header = header.push(
            row![
                button(l.pick("撤销", "Undo")).on_press_maybe(
                    (panel.source.document.undo_depth() > 0).then_some(Message::Nbt(
                        epoch,
                        nbt_editor::Action::Tree(nbt_tree::Action::Undo)
                    ))
                ),
                button(l.pick("重做", "Redo")).on_press_maybe(
                    (panel.source.document.redo_depth() > 0).then_some(Message::Nbt(
                        epoch,
                        nbt_editor::Action::Tree(nbt_tree::Action::Redo)
                    ))
                ),
                button(l.pick("准备保存", "Prepare save"))
                    .on_press(Message::Nbt(epoch, nbt_editor::Action::Prepare)),
                text(if panel.source.read_only() {
                    l.pick("只读", "Read only")
                } else {
                    l.pick("修改暂存于内存", "Changes stay in memory")
                })
            ]
            .spacing(5)
            .wrap(),
        );
        match panel.page {
            Page::World => {
                body = body.push(
                    world_info::view(&panel.source.document, &panel.typed)
                        .map(move |action| Message::Nbt(epoch, action)),
                )
            }
            Page::Player => {
                body = body.push(
                    player_editor::view(
                        &panel.source.document,
                        &panel.typed,
                        panel.player_prefix().expect("玩家页具有玩家来源"),
                    )
                    .map(move |action| Message::Nbt(epoch, action)),
                )
            }
            Page::Inventory | Page::Ender => {
                let prefix = panel.player_prefix().expect("库存页具有玩家来源");
                let (name, title, layout) = if panel.page == Page::Inventory {
                    (
                        "Inventory",
                        l.pick("玩家背包与装备", "Player inventory & equipment"),
                        mcedit2_world::inventory::Layout::Player,
                    )
                } else {
                    (
                        "EnderItems",
                        l.pick("末影箱", "Ender chest"),
                        mcedit2_world::inventory::Layout::Ender,
                    )
                };
                body = body.push(
                    inventory_ui::view(
                        &panel.inventory,
                        &panel.source.document,
                        inventory_ui::spec(
                            format!("{prefix}/{name}"),
                            layout,
                            panel.source.world_root(),
                            panel.player_data_version(),
                        ),
                        title,
                        app.texture_bank.as_deref(),
                        l,
                        app.entity_models.online_skins,
                        app.item_clipboard.clone(),
                        panel.inventory_editable(),
                        &app.drag_drop,
                    )
                    .map(move |action| Message::Nbt(epoch, nbt_editor::Action::Inventory(action))),
                );
            }
            Page::Nbt => {
                body = body.push(quicklinks(app, panel));
                body = body.push(
                    panel
                        .tree_view()
                        .map(move |action| Message::Nbt(epoch, action)),
                );
            }
        }
        body = body.push(
            panel
                .save_view()
                .map(move |action| Message::Nbt(epoch, action)),
        );
    }
    body = body.push(
        iced::widget::checkbox(app.nbt_editable)
            .label(l.pick(
                "下次打开允许准备写入（默认只读）",
                "Allow preparing writes on next open (read only by default)",
            ))
            .on_toggle(Message::NbtEditable),
    );
    let key = (
        app.nbt_documents
            .active(app.current.expect("文档工作区所属世界"))
            .map(|(id, _)| id),
        app.nbt_document().map(|p| p.page),
    );
    header
        .push(
            iced::widget::keyed_column([(
                key,
                scrollable(body)
                    .id(iced::advanced::widget::Id::new(
                        workspace_ui::DOCUMENT_SCROLL,
                    ))
                    .direction(iced::widget::scrollable::Direction::Vertical(
                        iced::widget::scrollable::Scrollbar::new().spacing(8),
                    ))
                    .height(Length::Fill)
                    .width(Length::Fill)
                    .into(),
            )])
            .height(Length::Fill)
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
}
fn quicklinks<'a>(app: &'a App, panel: &'a nbt_editor::Panel) -> Element<'a, Message> {
    let mut body = row![text(app.language.pick("字段捷径", "Field shortcuts"))].spacing(5);
    let epoch = app.nbt_document_id();
    if panel.world_file() {
        for (name, target) in [
            ("名称", "/Data/LevelName"),
            ("出生 X", "/Data/SpawnX"),
            ("出生 Y", "/Data/SpawnY"),
            ("出生 Z", "/Data/SpawnZ"),
            ("世界时间", "/Data/Time"),
            ("日间时间", "/Data/DayTime"),
            ("难度", "/Data/Difficulty"),
            ("锁定难度", "/Data/DifficultyLocked"),
            ("游戏模式", "/Data/GameType"),
            ("极限模式", "/Data/hardcore"),
            ("允许命令", "/Data/allowCommands"),
            ("游戏规则", "/Data/GameRules"),
            ("旧版种子", "/Data/RandomSeed"),
            ("旧版生成器", "/Data/generatorName"),
            ("旧版生成选项", "/Data/generatorOptions"),
            ("世界生成设置", "/Data/WorldGenSettings"),
            ("现代种子", "/Data/WorldGenSettings/seed"),
            (
                "现代主世界生成器",
                "/Data/WorldGenSettings/dimensions/minecraft:overworld/generator",
            ),
        ] {
            if [
                "/Data/GameRules",
                "/Data/WorldGenSettings",
                "/Data/WorldGenSettings/dimensions/minecraft:overworld/generator",
            ]
            .contains(&target)
            {
                body = body.push(button(name).on_press(Message::Nbt(
                    epoch,
                    nbt_editor::Action::Tree(crate::nbt_tree::Action::Select(target.into(), 10)),
                )));
            } else if let Ok(value) = panel.source.document.inspect(target) {
                let label = name.to_string();
                body = body.push(button(text(label)).on_press(Message::Nbt(
                    epoch,
                    nbt_editor::Action::Tree(crate::nbt_tree::Action::Select(
                        target.into(),
                        value.kind(),
                    )),
                )));
            }
        }
    }
    if let Some(prefix) = panel.player_prefix() {
        for (name, field) in [
            ("位置", "Pos"),
            ("朝向", "Rotation"),
            ("维度", "Dimension"),
            ("背包", "Inventory"),
            ("末影箱", "EnderItems"),
            ("生命", "Health"),
            ("饥饿", "foodLevel"),
            ("经验等级", "XpLevel"),
            ("经验进度", "XpP"),
            ("玩家游戏模式", "playerGameType"),
            ("能力", "abilities"),
            ("属性", "attributes"),
        ] {
            let target = format!("{prefix}/{field}");
            if ["Inventory", "EnderItems", "abilities", "attributes"].contains(&field) {
                let kind = if field == "abilities" { 10 } else { 9 };
                body = body.push(button(name).on_press(Message::Nbt(
                    epoch,
                    nbt_editor::Action::Tree(crate::nbt_tree::Action::Select(target, kind)),
                )));
            } else if let Ok(value) = panel.source.document.inspect(&target) {
                let label = name.to_string();
                body = body.push(button(text(label)).on_press(Message::Nbt(
                    epoch,
                    nbt_editor::Action::Tree(crate::nbt_tree::Action::Select(target, value.kind())),
                )));
            }
        }
    }
    body.wrap().into()
}
