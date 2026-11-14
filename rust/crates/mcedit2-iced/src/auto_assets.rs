//! Read-only discovery of an installed vanilla Minecraft client JAR.
//! Never infer that arbitrary resource packs are active, never download
//! assets, traverse a whole drive, or inspect an unrelated world folder.
use crate::{textures::TextureBank, Message};
use iced::Task;
use mcedit2_world::WorldSummary;
use std::path::{Path, PathBuf};
#[path = "launcher_framework.rs"]
mod framework;

pub(crate) fn is_simple_version(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && !value
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | ';'))
}
/// 读取客户端自身声明的版本；不以用户给资源包起的文件名判断新版本。
pub(crate) fn client_version(path: &Path) -> Result<Option<String>, String> {
    use std::io::Read;
    if crate::pack_sources::is_nested(path) {
        return Ok(None);
    }
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("jar"))
    {
        return Ok(None);
    }
    if !ordinary_file(path)? {
        return Err(format!("客户端候选不是普通文件：{}", path.display()));
    }
    let mut archive = zip::ZipArchive::new(
        std::fs::File::open(path).map_err(|error| format!("{}：{error}", path.display()))?,
    )
    .map_err(|error| format!("{}：{error}", path.display()))?;
    let member = match archive.by_name("version.json") {
        Ok(member) => member,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    member
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}：{error}", path.display()))?;
    if bytes.len() > 65536 {
        return Err("客户端版本声明超过 64 KiB".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}：{error}", path.display()))?;
    let (Some(world), Some(protocol)) = (value.get("world_version"), value.get("protocol_version"))
    else {
        return Ok(None);
    };
    if world.as_i64().is_none() || protocol.as_i64().is_none() {
        return Err(format!(
            "客户端版本声明的 world_version / protocol_version 必须为整数：{}",
            path.display()
        ));
    }
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|id| is_simple_version(id))
        .ok_or_else(|| format!("客户端版本声明缺少有效 id：{}", path.display()))?;
    Ok(Some(id.to_owned()))
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
pub(crate) fn locate_in<'a>(
    bases: impl IntoIterator<Item = &'a Path>,
    version: &str,
) -> Result<Option<PathBuf>, String> {
    if !is_simple_version(version) {
        return Err(format!("客户端版本标识无效：{version}"));
    }
    for base in bases {
        for candidate in candidate_paths(base, version) {
            // Never follow a symlink to a client JAR outside this install.
            // The ZIP reader is read-only; no unpacking or client modification.
            match candidate.symlink_metadata() {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        return Err(format!("客户端 JAR 不是普通文件：{}", candidate.display()));
                    }
                    return Ok(Some(candidate));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("{}：{error}", candidate.display())),
            }
        }
    }
    Ok(None)
}
/// Search the original world's launcher first, then a bounded set of
/// platform-standard vanilla/PrismLauncher/MultiMC installation roots.
/// This matters when users keep world saves on another drive or import a
/// downloaded world into the editor without moving the Minecraft client.
fn search_roots(root: &Path, configured: &[PathBuf]) -> Vec<PathBuf> {
    let mut bases = root.ancestors().map(Path::to_path_buf).collect::<Vec<_>>();
    bases.extend_from_slice(configured);
    bases.extend(crate::launcher_roots::installed());
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
    let mut unique = Vec::new();
    for base in bases {
        if !unique.contains(&base) {
            unique.push(base);
        }
    }
    unique
}
pub(crate) fn discover(
    root: &Path,
    version: &str,
    configured: &[PathBuf],
) -> Result<Option<PathBuf>, String> {
    if !is_simple_version(version) {
        // 无版本声明的旧存档仍可加载其实际模组 / 资源包，不能猜测客户端版本。
        if version.is_empty() {
            return Ok(None);
        }
        return Err(format!("客户端版本标识无效：{version}"));
    }
    locate_in(
        search_roots(root, configured).iter().map(PathBuf::as_path),
        version,
    )
}
/// One detected source stack. The order is low -> high priority; the
/// final user-selected overlays are appended in the UI worker.
#[derive(Debug, Clone)]
pub(crate) struct AssetStack {
    pub paths: Vec<PathBuf>,
    pub instance: Option<PathBuf>,
    pub vanilla: bool,
    pub mod_count: usize,
    pub framework_count: usize,
    pub selected_count: usize,
    pub generated_count: usize,
    pub warnings: Vec<String>,
}

