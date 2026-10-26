//! OS-native dialogs on background threads so iced stays responsive.
use crate::{AdvancedFile, Message};
use crate::language::Language;
use iced::Task;
pub(crate) fn world_folder() -> Task<Message> {
    Task::perform(
        async {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let v = rfd::FileDialog::new()
                    .set_title("MCEdit2 — 选择 Minecraft 存档目录 / Choose world folder")
                    .pick_folder();
                let _ = tx.send(v);
            });
            rx.await.unwrap_or(None)
        },
        Message::WorldChosen,
    )
}
pub(crate) fn resource_file() -> Task<Message> {
    Task::perform(
        async {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let v = rfd::FileDialog::new()
                    .add_filter("Resource packs", &["zip", "jar"])
                    .pick_file();
                let _ = tx.send(v);
            });
            rx.await.unwrap_or(None)
        },
        Message::TextureChosen,
    )
}
pub(crate) fn resource_directory() -> Task<Message> {
    Task::perform(
        async {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let v = rfd::FileDialog::new()
                    .set_title("MCEdit2 — 资源包目录 / Resource-pack folder")
                    .pick_folder();
                let _ = tx.send(v);
            });
            rx.await.unwrap_or(None)
        },
        Message::TextureChosen,
    )
}

/// Native dialog for advanced import/export paths, executed off the iced UI
/// thread. Cancelling never mutates the previously typed path.
pub(crate) fn advanced_file(kind: AdvancedFile, save: bool, language: Language) -> Task<Message> {
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let (title, suffixes) = kind.filter();
                let dialog = rfd::FileDialog::new()
                    .set_title(language.pick("选择文件路径", "Select file path"))
                    .add_filter(title, suffixes);
                let selected = if save { dialog.save_file() } else { dialog.pick_file() };
                let _ = tx.send(selected);
            });
            (kind, rx.await.unwrap_or(None))
        },
        |(kind, path)| Message::AdvancedFileChosen(kind, path),
    )
}
