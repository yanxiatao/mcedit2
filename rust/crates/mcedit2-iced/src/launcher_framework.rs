//! 从当前 Prism / MultiMC 实例声明中读取准确版本的框架资源，不下载或执行 JAR。
use super::{
    is_simple_version, metadata as source_metadata, mod_archive_contains_assets, ordinary_file,
};
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_METADATA: u64 = 2 * 1024 * 1024;

fn read_json(path: &Path) -> Result<Value, String> {
    if !ordinary_file(path)? {
        return Err(format!("不是普通元数据文件：{}", path.display()));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| format!("{}：{error}", path.display()))?
        .take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}：{error}", path.display()))?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(format!("启动器元数据超过 2 MiB：{}", path.display()));
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("{}：{e}", path.display()))
}

/// Maven 坐标中的版本必须与实例组件一致；Forge 的传统坐标带 Minecraft 前缀。
fn artifact(
    entry: &Value,
    uid: &str,
    version: &str,
    minecraft: &str,
) -> Result<Option<PathBuf>, String> {
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .ok_or("框架库缺少 name")?;
    let (coordinate, extension) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<_> = coordinate.split(':').collect();
    let (group, id) = match uid {
        "net.neoforged" => ("net.neoforged", "neoforge"),
        "net.minecraftforge" => ("net.minecraftforge", "forge"),
        _ => return Err(format!("不支持的框架组件：{uid}")),
    };
    if parts.first() != Some(&group) || parts.get(1) != Some(&id) {
        return Ok(None);
    }
    if !(3..=4).contains(&parts.len()) || extension != "jar" {
        return Err(format!("框架 Maven 坐标无效：{name}"));
    }
    if parts.get(3).is_some_and(|v| !matches!(*v, "universal")) {
        return Ok(None);
    }
    let expected_forge = format!("{minecraft}-{version}");
    if parts[2] != version && !(uid == "net.minecraftforge" && parts[2] == expected_forge) {
        return Err(format!("框架库 {name} 与实例组件 {uid} {version} 不一致"));
    }
    let classifier = parts.get(3).map_or(String::new(), |v| format!("-{v}"));
    let derived = format!(
        "{}/{}/{}/{}-{}{}.jar",
        group.replace('.', "/"),
        id,
        parts[2],
        id,
        parts[2],
        classifier
    );
    let path = match entry.get("downloads") {
        None => derived.as_str(),
        Some(Value::Object(downloads)) => match downloads.get("artifact") {
            None => derived.as_str(),
            Some(Value::Object(artifact)) => match artifact.get("path") {
                None => derived.as_str(),
                Some(Value::String(path)) => path.as_str(),
                Some(_) => return Err(format!("框架库 {name} 的 artifact.path 必须为字符串")),
            },
            Some(_) => return Err(format!("框架库 {name} 的 artifact 必须为对象")),
        },
        Some(_) => return Err(format!("框架库 {name} 的 downloads 必须为对象")),
    };
    // 不允许 JSON 中的绝对路径、Windows 前缀或遍历覆盖当前库目录。
    if path.is_empty()
        || path.contains(['\\', ':'])
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || !path.ends_with(".jar")
    {
        return Err(format!("框架库资源路径无效：{path}"));
    }
    Ok(Some(PathBuf::from(path)))
}

fn metadata(instance: &Path, uid: &str, version: &str, roots: &[PathBuf]) -> Result<Value, String> {
    let local = instance.join("patches").join(format!("{uid}.json"));
    let path = if source_metadata(&local)?.is_some() {
        local
    } else {
        let mut selected = None;
        for root in roots {
            let candidate = root.join("meta").join(uid).join(format!("{version}.json"));
            if source_metadata(&candidate)?.is_some() {
                selected = Some(candidate);
                break;
            }
        }
        selected.ok_or_else(|| format!("缺少框架元数据：{uid} {version}"))?
    };
    let value = read_json(&path)?;
    if value.get("uid").and_then(Value::as_str) != Some(uid)
        || value.get("version").and_then(Value::as_str) != Some(version)
    {
        return Err(format!(
            "框架元数据与实例组件不一致：{}，要求 {uid} {version}",
            path.display()
        ));
    }
    Ok(value)
}