/// Recognize an instance from the selected world's actual saves/ directory.
/// Never pick mods from a different nearby instance or scan sibling worlds.
fn game_directory(world: &Path) -> Option<PathBuf> {
    let saves = world.parent()?;
    if !saves
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("saves")
    {
        return None;
    }
    Some(saves.parent()?.to_path_buf())
}
fn metadata(path: &Path) -> Result<Option<std::fs::Metadata>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}：{error}", path.display())),
    }
}
fn ordinary_directory(path: &Path) -> Result<bool, String> {
    Ok(metadata(path)?
        .is_some_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink()))
}
fn ordinary_file(path: &Path) -> Result<bool, String> {
    Ok(metadata(path)?
        .is_some_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink()))
}
fn has_assets(path: &Path) -> Result<bool, String> {
    Ok(ordinary_directory(path)? && ordinary_directory(&path.join("assets"))?)
}
fn is_zip_file(path: &Path) -> Result<bool, String> {
    Ok(path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("zip"))
        && ordinary_file(path)?)
}
fn mod_archive_contains_assets(path: &Path) -> Result<bool, String> {
    use std::fs::File;
    if path
        .extension()
        .and_then(|v| v.to_str())
        .is_none_or(|v| !v.eq_ignore_ascii_case("jar"))
        || !ordinary_file(path)?
    {
        return Ok(false);
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    // No ZIP member is extracted or decompressed in discovery. Require a
    // namespace asset subtree；语言表、字体等同样是实际资源，不只识别模型或图片。
    let contains_assets = archive.file_names().any(|name| {
        let mut parts = name.split('/');
        !name.ends_with('/')
            && matches!(parts.next(), Some("assets"))
            && parts.next().is_some_and(crate::resource_ids::namespace)
            && parts.next().is_some_and(|part| !part.is_empty())
    });
    Ok(contains_assets)
}
fn selected_pack_names(game: &Path) -> Result<Vec<String>, String> {
    use std::io::Read;
    let settings = game.join("options.txt");
    let Some(metadata) = metadata(&settings)? else {
        return Ok(Vec::new());
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("游戏选项不是普通文件：{}", settings.display()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&settings)
        .map_err(|error| format!("{}：{error}", settings.display()))?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}：{error}", settings.display()))?;
    if bytes.len() > 1024 * 1024 {
        return Err(format!("游戏选项超过 1 MiB：{}", settings.display()));
    }
    let contents =
        String::from_utf8(bytes).map_err(|error| format!("{}：{error}", settings.display()))?;
    let Some(value) = contents
        .lines()
        .find_map(|line| line.strip_prefix("resourcePacks:"))
    else {
        return Ok(Vec::new());
    };
    serde_json::from_str(value.trim())
        .map_err(|error| format!("无法解析 {} 中启用的资源包：{error}", settings.display()))
}
fn selected_packs(game: &Path, client: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let directory = game.join("resourcepacks");
    let mut out = Vec::new();
    // Vanilla stores the selected list in pack repository order (low -> high).
    // 已包含的原版及模组聚合来源无需再次追加；旧列表也可以直接保存实际文件名。
    for entry in selected_pack_names(game)? {
        if matches!(entry.as_str(), "vanilla" | "fabric" | "mod_resources") {
            continue;
        }
        let name = entry.strip_prefix("file/").unwrap_or(&entry);
        // Restrict to a single filename. This rejects absolute paths,
        // escaped separators and ../ traversals from untrusted options.txt.
        if name.is_empty()
            || matches!(name, "." | "..")
            || name.contains('/')
            || name.contains('\\')
            || name.contains(':')
            || name.contains('\0')
        {
            return Err(format!("启用的资源包文件名无效：{entry}"));
        }
        if !entry.starts_with("file/") {
            if let Some(client) = client {
                if let Some(pack) = crate::client_assets::builtin_pack(client, name)? {
                    if !out.contains(&pack) {
                        out.push(pack);
                    }
                    continue;
                }
            }
        }
        let candidate = directory.join(name);
        if is_zip_file(&candidate)? || ordinary_directory(&candidate)? {
            if !out.contains(&candidate) {
                out.push(candidate);
            }
        } else {
            return Err(format!(
                "未找到启用资源包的实际文件或目录：{entry}（{}）",
                candidate.display()
            ));
        }
    }
    Ok(out)
}
fn mod_sources(game: &Path) -> Result<Vec<PathBuf>, String> {
    let mods = game.join("mods");
    let Some(metadata) = metadata(&mods)? else {
        return Ok(Vec::new());
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("模组来源不是普通目录：{}", mods.display()));
    }
    let entries =
        std::fs::read_dir(&mods).map_err(|error| format!("{}：{error}", mods.display()))?;
    let mut candidates = Vec::new();
    // The folder is flat by design: no recursive drive or dependency scan.
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读取 {} 目录项失败：{error}", mods.display()))?;
        let path = entry.path();
        if path
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("jar"))
            && ordinary_file(&path)?
        {
            candidates.push(path);
        }
    }
    candidates.sort();
    let mut sources = Vec::new();
    for path in candidates {
        if mod_archive_contains_assets(&path)
            .map_err(|error| format!("无法索引模组资源 {}：{error}", path.display()))?
        {
            sources.push(path);
        }
    }
    Ok(sources)
}

