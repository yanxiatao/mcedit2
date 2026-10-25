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
/// Probe exact, read-only launcher paths without filesystem-wide discovery.
/// Candidate order matters: prefer the launcher colocated with the world
/// over a different globally installed copy of the same Minecraft version.
fn candidate_paths(base: &Path, version: &str) -> [PathBuf; 4] {
    [
        base.join("libraries/com/mojang/minecraft")
            .join(version)
            .join(format!("minecraft-{version}-client.jar")),
        base.join("libraries/net/minecraft/client")
            .join(version)
            .join(format!("client-{version}.jar")),
        base.join("versions")
            .join(version)
            .join(format!("{version}.jar")),
        base.join(".minecraft/versions")
            .join(version)
            .join(format!("{version}.jar")),
    ]
}
fn locate_in<'a>(bases: impl IntoIterator<Item = &'a Path>, version: &str) -> Option<PathBuf> {
    if !is_simple_version(version) {
        return None;
    }
    for base in bases {
        for candidate in candidate_paths(base, version) {
            // Never follow a symlink to a client JAR outside this install.
            // The ZIP reader is read-only; no unpacking or client modification.
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
/// Search the original world's launcher first, then a bounded set of
/// platform-standard vanilla/PrismLauncher/MultiMC installation roots.
/// This matters when users keep world saves on another drive or import a
/// downloaded world into the editor without moving the Minecraft client.
pub(crate) fn discover(root: &Path, version: &str) -> Option<PathBuf> {
    if !is_simple_version(version) {
        return None;
    }
    let mut bases = root
        .ancestors()
        .take(9)
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let appdata = PathBuf::from(appdata);
        bases.push(appdata.clone());
        bases.push(appdata.join(".minecraft"));
        bases.push(appdata.join("PrismLauncher"));
        bases.push(appdata.join("MultiMC"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        bases.push(home.join(".minecraft"));
        bases.push(home.join(".local/share/PrismLauncher"));
        bases.push(home.join(".local/share/multimc"));
        bases.push(home.join(".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher"));
        bases.push(home.join("Library/Application Support/minecraft"));
        bases.push(home.join("Library/Application Support/PrismLauncher"));
    }
    locate_in(bases.iter().map(PathBuf::as_path), version)
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
    fn exact_launcher_paths_support_prism_and_vanilla_without_traversal() {
        let base = Path::new("/launcher");
        let paths = candidate_paths(base, "1.21.1");
        assert_eq!(
            paths[0],
            base.join("libraries/com/mojang/minecraft/1.21.1/minecraft-1.21.1-client.jar")
        );
        assert_eq!(paths[2], base.join("versions/1.21.1/1.21.1.jar"));
        assert_eq!(paths[3], base.join(".minecraft/versions/1.21.1/1.21.1.jar"));
        assert_eq!(locate_in([base], "../unsafe"), None);
    }
    #[test]
    fn world_local_launcher_takes_priority_over_global_installs() {
        let root = std::env::temp_dir().join(format!(
            "mcedit2-auto-asset-precedence-{}-{}",
            std::process::id(),
            "local-before-global"
        ));
        let _ = std::fs::remove_dir_all(&root);
        let first = root.join("instance");
        let second = root.join("global");
        let version = "9.9.9-test";
        for base in [&first, &second] {
            let jar = candidate_paths(base, version)[2].clone();
            std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
            std::fs::write(jar, b"placeholder").unwrap();
        }
        let expected = candidate_paths(&first, version)[2].clone();
        assert_eq!(
            locate_in([first.as_path(), second.as_path()], version),
            Some(expected)
        );
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn missing_local_install_does_not_guess_a_nonexistent_jar() {
        let root = std::env::temp_dir().join(format!(
            "mcedit2-auto-assets-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        assert!(locate_in([root.as_path()], "9.9.9-absent").is_none());
    }
}
