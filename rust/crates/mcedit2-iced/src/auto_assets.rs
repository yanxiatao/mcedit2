//! Read-only discovery of an installed vanilla Minecraft client JAR.
//! Never infer that arbitrary resource packs are active, never download
//! assets, traverse a whole drive, or inspect an unrelated world folder.
use crate::{textures::TextureBank, Message};
use iced::Task;
use mcedit2_world::WorldSummary;
use std::path::{Path, PathBuf};

fn is_simple_version(value: &str) -> bool {
    value.len() <= 64
        && !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !value.contains("..")
}
/// Probe only exact conventional client locations under the save's
/// already-local launcher ancestors (PrismLauncher/MultiMC and vanilla).
pub(crate) fn discover(root: &Path, version: &str) -> Option<PathBuf> {
    if !is_simple_version(version) {
        return None;
    }
    for ancestor in root.ancestors().take(9) {
        let names = [
            ancestor
                .join("libraries")
                .join("com/mojang/minecraft")
                .join(version)
                .join(format!("minecraft-{version}-client.jar")),
            ancestor
                .join("libraries")
                .join("net/minecraft/client")
                .join(version)
                .join(format!("client-{version}.jar")),
            ancestor
                .join("versions")
                .join(version)
                .join(format!("{version}.jar")),
            ancestor
                .join(".minecraft/versions")
                .join(version)
                .join(format!("{version}.jar")),
        ];
        for candidate in names {
            // Skip symlinks and files that could be under a drive-wide
            // scan; none of these exact locations is writable by us.
            if candidate
                .symlink_metadata()
                .ok()
                .is_some_and(|m| m.is_file() && !m.file_type().is_symlink())
            {
                return Some(candidate);
            }
        }
    }
    None
}
pub(crate) fn load(index: usize, world: &WorldSummary) -> Task<Message> {
    let root = world.root.clone();
    let version = world.version_name.clone().unwrap_or_default();
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let result = discover(&root, &version)
                    .map(|path| TextureBank::open(&path).map(|bank| (path, bank)));
                let _ = tx.send(result);
            });
            (index, rx.await.unwrap_or(None))
        },
        |(index, result)| Message::AutoAssetsLoaded(index, result),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_version_path_cannot_escape_world_ancestors() {
        for value in ["../../other", "", "1.21.1/../evil", "1.21.1\\other"] {
            assert!(!is_simple_version(value));
        }
        assert!(is_simple_version("1.21.1"));
        assert!(is_simple_version("1.20.1-forge"));
    }
    #[test]
    fn no_installed_client_jar_returns_no_textures() {
        let root = std::env::temp_dir()
            .join(format!(
                "mcedit2-auto-assets-nonexistent-{}-test",
                std::process::id()
            ))
            .join("saves/test");
        assert!(discover(&root, "1.21.1").is_none());
    }
}