/// Probe only the selected world's launcher / game folder, not every instance
/// under the launcher. Mod archive assets are read-only and cannot execute.
pub(crate) fn discover_stack(
    root: &Path,
    version: &str,
    configured: &[PathBuf],
) -> Result<Option<AssetStack>, String> {
    let instance = game_directory(root);
    let stack = assemble_stack(
        root,
        version,
        configured,
        instance,
        discover(root, version, configured)?,
    )?;
    Ok((!stack.paths.is_empty()).then_some(stack))
}
/// 显式实例资源只接受该启动器的准确客户端，不借用其他安装或伪造世界路径。
pub(crate) fn instance_stack(
    game: &Path,
    version: &str,
    launcher: &Path,
) -> Result<AssetStack, String> {
    if !ordinary_directory(game)? {
        return Err(format!(
            "实例游戏目录不存在或不是普通目录：{}",
            game.display()
        ));
    }
    let client = locate_in([launcher], version)?.ok_or_else(|| {
        format!(
            "启动器 {} 中没有实例所需的 {version} 客户端",
            launcher.display()
        )
    })?;
    assemble_stack(
        game,
        version,
        &[launcher.to_path_buf()],
        Some(game.to_path_buf()),
        Some(client),
    )
}
fn assemble_stack(
    root: &Path,
    version: &str,
    configured: &[PathBuf],
    instance: Option<PathBuf>,
    client: Option<PathBuf>,
) -> Result<AssetStack, String> {
    let mut out = AssetStack {
        paths: Vec::new(),
        instance: instance.clone(),
        vanilla: false,
        mod_count: 0,
        framework_count: 0,
        selected_count: 0,
        generated_count: 0,
        warnings: Vec::new(),
    };
    if let Some(client) = client {
        out.paths.push(client);
        out.vanilla = true;
    }
    if let Some(game) = instance {
        // 框架资源优先于用户模组，且版本只取当前实例显式声明。
        let frameworks = framework::discover(&game, version, &search_roots(root, configured))?;
        out.framework_count = frameworks.len();
        out.paths.extend(frameworks);
        let mods = mod_sources(&game)?;
        out.mod_count = mods.len();
        out.paths.extend(mods);
        // KubeJS' client assets directory behaves as a generated mod pack.
        // Data and scripts are NEVER read or executed.
        let kubejs = game.join("kubejs");
        if has_assets(&kubejs)? {
            out.paths.push(kubejs);
            out.generated_count += 1;
        }
        let selected = selected_packs(&game, out.vanilla.then(|| out.paths[0].as_path()))?;
        out.selected_count = selected.len();
        out.paths.extend(selected);
    }
    if !out.vanilla && !out.paths.is_empty() {
        out.warnings.push(
            "Original client JAR was not found; vanilla block textures may be missing".into(),
        );
    }
    Ok(out)
}
pub(crate) fn load(
    index: usize,
    world: &WorldSummary,
    nonce: u64,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    budget: &str,
    all_png: bool,
    plugin_sources: Vec<mcedit2_plugins::catalog::PluginSource>,
    block_definitions: std::sync::Arc<Vec<crate::block_definitions::Binding>>,
    configured: Vec<PathBuf>,
) -> Task<Message> {
    let root = world.root.clone();
    let budget = match crate::resource_budget::ResourceBudget::from_input(budget) {
        Ok(budget) => budget,
        Err(e) => return Task::done(Message::AutoAssetsLoaded(index, root, nonce, Some(Err(e)))),
    };
    let version = world.version_name.clone().unwrap_or_default();
    let world = world.clone();
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let progress = sender.clone();
    let control = crate::textures::LoadControl {
        budget: Some(budget),
        all_png,
        block_definitions,
        world_data: std::sync::Arc::clone(&world.world_data),
        cancelled,
        progress: std::sync::Arc::new(move |text| {
            if progress
                .unbounded_send(Message::TextureProgress(nonce, text))
                .is_err()
            {
                log::debug!("材质加载进度接收端已关闭");
            }
        }),
    };
    std::thread::spawn(move || {
        let result = (|| {
            discover_stack(&root, &version, &configured)?
                .or_else(|| {
                    (!plugin_sources.is_empty()).then(|| AssetStack {
                        paths: Vec::new(),
                        instance: game_directory(&root),
                        vanilla: false,
                        mod_count: 0,
                        framework_count: 0,
                        selected_count: 0,
                        generated_count: 0,
                        warnings: vec!["未发现原版客户端资源，插件资源单独加载".into()],
                    })
                })
                .map(|mut stack| {
                    let mut plugin_snapshots = Vec::new();
                    for source in &plugin_sources {
                        source.verify_unchanged()?;
                        if let Some(resources) = source.resources()? {
                            stack.warnings.push(format!(
                                "插件资源 {}：命名空间 {}，SHA-256 {}",
                                source.path.display(),
                                resources.namespaces.join(", "),
                                resources.sha256
                            ));
                            stack.paths.push(resources.root.clone());
                            plugin_snapshots.push((source, resources));
                        }
                    }
                    if stack.vanilla && control.block_definitions.is_empty() {
                        let mut base =
                            TextureBank::open_stack_java_controlled(&stack.paths[..1], &control)?;
                        base.apply_world_data(&world, &control)?;
                        sender
                            .unbounded_send(Message::TextureBaseLoaded(
                                nonce,
                                Some((index, root.clone())),
                                base,
                            ))
                            .map_err(|e| format!("无法显示基础材质：{e}"))?;
                    }
                    let mut bank = TextureBank::open_stack_java_controlled(&stack.paths, &control)?;
                    bank.apply_world_data(&world, &control)?;
                    for (source, expected) in plugin_snapshots {
                        source.verify_unchanged()?;
                        let current = source.resources()?.ok_or("插件资源声明在加载期间消失")?;
                        if current.sha256 != expected.sha256 {
                            return Err("插件资源加载期间发生变化，请重新加载".into());
                        }
                    }
                    Ok((stack, bank))
                })
                .transpose()
        })()
        .transpose();
        if sender
            .unbounded_send(Message::AutoAssetsLoaded(index, root, nonce, result))
            .is_err()
        {
            log::debug!("材质加载完成，但界面接收端已关闭");
        }
    });
    Task::run(receiver, |message| message)
}

