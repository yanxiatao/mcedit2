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
/// One detected source stack. The order is low -> high priority; the
/// final user-selected overlays are appended in the UI worker.
#[derive(Debug, Clone)]
pub(crate) struct AssetStack {
    pub paths: Vec<PathBuf>,
    pub instance: Option<PathBuf>,
    pub vanilla: bool,
    pub mod_count: usize,
    pub selected_count: usize,
    pub generated_count: usize,
    pub warnings: Vec<String>,
}
const MAX_MOD_JARS: usize = 768;
const MAX_SELECTED_PACKS: usize = 128;
const MAX_ASSET_LAYERS: usize = 1024;

/// Recognize an instance from the selected world's actual saves/ directory.
/// Never pick mods from a different nearby instance or scan sibling worlds.
fn game_directory(world: &Path) -> Option<PathBuf> {
    let saves = world.parent()?;
    if !saves.file_name()?.to_string_lossy().eq_ignore_ascii_case("saves") {
        return None;
    }
    Some(saves.parent()?.to_path_buf())
}
pub(crate) fn same_instance(a: &Path, b: &Path) -> bool {
    let left = game_directory(a);
    left.is_some() && left == game_directory(b)
}
fn ordinary_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).ok().is_some_and(|m|
        m.is_dir() && !m.file_type().is_symlink())
}
fn ordinary_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).ok().is_some_and(|m|
        m.is_file() && !m.file_type().is_symlink())
}
fn has_assets(path: &Path) -> bool {
    ordinary_directory(path)
        && ordinary_directory(&path.join("assets"))
}
fn is_zip_file(path: &Path) -> bool {
    ordinary_file(path)
        && path.extension().and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("zip"))
}
fn mod_archive_contains_assets(path: &Path) -> bool {
    use std::fs::File;
    if !ordinary_file(path)
        || path.extension().and_then(|v| v.to_str())
            .is_none_or(|v| !v.eq_ignore_ascii_case("jar"))
        || std::fs::metadata(path).is_ok_and(|m| m.len() > 1024 * 1024 * 1024)
    {
        return false;
    }
    let Ok(file) = File::open(path) else { return false };
    let Ok(archive) = zip::ZipArchive::new(file) else { return false };
    // No ZIP member is extracted or decompressed in discovery. Require a
    // namespace asset subtree rather than adding a library JAR as a pack.
    archive.file_names().any(|name| {
        let mut parts = name.split('/');
        matches!(parts.next(), Some("assets"))
            && parts.next().is_some_and(crate::resource_ids::namespace)
            && matches!(parts.next(), Some("textures" | "models" | "blockstates"))
    })
}
fn selected_pack_names(game: &Path, warnings: &mut Vec<String>) -> Vec<String> {
    let settings = game.join("options.txt");
    if !ordinary_file(&settings)
        || std::fs::metadata(&settings).is_ok_and(|m| m.len() > 1024 * 1024)
    {
        return Vec::new();
    }
    let Ok(contents) = std::fs::read_to_string(settings) else { return Vec::new() };
    let Some(value) = contents.lines().find_map(|line|
        line.strip_prefix("resourcePacks:")) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<String>>(value.trim()) {
        Ok(packs) if packs.len() <= MAX_SELECTED_PACKS => packs,
        _ => {
            warnings.push("Could not safely parse selected packs from options.txt".into());
            Vec::new()
        }
    }
}
fn selected_packs(game: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let directory = game.join("resourcepacks");
    if !ordinary_directory(&directory) { return Vec::new(); }
    let mut out = Vec::new();
    // Vanilla stores the selected list in pack repository order (low -> high).
    // Ignore built-in IDs such as vanilla, fabric, or mod_resources here.
    for entry in selected_pack_names(game, warnings) {
        let Some(name) = entry.strip_prefix("file/") else { continue };
        // Restrict to a single filename. This rejects absolute paths,
        // escaped separators and ../ traversals from untrusted options.txt.
        if name.is_empty() || name.len() > 200
            || matches!(name, "." | "..")
            || name.contains('/') || name.contains('\\') || name.contains(':')
            || name.contains('\0')
        {
            warnings.push("Ignored unsafe selected resource-pack filename".into());
            continue;
        }
        let candidate = directory.join(name);
        if is_zip_file(&candidate) || has_assets(&candidate) {
            if !out.contains(&candidate) { out.push(candidate); }
        } else {
            warnings.push(format!("Missing or unsupported enabled resource pack: {name}"));
        }
    }
    out
}
fn mod_sources(game: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let mods = game.join("mods");
    if !ordinary_directory(&mods) { return Vec::new(); }
    let Ok(entries) = std::fs::read_dir(&mods) else {
        warnings.push("Cannot enumerate this instance's mods directory".into());
        return Vec::new();
    };
    let mut candidates = Vec::new();
    // The folder is flat by design: no recursive drive or dependency scan.
    for entry in entries.take(4096) {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("jar"))
            && ordinary_file(&path) {
            candidates.push(path);
        }
    }
    candidates.sort();
    if candidates.len() > MAX_MOD_JARS {
        warnings.push(format!(
            "Only the first {MAX_MOD_JARS} mod JARs are eligible for the bounded asset scan"
        ));
    }
    candidates.truncate(MAX_MOD_JARS);
    candidates.into_iter().filter(|p| mod_archive_contains_assets(p)).collect()
}

