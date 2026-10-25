//! OS-native dialogs on background threads so iced stays responsive.
use crate::Message;
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