pub(crate) fn load_manual(
    path: PathBuf,
    world: Option<WorldSummary>,
    nonce: u64,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    budget: &str,
    all_png: bool,
    plugin_sources: Vec<mcedit2_plugins::catalog::PluginSource>,
    block_definitions: std::sync::Arc<Vec<crate::block_definitions::Binding>>,
    configured: Vec<PathBuf>,
    discover_world: bool,
) -> Task<Message> {
    let budget = match crate::resource_budget::ResourceBudget::from_input(budget) {
        Ok(budget) => budget,
        Err(e) => return Task::done(Message::TextureLoadFinished(nonce, path, Err(e))),
    };
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let progress = sender.clone();
    let control = crate::textures::LoadControl {
        budget: Some(budget),
        all_png,
        block_definitions,
        world_data: world
            .as_ref()
            .map(|world| std::sync::Arc::clone(&world.world_data))
            .unwrap_or_default(),
        cancelled,
        progress: std::sync::Arc::new(move |text| {
            if progress
                .unbounded_send(Message::TextureProgress(nonce, text))
                .is_err()
            {
                log::debug!("材质加载进度接收端已关闭");
            }
        }),
    };
    std::thread::spawn(move || {
        let result = (|| {
            let layers = path
                .to_string_lossy()
                .split(';')
                .map(|p| PathBuf::from(p.trim()))
                .collect::<Vec<_>>();
            if layers.is_empty() || layers.iter().any(|p| p.as_os_str().is_empty()) {
                return Err("资源来源包含空路径".into());
            }
            let mut selected_client = None;
            for layer in &layers {
                if let Some(version) = client_version(layer)? {
                    if selected_client.as_ref().is_some_and(|old| old != &version) {
                        return Err("不能叠加两个不同版本的原版客户端".into());
                    }
                    selected_client = Some(version);
                }
            }
            if let (Some(expected), Some(selected)) = (
                world.as_ref().and_then(|w| w.version_name.as_ref()),
                selected_client.as_ref(),
            ) {
                if expected != selected {
                    sender.unbounded_send(Message::TextureWarning(nonce,format!("此存档记录的版本为 {expected}，当前手动选择的客户端为 {selected}；新增方块的模型或纹理可能缺失。"))).map_err(|e|e.to_string())?;
                }
            }
            // 手动选择的资源先独立完成并显示，不必等待整个模组实例的索引。
            let mut base = TextureBank::open_stack_java_controlled(&layers, &control)?;
            if let Some(world) = &world {
                base.apply_world_data(world, &control)?;
            }
            sender
                .unbounded_send(Message::TextureBaseLoaded(nonce, None, base))
                .map_err(|e| format!("无法显示基础材质：{e}"))?;
            let detected = world
                .as_ref()
                .filter(|_| discover_world)
                .map(|w| {
                    discover_stack(
                        &w.root,
                        w.version_name.as_deref().unwrap_or(""),
                        &configured,
                    )
                })
                .transpose()?
                .flatten();
            let mut stack = detected
                .as_ref()
                .map(|s| s.paths.clone())
                .unwrap_or_default();
            if selected_client.is_some()
                && detected.as_ref().is_some_and(|s| s.vanilla)
                && !stack.is_empty()
            {
                stack.remove(0);
            }
            stack.retain(|base| !layers.contains(base));
            stack.extend(layers);
            let mut snapshots = Vec::new();
            for source in &plugin_sources {
                source.verify_unchanged()?;
                if let Some(resources) = source.resources()? {
                    sender
                        .unbounded_send(Message::TextureWarning(
                            nonce,
                            format!(
                                "插件资源 {}：命名空间 {}，SHA-256 {}",
                                source.path.display(),
                                resources.namespaces.join(", "),
                                resources.sha256
                            ),
                        ))
                        .map_err(|e| e.to_string())?;
                    stack.push(resources.root.clone());
                    snapshots.push((source, resources));
                }
            }
            let mut bank = TextureBank::open_stack_java_controlled(&stack, &control)?;
            if let Some(world) = &world {
                bank.apply_world_data(world, &control)?;
            }
            for (source, expected) in snapshots {
                source.verify_unchanged()?;
                let current = source.resources()?.ok_or("插件资源声明在加载期间消失")?;
                if current.sha256 != expected.sha256 {
                    return Err("插件资源加载期间发生变化，请重新加载".into());
                }
            }
            Ok((bank, detected))
        })();
        if sender
            .unbounded_send(Message::TextureLoadFinished(nonce, path, result))
            .is_err()
        {
            log::debug!("手动资源加载完成，但界面接收端已关闭");
        }
    });
    Task::run(receiver, |message| message)
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
    framework::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_chunk_mesh_matches_source_coordinates() {
        let world = mcedit2_world::inspect_world(PathBuf::from(
            std::env::var_os("MCEDIT2_TEST_WORLD").unwrap(),
        ))
        .unwrap();
        probe_chunk_mesh(world, None);
    }
    fn probe_chunk_mesh(world: WorldSummary, bank: Option<std::sync::Arc<TextureBank>>) {
        use std::sync::{Arc, Mutex};
        let session =
            mcedit2_world::editing::Session::new(world.clone(), world.dimensions[0].name.clone())
                .unwrap();
        for (x, y, z) in [
            (0, 211, 0),
            (0, 125, 0),
            (43, 125, -66),
            (43, 70, -66),
            (0, 70, 0),
        ] {
            eprintln!(
                "源 {x},{y},{z}: {:?}",
                mcedit2_world::block_lookup::read_world_block(&world, session.dimension(), x, y, z)
                    .unwrap()
            );
        }
        let mut reader =
            mcedit2_world::block_lookup::ReadOnlyRegionCache::new(&world, session.dimension())
                .unwrap();
        let patch =
            mcedit2_world::chunk_surface::inspect_full_chunk_cached(&session, &mut reader, 2, -5)
                .unwrap();
        let surface = Arc::new(mcedit2_world::surface::SurfaceView {
            biomes: Default::default(),
            x: 32,
            z: -80,
            y_min: -64,
            y_max: 319,
            width: 16,
            depth: 16,
            voxels: patch.voxels,
            source_chunks: 1,
            missing_columns: 0,
            source_tiles: vec![patch.tile],
        });
        let scene = crate::spectator::SpectatorScene {
            entity_frame: None,
            entity_enabled: false,
            entity_cache: Default::default(),
            surface,
            show_textures: bank.is_some(),
            block_layers: Default::default(),
            texture_bank: bank,
            camera: crate::spectator::Camera::default(),
            quality: crate::gpu_mesh::Quality::Fast,
            cache: Arc::new(Mutex::new(None)),
            pending: Arc::new(Mutex::new(false)),
            presented: Default::default(),
            presented_view: Default::default(),
            fallback: None,
            snapshot: None,
            chunk_cache: Arc::new(Mutex::new(
                crate::chunk_mesh_cache::ChunkMeshCache::default(),
            )),
            lighting: crate::lighting::Table::unlit(),
        };
        let start = std::time::Instant::now();
        let vertices = crate::spectator::build_cancellable(&scene, &|| false).unwrap();
        let min: [f32; 3] = std::array::from_fn(|i| {
            vertices
                .iter()
                .map(|v| v.xyz[i])
                .fold(f32::INFINITY, f32::min)
        });
        let max: [f32; 3] = std::array::from_fn(|i| {
            vertices
                .iter()
                .map(|v| v.xyz[i])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        eprintln!(
            "单区块真实网格 {} 顶点，{:?}，相对坐标 {min:?}..{max:?}",
            vertices.len(),
            start.elapsed()
        );
        assert!(!vertices.is_empty());
        for vertex in vertices {
            assert!(
                (-2.0..=18.0).contains(&vertex.xyz[0]) && (-2.0..=18.0).contains(&vertex.xyz[2])
            );
        }
    }

    fn real_instance_assets_finish_loading() {
        let path = std::env::var_os("MCEDIT2_TEST_WORLD").expect("设置 MCEDIT2_TEST_WORLD");
        let world = mcedit2_world::inspect_world(PathBuf::from(path)).unwrap();
        eprintln!(
            "世界版本 {:?}, DataVersion {:?}",
            world.version_name, world.data_version
        );
        let stack = discover_stack(
            &world.root,
            world.version_name.as_deref().unwrap_or(""),
            &[],
        )
        .unwrap()
        .expect("必须找到实际实例资源");
        eprintln!(
            "资源栈：原版 {}，模组 {}，生成包 {}，启用包 {}，诊断 {:?}",
            stack.vanilla,
            stack.mod_count,
            stack.generated_count,
            stack.selected_count,
            stack.warnings
        );
        let start = std::time::Instant::now();
        let bank = TextureBank::open_stack_java(&stack.paths).unwrap();
        eprintln!(
            "实例资源完成：{} PNG / {} 模型 / {:?}；索引/请求/成功/失败 {}/{}/{}/{}，缺失引用 {}，RGBA {} MiB，3D 图集 {} PNG / {} MiB，预算 {} MiB",
            bank.count(),
            bank.model_count(),
            start.elapsed(),
            bank.statistics.indexed_png,bank.statistics.requested_png,bank.statistics.decoded_png,bank.statistics.failed_png,
            bank.statistics.missing_references.len(),bank.statistics.decoded_bytes/(1024*1024),
            bank.statistics.atlas_png,bank.statistics.atlas_bytes/(1024*1024),bank.statistics.budget_bytes/(1024*1024)
        );
        assert!(bank.count() > 100);
        crate::entity_geometry::tests::installed(&bank, &world);
        std::fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/instance-asset-diagnostics.txt"),
            bank.warnings.join("\n"),
        )
        .unwrap();
        eprintln!("原始存档共 {} 个区块", world.total_chunks());
        let session =
            mcedit2_world::editing::Session::new(world.clone(), world.dimensions[0].name.clone())
                .unwrap();
        let mut reader =
            mcedit2_world::block_lookup::ReadOnlyRegionCache::new(&world, session.dimension())
                .unwrap();
        let start = std::time::Instant::now();
        let mut count = 0;
        let mut outside = 0;
        let mut voxels = Vec::new();
        let mut tiles = Vec::new();
        for cx in 1..=3 {
            for cz in -6..=-4 {
                let patch = mcedit2_world::chunk_surface::inspect_full_chunk_cached(
                    &session,
                    &mut reader,
                    cx,
                    cz,
                )
                .unwrap();
                count += patch.voxels.len();
                outside += patch
                    .voxels
                    .iter()
                    .filter(|v| v.y < 101 || v.y > 148)
                    .count();
                eprintln!(
                    "完整区块 {cx},{cz}: {} 暴露方块，高度 {:?}..{:?}",
                    patch.voxels.len(),
                    patch.voxels.iter().map(|v| v.y).min(),
                    patch.voxels.iter().map(|v| v.y).max()
                );
                voxels.extend(patch.voxels);
                tiles.push(patch.tile);
            }
        }
        eprintln!(
            "9 个完整区块：{count} 暴露方块，其中 {outside} 在旧相机 48 层以外，耗时 {:?}",
            start.elapsed()
        );
        assert!(count > 0);
        let bank = std::sync::Arc::new(bank);
        {
            use crate::spectator::{build_scene_cancellable, SpectatorScene};
            use std::sync::Arc;
            let mut scene = SpectatorScene {
                entity_frame: None,
                entity_enabled: false,
                entity_cache: Default::default(),
                surface: Arc::new(mcedit2_world::surface::SurfaceView {
                    biomes: Default::default(),
                    x: 16,
                    z: -96,
                    y_min: -64,
                    y_max: 319,
                    width: 48,
                    depth: 48,
                    voxels,
                    source_chunks: 9,
                    source_tiles: tiles,
                    missing_columns: 0,
                }),
                texture_bank: Some(Arc::clone(&bank)),
                show_textures: true,
                block_layers: Default::default(),
                camera: Default::default(),
                quality: crate::gpu_mesh::Quality::Fast,
                cache: Default::default(),
                pending: Default::default(),
                presented: Default::default(),
                presented_view: Default::default(),
                fallback: None,
                snapshot: None,
                chunk_cache: Default::default(),
                lighting: crate::lighting::Table::unlit(),
            };
            let started = std::time::Instant::now();
            let cold = build_scene_cancellable(&scene, &|| false).unwrap();
            let cold_time = started.elapsed();
            assert_eq!(cold.chunks.len(), 9);
            let surface = Arc::make_mut(&mut scene.surface);
            surface.x -= 16;
            surface.z += 16;
            surface.y_min += 16;
            let started = std::time::Instant::now();
            let warm = build_scene_cancellable(&scene, &|| false).unwrap();
            let warm_time = started.elapsed();
            assert_ne!(cold.origin, warm.origin);
            for (a, b) in cold.chunks.iter().zip(&warm.chunks) {
                assert_eq!(a.origin, b.origin);
                assert!(Arc::ptr_eq(&a.quads(), &b.quads()));
                assert!(Arc::ptr_eq(&a.picking, &b.picking));
            }
            eprintln!("真实 9 区块材质场景：冷建模 {cold_time:?}，移动原点复用 {warm_time:?}；9/9 源面与拾取索引原分配复用，{} 个紧凑面",
                cold.chunks.iter().map(|c| c.quads().len()).sum::<usize>());
        }
        probe_chunk_mesh(world.clone(), Some(bank));
    }

    fn client_version_path_cannot_escape_world_ancestors() {
        for value in ["../../other", "", "1.21.1/../evil", "1.21.1\\other"] {
            assert!(!is_simple_version(value));
        }
        assert!(is_simple_version("1.21.1"));
        assert!(is_simple_version("1.20.1-forge"));
    }

    fn exact_launcher_paths_support_prism_and_vanilla_without_traversal() {
        let base = Path::new("/launcher");
        let paths = candidate_paths(base, "1.21.1");
        assert_eq!(
            paths[0],
            base.join("libraries/com/mojang/minecraft/1.21.1/minecraft-1.21.1-client.jar")
        );
        assert_eq!(paths[2], base.join("versions/1.21.1/1.21.1.jar"));
        assert_eq!(paths[3], base.join(".minecraft/versions/1.21.1/1.21.1.jar"));
        assert!(locate_in([base], "../unsafe").is_err());
    }

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
            locate_in([first.as_path(), second.as_path()], version).unwrap(),
            Some(expected)
        );
        let _ = std::fs::remove_dir_all(root);
    }
    fn write_fake_zip(path: &Path, name: &str) {
        use std::io::Write;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"opaque bytes for discovery only").unwrap();
        zip.finish().unwrap();
    }

    fn large_instance_keeps_all_mods_and_enabled_packs_in_priority_order() {
        let root = std::env::temp_dir().join(format!(
            "mcedit2-large-resource-stack-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = root.join("minecraft");
        let world = game.join("saves/TestWorld");
        std::fs::create_dir_all(&world).unwrap();
        let mut expected = Vec::new();
        for i in 0..1100 {
            let path = game.join(format!("mods/mod-{i:04}.jar"));
            write_fake_zip(&path, "assets/demo/textures/block/tile.png");
            expected.push(path);
        }
        // 扫描目录项不能先截断，否则禁用文件或日志也会挤掉有效 JAR。
        for i in 0..4100 {
            std::fs::write(game.join(format!("mods/disabled-{i:04}.jar.disabled")), []).unwrap();
        }
        let generated = game.join("kubejs");
        std::fs::create_dir_all(generated.join("assets")).unwrap();
        expected.push(generated);
        let mut options = vec!["vanilla".to_owned()];
        for i in 0..130 {
            let name = format!("pack-{i:03}");
            let path = game.join("resourcepacks").join(&name);
            std::fs::create_dir_all(path.join("assets")).unwrap();
            options.push(format!("file/{name}"));
            expected.push(path);
        }
        std::fs::write(
            game.join("options.txt"),
            format!(
                "resourcePacks:{}\n",
                serde_json::to_string(&options).unwrap()
            ),
        )
        .unwrap();
        let stack = discover_stack(&world, "9.9.9-count-regression", &[])
            .unwrap()
            .unwrap();
        assert!(!stack.vanilla);
        assert_eq!(stack.mod_count, 1100);
        assert_eq!(stack.selected_count, 130);
        assert_eq!(stack.generated_count, 1);
        assert_eq!(stack.paths, expected);
        assert_eq!(stack.warnings.len(), 1); // 仅缺少该虚构版本的原版客户端。
        let root = root.canonicalize().unwrap();
        assert_eq!(
            root.parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn damaged_mod_archive_stops_resource_discovery() {
        let root =
            std::env::temp_dir().join(format!("mcedit2-damaged-mod-assets-{}", std::process::id()));
        let good = root.join("mods/good.jar");
        write_fake_zip(&good, "assets/demo/textures/block/tile.png");
        std::fs::write(root.join("mods/bad.jar"), b"damaged ZIP").unwrap();
        let error = mod_sources(&root).unwrap_err();
        assert!(error.contains("bad.jar"), "{error}");
        let root = root.canonicalize().unwrap();
        assert_eq!(
            root.parent().unwrap(),
            std::env::temp_dir().canonicalize().unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn active_mods_kubejs_and_enabled_packs_follow_the_selected_world_instance() {
        let root =
            std::env::temp_dir().join(format!("mcedit2-instance-discovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("instances/demo/.minecraft");
        let world = game.join("saves/TestWorld");
        std::fs::create_dir_all(&world).unwrap();
        let vanilla = candidate_paths(&game, "1.21.1")[2].clone();
        std::fs::create_dir_all(vanilla.parent().unwrap()).unwrap();
        std::fs::write(&vanilla, b"vanilla discovery placeholder").unwrap();
        let mods = game.join("mods");
        write_fake_zip(
            &mods.join("a-mod.jar"),
            "assets/a_mod/textures/block/machine.png",
        );
        write_fake_zip(
            &mods.join("z-mod.jar"),
            "assets/z_mod/blockstates/machine.json",
        );
        write_fake_zip(
            &mods.join("disabled.jar.disabled"),
            "assets/disabled/textures/block/unused.png",
        );
        write_fake_zip(&mods.join("library.jar"), "com/example/Test.class");
        std::fs::create_dir_all(game.join("kubejs/assets/kubejs/textures/block")).unwrap();
        write_fake_zip(
            &game.join("resourcepacks/low.zip"),
            "assets/minecraft/textures/block/stone.png",
        );
        write_fake_zip(
            &game.join("resourcepacks/high.zip"),
            "assets/minecraft/textures/block/stone.png",
        );
        write_fake_zip(
            &game.join("resourcepacks/not-enabled.zip"),
            "assets/minecraft/textures/block/stone.png",
        );
        std::fs::write(
            game.join("options.txt"),
            "resourcePacks:[\"vanilla\",\"file/low.zip\",\"file/high.zip\"]\n",
        )
        .unwrap();
        let stack = discover_stack(&world, "1.21.1", &[]).unwrap().unwrap();
        assert!(stack.vanilla);
        assert_eq!(stack.mod_count, 2);
        assert_eq!(stack.generated_count, 1);
        assert_eq!(stack.selected_count, 2);
        assert_eq!(
            stack.paths,
            vec![
                vanilla,
                mods.join("a-mod.jar"),
                mods.join("z-mod.jar"),
                game.join("kubejs"),
                game.join("resourcepacks/low.zip"),
                game.join("resourcepacks/high.zip"),
            ]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    fn selected_pack_options_refuse_escape_and_never_enable_all_installed_zips() {
        let root = std::env::temp_dir().join(format!(
            "mcedit2-selected-pack-paths-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("minecraft");
        write_fake_zip(
            &game.join("resourcepacks/good.zip"),
            "assets/a/textures/block/a.png",
        );
        write_fake_zip(&root.join("evil.zip"), "assets/a/textures/block/a.png");
        std::fs::write(
            game.join("options.txt"),
            "resourcePacks:[\"file/../evil.zip\",\"file/good.zip\",\"file/disabled.zip\"]\n",
        )
        .unwrap();
        let error = selected_packs(&game, None).unwrap_err();
        assert!(error.contains("文件名无效"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    fn missing_local_install_does_not_guess_a_nonexistent_jar() {
        let root = std::env::temp_dir().join(format!(
            "mcedit2-auto-assets-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        assert!(locate_in([root.as_path()], "9.9.9-absent")
            .unwrap()
            .is_none());
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Installed {
            real_chunk_mesh_matches_source_coordinates();
            real_instance_assets_finish_loading();
        }

        if category == crate::test_categories::Category::Sources {
            client_version_path_cannot_escape_world_ancestors();
            exact_launcher_paths_support_prism_and_vanilla_without_traversal();
            world_local_launcher_takes_priority_over_global_installs();
            large_instance_keeps_all_mods_and_enabled_packs_in_priority_order();
            damaged_mod_archive_stops_resource_discovery();
            active_mods_kubejs_and_enabled_packs_follow_the_selected_world_instance();
            selected_pack_options_refuse_escape_and_never_enable_all_installed_zips();
            missing_local_install_does_not_guess_a_nonexistent_jar();
        }
    }
}