/// Probe only the selected world's launcher / game folder, not every instance
/// under the launcher. Mod archive assets are read-only and cannot execute.
pub(crate) fn discover_stack(root: &Path, version: &str) -> Option<AssetStack> {
    let instance = game_directory(root);
    let mut out = AssetStack {
        paths: Vec::new(),
        instance: instance.clone(),
        vanilla: false,
        mod_count: 0,
        selected_count: 0,
        generated_count: 0,
        warnings: Vec::new(),
    };
    if let Some(client) = discover(root, version) {
        out.paths.push(client);
        out.vanilla = true;
    }
    if let Some(game) = instance {
        let mods = mod_sources(&game, &mut out.warnings);
        out.mod_count = mods.len();
        out.paths.extend(mods);
        // KubeJS' client assets directory behaves as a generated mod pack.
        // Data and scripts are NEVER read or executed.
        let kubejs = game.join("kubejs");
        if has_assets(&kubejs) {
            out.paths.push(kubejs);
            out.generated_count += 1;
        }
        let selected = selected_packs(&game, &mut out.warnings);
        out.selected_count = selected.len();
        out.paths.extend(selected);
    }
    if !out.vanilla && !out.paths.is_empty() {
        out.warnings.push(
            "Original client JAR was not found; vanilla block textures may be missing".into()
        );
    }
    if out.paths.len() > MAX_ASSET_LAYERS {
        out.warnings.push("Local resource stack exceeds the layer budget".into());
        out.paths.truncate(MAX_ASSET_LAYERS);
    }
    (!out.paths.is_empty()).then_some(out)
}
pub(crate) fn load(index: usize, world: &WorldSummary, nonce: u64) -> Task<Message> {
    let root = world.root.clone();
    let version = world.version_name.clone().unwrap_or_default();
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            let return_root = root.clone();
            std::thread::spawn(move || {
                let result = discover_stack(&root, &version).map(|stack| {
                    TextureBank::open_stack(&stack.paths).map(|bank| (stack, bank))
                });
                let _ = tx.send(result);
            });
            (index, return_root, nonce, rx.await.unwrap_or(None))
        },
        |(index, root, nonce, result)| Message::AutoAssetsLoaded(index, root, nonce, result),
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
    fn write_fake_zip(path: &Path, name: &str) {
        use std::io::Write;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"opaque bytes for discovery only").unwrap();
        zip.finish().unwrap();
    }
    #[test]
    fn active_mods_kubejs_and_enabled_packs_follow_the_selected_world_instance() {
        let root = std::env::temp_dir().join(format!(
            "mcedit2-instance-discovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("instances/demo/.minecraft");
        let world = game.join("saves/TestWorld");
        std::fs::create_dir_all(&world).unwrap();
        let vanilla = candidate_paths(&game, "1.21.1")[2].clone();
        std::fs::create_dir_all(vanilla.parent().unwrap()).unwrap();
        std::fs::write(&vanilla, b"vanilla discovery placeholder").unwrap();
        let mods = game.join("mods");
        write_fake_zip(&mods.join("a-mod.jar"),
            "assets/a_mod/textures/block/machine.png");
        write_fake_zip(&mods.join("z-mod.jar"),
            "assets/z_mod/blockstates/machine.json");
        write_fake_zip(&mods.join("disabled.jar.disabled"),
            "assets/disabled/textures/block/unused.png");
        write_fake_zip(&mods.join("library.jar"), "com/example/Test.class");
        std::fs::create_dir_all(game.join("kubejs/assets/kubejs/textures/block")).unwrap();
        write_fake_zip(&game.join("resourcepacks/low.zip"),
            "assets/minecraft/textures/block/stone.png");
        write_fake_zip(&game.join("resourcepacks/high.zip"),
            "assets/minecraft/textures/block/stone.png");
        write_fake_zip(&game.join("resourcepacks/not-enabled.zip"),
            "assets/minecraft/textures/block/stone.png");
        std::fs::write(game.join("options.txt"),
            "resourcePacks:[\"vanilla\",\"file/low.zip\",\"file/high.zip\"]\n").unwrap();
        let stack = discover_stack(&world, "1.21.1").unwrap();
        assert!(stack.vanilla);
        assert_eq!(stack.mod_count, 2);
        assert_eq!(stack.generated_count, 1);
        assert_eq!(stack.selected_count, 2);
        assert_eq!(stack.paths, vec![
            vanilla,
            mods.join("a-mod.jar"),
            mods.join("z-mod.jar"),
            game.join("kubejs"),
            game.join("resourcepacks/low.zip"),
            game.join("resourcepacks/high.zip"),
        ]);
        assert!(same_instance(&world, &game.join("saves/AnotherWorld")));
        assert!(!same_instance(&world,
            &root.join("instances/unrelated/.minecraft/saves/World")));
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn selected_pack_options_refuse_escape_and_never_enable_all_installed_zips() {
        let root = std::env::temp_dir().join(format!(
            "mcedit2-selected-pack-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("minecraft");
        write_fake_zip(&game.join("resourcepacks/good.zip"), "assets/a/textures/block/a.png");
        write_fake_zip(&root.join("evil.zip"), "assets/a/textures/block/a.png");
        std::fs::write(game.join("options.txt"),
            "resourcePacks:[\"file/../evil.zip\",\"file/good.zip\",\"file/disabled.zip\"]\n").unwrap();
        let mut warnings = Vec::new();
        let packs = selected_packs(&game, &mut warnings);
        assert_eq!(packs, vec![game.join("resourcepacks/good.zip")]);
        assert!(warnings.iter().any(|s| s.contains("unsafe")));
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
