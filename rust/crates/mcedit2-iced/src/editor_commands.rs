//! 原版编辑命令的公共文档路由；设置、快捷键和界面共用实际操作入口。
use super::Command;
use crate::{App, Message};
use iced::Task;

pub(crate) fn document_available(app: &App) -> bool {
    !app.desktop.show_recent
        && super::owner(app).is_some()
        && !(crate::structure_workspace::active(app).is_none()
            && app.show_advanced
            && matches!(
                app.advanced_section,
                crate::AdvancedSection::Files
                    | crate::AdvancedSection::Players
                    | crate::AdvancedSection::WorldInfo
            )
            && app
                .current
                .is_some_and(|world| app.nbt_documents.active(world).is_some()))
}

pub(super) fn dispatch(app: &mut App, command: Command) -> Option<Task<Message>> {
    use Command::*;
    if !matches!(
        command,
        DeleteBlocks
            | DeleteEntities
            | FillSelection
            | ReplaceBlocks
            | ShowLibrary
            | ExportStructure
            | ImportStructure
    ) && !super::tools::handles(command)
    {
        return None;
    }
    if app.shortcuts.open || app.block_picker.open() || app.workspace.closing.is_some() {
        return Some(Task::none());
    }
    if command == ShowLibrary {
        return Some(crate::library::update(app, crate::library::Action::Show));
    }
    if !document_available(app) {
        return Some(Task::none());
    }
    if let Some(task) = super::tools::dispatch(app, command) {
        return Some(task);
    }
    Some(match command {
        DeleteBlocks | DeleteEntities => crate::deletion_ui::update(
            app,
            crate::deletion_ui::Action::QuickMode(if command == DeleteBlocks {
                mcedit2_world::editing::deletion::Mode::Blocks
            } else {
                mcedit2_world::editing::deletion::Mode::Entities
            }),
        ),
        FillSelection => {
            if let Some(id) = crate::structure_workspace::active(app) {
                crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::Edit,
                    ),
                )
            } else {
                super::tools::show_edit(app, crate::editor_options::Tool::Selection)
            }
        }
        ReplaceBlocks => {
            crate::block_replacement_ui::update(app, crate::block_replacement_ui::Action::Show)
        }
        ExportStructure => {
            if let Some(id) = crate::structure_workspace::active(app) {
                crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Page(
                        crate::structure_workspace::Page::File,
                    ),
                )
            } else {
                crate::structure_exchange::update(
                    app,
                    crate::structure_exchange::Action::ShowExport,
                )
            }
        }
        ImportStructure => {
            if let Some(id) = crate::structure_workspace::active(app) {
                crate::structure_workspace::update(
                    app,
                    id,
                    crate::structure_workspace::Action::Import(
                        crate::structure_workspace::import::Action::Show,
                    ),
                )
            } else {
                crate::structure_exchange::update(
                    app,
                    crate::structure_exchange::Action::ShowImport,
                )
            }
        }
        _ => unreachable!("仅接收已声明的编辑命令"),
    })
}
