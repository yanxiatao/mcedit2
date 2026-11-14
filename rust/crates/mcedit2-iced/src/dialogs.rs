//! 原生文件选择在后台执行；取消保持草稿，线程异常不伪装成取消。
use crate::{
    dialog_paths::{self, Choice, Kind},
    AdvancedFile, App, Message,
};
use iced::Task;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct Context {
    owner: Option<crate::keyboard_bindings::Owner>,
    opening: u64,
    welcome: bool,
}
impl Context {
    fn capture(app: &App) -> Self {
        Self {
            owner: crate::keyboard_bindings::owner(app),
            opening: app.open_nonce,
            welcome: app.desktop.show_recent,
        }
    }
    fn current(&self, app: &App) -> bool {
        self.owner == crate::keyboard_bindings::owner(app)
            && self.opening == app.open_nonce
            && self.welcome == app.desktop.show_recent
    }
}
#[derive(Debug, Clone)]
pub(crate) struct ResourceRequest {
    context: Context,
    source: u64,
    draft: String,
}
impl ResourceRequest {
    fn capture(app: &App) -> Self {
        Self {
            context: Context::capture(app),
            source: app.texture_load_nonce,
            draft: app.texture_path.clone(),
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct AdvancedRequest {
    context: Context,
    kind: AdvancedFile,
    draft: String,
    light: Option<crate::save_postprocess::Request>,
}
#[derive(Debug, Clone)]
pub(crate) enum Action {
    ResourceChosen(ResourceRequest, Option<PathBuf>),
    AdvancedChosen(AdvancedRequest, Option<PathBuf>),
}
fn advanced_path(app: &App, kind: AdvancedFile) -> Option<&str> {
    Some(match kind {
        AdvancedFile::BeBatch => &app.be_batch_path,
        AdvancedFile::BeJournal => &app.be_journal_path,
        AdvancedFile::EntitySnapshot => &app.entity_snapshot_path,
        AdvancedFile::BeSnapshot => &app.be_snapshot_path,
        AdvancedFile::Structure(format, output) => {
            return crate::structure_exchange::dialog_path(app, format, output)
        }
        AdvancedFile::RegionPlan => &app.region_batch_path,
        AdvancedFile::PluginManifest => &app.plugin_manifest_path,
        AdvancedFile::Patch => &app.patch_path,
        AdvancedFile::LightTraits => return crate::save_postprocess::traits_path(app),
        AdvancedFile::MapImage => {
            if app.current_key().is_none()
                || app.structures.active.is_some()
                || app.desktop.show_recent
            {
                return None;
            }
            app.map_editor.image_path()
        }
    })
}
fn changed(app: &mut App) -> Task<Message> {
    let message = "文件选择对应的文档、资源请求或路径参数已改变，未应用所选路径。";
    log::info!("[native-dialog] {message}");
    app.messages.push(message.into());
    Task::none()
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    match action {
        Action::ResourceChosen(_, None) | Action::AdvancedChosen(_, None) => (),
        Action::ResourceChosen(request, Some(path)) => {
            if !request.context.current(app)
                || request.source != app.texture_load_nonce
                || request.draft != app.texture_path
            {
                return changed(app);
            }
            app.texture_path = path.to_string_lossy().into_owned();
            return crate::update_inner(app, Message::LoadTextures);
        }
        Action::AdvancedChosen(request, Some(path)) => {
            if !request.context.current(app)
                || advanced_path(app, request.kind) != Some(request.draft.as_str())
                || (request.kind == AdvancedFile::LightTraits
                    && request.light != crate::save_postprocess::selection(app))
            {
                return changed(app);
            }
            let value = path.to_string_lossy().into_owned();
            match request.kind {
                AdvancedFile::BeBatch => app.be_batch_path = value,
                AdvancedFile::BeJournal => app.be_journal_path = value,
                AdvancedFile::EntitySnapshot => app.entity_snapshot_path = value,
                AdvancedFile::BeSnapshot => app.be_snapshot_path = value,
                AdvancedFile::Structure(format, output) => {
                    crate::structure_exchange::chosen(app, format, output, path)
                }
                AdvancedFile::RegionPlan => app.region_batch_path = value,
                AdvancedFile::PluginManifest => app.plugin_manifest_path = value,
                AdvancedFile::Patch => app.patch_path = value,
                AdvancedFile::LightTraits => return crate::save_postprocess::chosen(app, value),
                AdvancedFile::MapImage => {
                    return crate::map_editor::update(app, crate::map_editor::Action::Path(value))
                }
            }
        }
    }
    Task::none()
}

enum Mode {
    File,
    Folder,
    Save,
}
async fn select(dialog: rfd::FileDialog, mode: Mode) -> Option<std::path::PathBuf> {
    let (send, receive) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let selected = match mode {
            Mode::File => dialog.pick_file(),
            Mode::Folder => dialog.pick_folder(),
            Mode::Save => dialog.save_file(),
        };
        if send.send(selected).is_err() {
            log::debug!("[native-dialog] 文件选择接收端已关闭");
        }
    });
    receive.await.expect("原生文件选择线程未返回结果")
}
pub(crate) fn world_folder(app: &App) -> Task<Message> {
    let dialog = dialog_paths::start(app, Kind::WorldFolder, None, false).configure(
        rfd::FileDialog::new().set_title("MCEdit2 — 选择 Minecraft 存档目录 / Choose world folder"),
    );
    Task::perform(select(dialog, Mode::Folder), |path| {
        let choice = Choice::folder(Kind::WorldFolder, path.as_deref());
        choice.then(Message::WorldChosen(path))
    })
}
pub(crate) fn resource_file(app: &App) -> Task<Message> {
    let request = ResourceRequest::capture(app);
    let dialog = dialog_paths::start(
        app,
        Kind::Resources,
        Some(Path::new(&app.texture_path)),
        false,
    )
    .configure(rfd::FileDialog::new().add_filter("Resource packs", &["zip", "jar"]));
    Task::perform(select(dialog, Mode::File), move |path| {
        let choice = Choice::file(Kind::Resources, path.as_deref());
        choice.then(Message::NativeDialog(Action::ResourceChosen(
            request.clone(),
            path,
        )))
    })
}
pub(crate) fn resource_directory(app: &App) -> Task<Message> {
    let request = ResourceRequest::capture(app);
    let dialog = dialog_paths::start(app, Kind::Resources, None, false)
        .configure(rfd::FileDialog::new().set_title("MCEdit2 — 资源包目录 / Resource-pack folder"));
    Task::perform(select(dialog, Mode::Folder), move |path| {
        let choice = Choice::folder(Kind::Resources, path.as_deref());
        choice.then(Message::NativeDialog(Action::ResourceChosen(
            request.clone(),
            path,
        )))
    })
}
pub(crate) fn advanced_file(app: &mut App, kind: AdvancedFile, save: bool) -> Task<Message> {
    let role = match (kind, save) {
        (AdvancedFile::MapImage, false) => Kind::MapImageImport,
        (AdvancedFile::Structure(..), false) => Kind::StructureImport,
        (AdvancedFile::Structure(..), true) => Kind::StructureExport,
        (_, false) => Kind::OperationImport,
        (_, true) => Kind::OperationExport,
    };
    let Some(draft) = advanced_path(app, kind) else {
        return changed(app);
    };
    let request = AdvancedRequest {
        context: Context::capture(app),
        kind,
        draft: draft.into(),
        light: if kind == AdvancedFile::LightTraits {
            crate::save_postprocess::selection(app)
        } else {
            None
        },
    };
    let (title, suffixes) = kind.filter();
    let dialog = dialog_paths::start(app, role, Some(Path::new(draft.trim())), save).configure(
        rfd::FileDialog::new()
            .set_title(app.language.pick("选择文件路径", "Select file path"))
            .add_filter(title, suffixes),
    );
    Task::perform(
        select(dialog, if save { Mode::Save } else { Mode::File }),
        move |path| {
            let choice = Choice::file(role, path.as_deref());
            choice.then(Message::NativeDialog(Action::AdvancedChosen(
                request.clone(),
                path,
            )))
        },
    )
}