fn resolve(
    relative: &Path,
    roots: &[PathBuf],
    local: bool,
    instance: &Path,
) -> Result<PathBuf, String> {
    let bases: Vec<_> = if local {
        vec![instance.join("libraries")]
    } else {
        roots.iter().map(|p| p.join("libraries")).collect()
    };
    for base in bases {
        let path = if local {
            base.join(relative.file_name().ok_or("本地框架库缺少文件名")?)
        } else {
            base.join(relative)
        };
        if source_metadata(&path)?.is_none() {
            continue;
        }
        if !ordinary_file(&path)? {
            return Err(format!("框架 JAR 不是普通文件：{}", path.display()));
        }
        let canonical = path.canonicalize().map_err(|e| e.to_string())?;
        if !canonical.starts_with(base.canonicalize().map_err(|e| e.to_string())?) {
            return Err(format!("框架 JAR 越过所声明的库目录：{}", path.display()));
        }
        if !mod_archive_contains_assets(&canonical)
            .map_err(|error| format!("无法索引框架资源 {}：{error}", canonical.display()))?
        {
            return Err(format!("框架 JAR 无有效模型或纹理资源：{}", path.display()));
        }
        return Ok(canonical);
    }
    Err(format!(
        "未安装当前实例声明的框架资源：{}；请提供准确版本的 JAR",
        relative.display()
    ))
}

fn component(
    instance: &Path,
    uid: &str,
    version: &str,
    minecraft: &str,
    roots: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let data = metadata(instance, uid, version, roots)?;
    let mut paths = Vec::new();
    let mut declared = false;
    for key in ["libraries", "mavenFiles", "+libraries"] {
        let Some(entries) = data.get(key) else {
            continue;
        };
        let entries = entries
            .as_array()
            .ok_or_else(|| format!("框架 {key} 必须是数组"))?;
        for entry in entries {
            if let Some(relative) = artifact(entry, uid, version, minecraft)? {
                declared = true;
                let hint = entry
                    .get("MMC-hint")
                    .map(|value| value.as_str().ok_or("框架库 MMC-hint 必须为字符串"))
                    .transpose()?;
                let local = hint == Some("local");
                let path = resolve(&relative, roots, local, instance)?;
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
    }
    if !declared {
        return Err(format!("框架元数据没有可读取的原生资源库：{uid} {version}"));
    }
    Ok(paths)
}

pub(super) fn discover(
    game: &Path,
    minecraft: &str,
    roots: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let Some(instance) = game.parent() else {
        return Ok(Vec::new());
    };
    let manifest = instance.join("mmc-pack.json");
    if source_metadata(&manifest)?.is_none() {
        return Ok(Vec::new());
    }
    let data = read_json(&manifest)?;
    let components = data
        .get("components")
        .and_then(Value::as_array)
        .ok_or("实例缺少 components 数组")?;
    let mut identifiers = std::collections::BTreeSet::new();
    let mut frameworks = Vec::new();
    for part in components {
        let uid = part
            .get("uid")
            .and_then(Value::as_str)
            .filter(|uid| !uid.is_empty())
            .ok_or("实例组件缺少实际 uid")?;
        if !identifiers.insert(uid) {
            return Err(format!("实例组件 uid 重复：{uid}"));
        }
        if matches!(uid, "net.neoforged" | "net.minecraftforge") {
            frameworks.push((part, uid));
        }
    }
    if frameworks.is_empty() {
        return Ok(Vec::new());
    }
    let mc = components
        .iter()
        .find(|v| v.get("uid").and_then(Value::as_str) == Some("net.minecraft"))
        .and_then(|v| v.get("version"))
        .and_then(Value::as_str);
    if mc != Some(minecraft) {
        return Err(format!(
            "实例 Minecraft 版本 {:?} 与存档 {minecraft} 不一致，未混入框架资源",
            mc
        ));
    }
    let mut out = Vec::new();
    for (part, uid) in frameworks {
        let version = part
            .get("version")
            .and_then(Value::as_str)
            .filter(|v| is_simple_version(v))
            .ok_or_else(|| format!("框架组件版本无效：{uid}"))?;
        for path in component(instance, uid, version, minecraft, roots)? {
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textures::{java_obj_tests::Fixture, LoadControl, TextureBank};
    use mcedit2_world::sections::State;
    use std::io::Write;
    const VERSION: &str = "21.1.234-test";
    const RELATIVE: &str =
        "net/neoforged/neoforge/21.1.234-test/neoforge-21.1.234-test-universal.jar";

    fn zip(path: &Path, files: &[(&str, &[u8])]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    fn png(color: [u8; 4]) -> Vec<u8> {
        use image::ImageEncoder;
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&color, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }
    fn metadata_value() -> Value {
        serde_json::json!({"uid":"net.neoforged","version":VERSION,"mavenFiles":[
            {"name":format!("net.neoforged:neoforge:{VERSION}:installer")},
            {"name":format!("net.neoforged:neoforge:{VERSION}:universal"),"downloads":{"artifact":{"path":RELATIVE}}}
        ]})
    }
    fn setup() -> (Fixture, PathBuf, PathBuf) {
        let fixture = Fixture::new();
        let launcher = fixture.path.join("launcher");
        let game = launcher.join("instances/demo/minecraft");
        std::fs::create_dir_all(game.join("saves/TestWorld")).unwrap();
        fixture.put(
            "launcher/instances/demo/mmc-pack.json",
            serde_json::to_string(&serde_json::json!({"components":[
                {"uid":"net.minecraft","version":"1.21.1"},
                {"uid":"net.neoforged","version":VERSION}
            ]}))
            .unwrap()
            .as_bytes(),
        );
        fixture.put(
            &format!("launcher/meta/net.neoforged/{VERSION}.json"),
            &serde_json::to_vec(&metadata_value()).unwrap(),
        );
        (fixture, launcher, game)
    }

    #[test]
    fn declared_framework_is_loaded_before_mod_overlays_and_resolves_model_parent() {
        let (_fixture, launcher, game) = setup();
        let red = png([200, 20, 10, 255]);
        let green = png([10, 200, 20, 255]);
        zip(&launcher.join("versions/1.21.1/1.21.1.jar"),&[
            ("assets/minecraft/models/block/block.json",br##"{"textures":{"all":"neoforge:block/panel"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all"}}}]}"##),
        ]);
        let framework = launcher.join("libraries").join(RELATIVE);
        zip(
            &framework,
            &[
                (
                    "assets/neoforge/models/block/default.json",
                    br#"{"parent":"minecraft:block/block"}"#,
                ),
                ("assets/neoforge/textures/block/panel.png", &red),
            ],
        );
        zip(
            &game.join("mods/demo.jar"),
            &[
                (
                    "assets/demo/blockstates/machine.json",
                    br#"{"variants":{"":{"model":"demo:block/machine"}}}"#,
                ),
                (
                    "assets/demo/models/block/machine.json",
                    br#"{"parent":"neoforge:block/default"}"#,
                ),
                ("assets/neoforge/textures/block/panel.png", &green),
            ],
        );
        let stack =
            crate::auto_assets::discover_stack(&game.join("saves/TestWorld"), "1.21.1", &[])
                .unwrap()
                .unwrap();
        assert!(stack.warnings.is_empty(), "{:?}", stack.warnings);
        assert_eq!(stack.framework_count, 1);
        assert_eq!(stack.mod_count, 1);
        assert_eq!(stack.paths[1], framework.canonicalize().unwrap());
        let bank = TextureBank::open_stack_java_controlled(
            &stack.paths,
            &LoadControl {
                all_png: false,
                ..Default::default()
            },
        )
        .unwrap();
        let state = State {
            name: "demo:machine".into(),
            properties: vec![],
        };
        assert_eq!(bank.geometry(&state).unwrap().len(), 1);
        let (uv, _, page) = bank
            .atlas_face_style_at(&state, crate::isometric::Face::Top, 0, (0, 0, 0))
            .unwrap();
        assert_eq!(bank.atlas_pixel(uv[0], page).unwrap(), [10, 200, 20, 255]);
    }

    #[test]
    fn missing_exact_framework_does_not_select_another_installed_version() {
        let (_fixture, launcher, game) = setup();
        zip(
            &launcher
                .join("libraries/net/neoforged/neoforge/21.1.999/neoforge-21.1.999-universal.jar"),
            &[("assets/neoforge/models/block/default.json", b"{}")],
        );
        let error = discover(&game, "1.21.1", &[launcher]).unwrap_err();
        assert!(error.contains("未安装当前实例声明"), "{error}");
        assert!(error.contains(VERSION), "{error}");
    }

    #[test]
    fn local_patch_and_local_library_override_global_component_metadata() {
        let (fixture, launcher, game) = setup();
        let data = serde_json::json!({"uid":"net.neoforged","version":VERSION,"libraries":[
            {"name":format!("net.neoforged:neoforge:{VERSION}:universal"),"MMC-hint":"local"}
        ]});
        fixture.put(
            "launcher/instances/demo/patches/net.neoforged.json",
            &serde_json::to_vec(&data).unwrap(),
        );
        let local = game
            .parent()
            .unwrap()
            .join(format!("libraries/neoforge-{VERSION}-universal.jar"));
        zip(
            &local,
            &[("assets/neoforge/models/block/default.json", b"{}")],
        );
        assert_eq!(
            discover(&game, "1.21.1", &[launcher]).unwrap(),
            vec![local.canonicalize().unwrap()]
        );
    }

    #[test]
    fn metadata_version_and_unsafe_artifact_paths_are_refused() {
        let (fixture, launcher, game) = setup();
        for path in [
            "../outside.jar",
            "/outside.jar",
            "E:/outside.jar",
            "sub\\outside.jar",
            "https://host/outside.jar",
        ] {
            let mut data = metadata_value();
            data["mavenFiles"][1]["downloads"]["artifact"]["path"] = Value::String(path.into());
            fixture.put(
                &format!("launcher/meta/net.neoforged/{VERSION}.json"),
                &serde_json::to_vec(&data).unwrap(),
            );
            let error = discover(&game, "1.21.1", std::slice::from_ref(&launcher)).unwrap_err();
            assert!(error.contains("资源路径无效"), "{error}");
        }
        let mut data = metadata_value();
        data["version"] = Value::String("21.1.999".into());
        fixture.put(
            &format!("launcher/meta/net.neoforged/{VERSION}.json"),
            &serde_json::to_vec(&data).unwrap(),
        );
        let error = discover(&game, "1.21.1", std::slice::from_ref(&launcher)).unwrap_err();
        assert!(error.contains("元数据与实例组件不一致"), "{error}");
        let error = discover(&game, "1.20.1", &[launcher]).unwrap_err();
        assert!(error.contains("与存档 1.20.1 不一致"), "{error}");
    }

    #[test]
    fn legacy_forge_coordinates_keep_the_minecraft_prefix() {
        let entry = serde_json::json!({"name":"net.minecraftforge:forge:1.20.1-47.4.0:universal"});
        assert_eq!(
            artifact(&entry, "net.minecraftforge", "47.4.0", "1.20.1").unwrap(),
            Some(PathBuf::from(
                "net/minecraftforge/forge/1.20.1-47.4.0/forge-1.20.1-47.4.0-universal.jar"
            ))
        );
        assert!(artifact(&entry, "net.minecraftforge", "47.3.0", "1.20.1").is_err());
    }

    #[test]
    #[ignore = "requires local GPU / installed Minecraft resources"]

    fn real_framework_assets_restore_native_obj_and_pneumatic_models() {
        let world = mcedit2_world::inspect_world(PathBuf::from(
            std::env::var_os("MCEDIT2_TEST_WORLD").unwrap(),
        ))
        .unwrap();
        let mut stack = crate::auto_assets::discover_stack(
            &world.root,
            world.version_name.as_deref().unwrap(),
            &[],
        )
        .unwrap()
        .unwrap();
        eprintln!(
            "自动发现框架 {}；诊断 {:?}",
            stack.framework_count, stack.warnings
        );
        let reference = PathBuf::from(std::env::var_os("MCEDIT2_FRAMEWORK_JAR").unwrap());
        stack.paths.insert(usize::from(stack.vanilla), reference);
        let bank = TextureBank::open_stack_java_controlled(
            &stack.paths,
            &LoadControl {
                all_png: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !bank
                .warnings
                .iter()
                .any(|w| w.contains("父模型资源不存在") && w.contains("model:neoforge:default")),
            "{:?}",
            bank.warnings
        );
        let source = State {
            name: "chancecubes:chance_icosahedron".into(),
            properties: vec![],
        };
        let obj = bank.static_obj_at(&source, (0, 0, 0)).unwrap();
        assert!(!obj.is_empty());
        eprintln!("真实几何 {}：{} OBJ 三角形", source.name, obj.len());
        for name in ["pneumaticcraft:etching_tank", "pneumaticcraft:uv_light_box"] {
            let state = State {
                name: name.into(),
                properties: if name.ends_with("uv_light_box") {
                    vec![
                        ("facing".into(), "north".into()),
                        ("loaded".into(), "false".into()),
                    ]
                } else {
                    vec![]
                },
            };
            let geometry = bank.geometry(&state).unwrap();
            assert!(!geometry.is_empty());
            eprintln!("真实几何 {name}：{} JSON 元素", geometry.len());
            if name.ends_with("uv_light_box") {
                let loaded = State {
                    properties: vec![
                        ("facing".into(), "north".into()),
                        ("loaded".into(), "true".into()),
                    ],
                    ..state
                };
                let loaded_geometry = bank.geometry(&loaded).unwrap();
                assert!(
                    loaded_geometry.len() > geometry.len(),
                    "loaded:true 应添加真实 PCB 子模型"
                );
                eprintln!(
                    "真实几何 {name} loaded=true：{} JSON 元素",
                    loaded_geometry.len()
                );
            }
        }
        eprintln!(
            "框架资源叠加后：{} PNG / {} 模型 / {} 条诊断",
            bank.count(),
            bank.model_count(),
            bank.warnings.len()
        );
        crate::gpu_scene::hardware_tests::assert_source_models_render(
            std::sync::Arc::new(bank),
            &[
                source,
                State {
                    name: "pneumaticcraft:etching_tank".into(),
                    properties: vec![],
                },
                State {
                    name: "pneumaticcraft:uv_light_box".into(),
                    properties: vec![
                        ("facing".into(), "north".into()),
                        ("loaded".into(), "true".into()),
                    ],
                },
            ],
        );
    }
}
