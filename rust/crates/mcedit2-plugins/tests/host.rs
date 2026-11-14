use mcedit2_plugins::{
    capture,
    catalog::{Catalog, PluginSource},
    prepare_output, RunOutput, PROTOCOL_VERSION,
};
use mcedit2_world::{
    editing::{Bounds, Session},
    inspect_world,
    sections::decode_sections,
    RegionIndex,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static COUNTER: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mcedit2-plugins-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("清理插件测试目录失败：{error}");
        }
    }
}

fn catalog_enable_disable_and_reload_preserve_explicit_state() {
    let directory = Directory::new();
    let path = directory.0.join("example.py");
    fs::write(&path, "def perform(level,box,options): pass").unwrap();
    let mut catalog = Catalog::scan(&directory.0).unwrap();
    assert_eq!(catalog.entries.len(), 1);
    assert!(!catalog.entries[0].enabled);
    let id = catalog.entries[0].source.id.clone();
    catalog.set_enabled(&id, true).unwrap();
    assert!(Catalog::scan(&directory.0).unwrap().entries[0].enabled);
    let old = catalog.entries[0].source.clone();
    fs::write(&path, "def perform(level,box,options): print('changed')").unwrap();
    assert!(old.verify_unchanged().is_err());
    let mut reloaded = Catalog::scan(&directory.0).unwrap();
    assert_eq!(reloaded.entries[0].source.id, id);
    assert_ne!(reloaded.entries[0].source.digest, old.digest);
    reloaded.set_enabled(&id, false).unwrap();
    assert!(!Catalog::scan(&directory.0).unwrap().entries[0].enabled);
    fs::write(directory.0.join(".mcedit2-enabled.json"), b"invalid").unwrap();
    assert!(Catalog::scan(&directory.0).is_err());
}

fn invalid_sources_are_reported_without_execution() {
    let directory = Directory::new();
    fs::write(directory.0.join("bad.py"), [0xff, 0xfe]).unwrap();
    fs::write(directory.0.join("ignored.pyc"), b"not executable").unwrap();
    let catalog = Catalog::scan(&directory.0).unwrap();
    assert!(catalog.entries.is_empty());
    assert_eq!(catalog.diagnostics.len(), 1);
    assert!(PluginSource::read(&directory.0.join("ignored.pyc")).is_err());
}

fn package_snapshot_tracks_all_modules_and_rejects_ambiguous_names() {
    let directory = Directory::new();
    let package = directory.0.join("包插件");
    fs::create_dir(&package).unwrap();
    fs::create_dir(package.join("tools")).unwrap();
    fs::write(package.join("__init__.py"), "from .tools import main").unwrap();
    fs::write(package.join("tools/__init__.py"), "from .main import value").unwrap();
    fs::write(package.join("tools/main.py"), "value = 41").unwrap();
    let catalog = Catalog::scan(&directory.0).unwrap();
    assert!(catalog.diagnostics.is_empty());
    let source = &catalog.entries[0].source;
    assert!(source.package);
    assert_eq!(source.modules.len(), 2);
    assert!(source.modules["tools"].is_package);
    assert!(!source.modules["tools.main"].is_package);
    let request = mcedit2_plugins::RunRequest::inspect(source);
    request.validate().unwrap();
    assert!(request.filename.ends_with("__init__.py"));
    fs::write(package.join("tools/main.py"), "value = 42").unwrap();
    assert!(source.verify_unchanged().is_err());
    assert_ne!(PluginSource::read(&package).unwrap().digest, source.digest);
    fs::write(package.join("tools.py"), "value = 43").unwrap();
    assert!(PluginSource::read(&package).unwrap_err().contains("冲突"));
}

fn package_rejects_invalid_utf8_before_execution() {
    let directory = Directory::new();
    let package = directory.0.join("example");
    fs::create_dir(&package).unwrap();
    fs::write(
        package.join("__init__.py"),
        "raise RuntimeError('不应执行')",
    )
    .unwrap();
    fs::write(package.join("helper.py"), [0xff]).unwrap();
    assert!(PluginSource::read(&package).unwrap_err().contains("UTF-8"));
}

fn original_selection() -> (Session, Bounds, PathBuf, Vec<u8>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test_files/AnvilWorld_1.8");
    let world = inspect_world(&root).unwrap();
    let dimension = world.dimensions[0].clone();
    let region = dimension.region_dir.join("r.-1.0.mca");
    let original = fs::read(&region).unwrap();
    let index = RegionIndex::open(&region).unwrap();
    for z in 0..32u8 {
        for x in 0..32u8 {
            let Some(bytes) = index
                .read_chunk_at(i32::from(x) - 32, i32::from(z))
                .unwrap()
            else {
                continue;
            };
            let chunk = decode_sections(&bytes).unwrap();
            for section in &chunk.sections {
                let point = (
                    (i32::from(x) - 32) * 16,
                    i32::from(section.y) * 16,
                    i32::from(z) * 16,
                );
                if !chunk.has_block_entity(point.0, point.1, point.2) {
                    return (
                        Session::new(world, dimension.name).unwrap(),
                        Bounds::new(point, point).unwrap(),
                        region,
                        original,
                    );
                }
            }
        }
    }
    panic!("原始样本必须存在可检查方块");
}

fn python_result_uses_original_preview_undo_and_stale_source_guards() {
    let (mut session, bounds, region, original) = original_selection();
    let snapshot = capture(&session, bounds).unwrap();
    let mut changed = snapshot.blocks[0].clone();
    changed.name = if changed.name == "numeric:1" {
        "numeric:2"
    } else {
        "numeric:1"
    }
    .into();
    let output = RunOutput {
        structure: None,
        tool_pick: None,
        object_bindings: None,
        tool_scene: None,
        inspector_operation: None,
        content: None,
        protocol_version: PROTOCOL_VERSION,
        commands: vec![],
        messages: vec![],
        blocks: Some(vec![changed]),
    };
    let preview = prepare_output(&session, &snapshot, &output)
        .unwrap()
        .unwrap();
    assert_eq!(session.pending_cells(), 0);
    assert_eq!(session.commit(preview).unwrap(), 1);
    assert!(prepare_output(&session, &snapshot, &output)
        .unwrap_err()
        .contains("已改变"));
    assert!(session.undo().unwrap());
    assert_eq!(capture(&session, bounds).unwrap().blocks, snapshot.blocks);
    assert!(session.redo().unwrap());
    assert_eq!(fs::read(region).unwrap(), original);
}

#[test]
fn plugin_host_and_protocol() {
    large_request_and_numeric_biome_frames();
    catalog_enable_disable_and_reload_preserve_explicit_state();
    invalid_sources_are_reported_without_execution();
    package_snapshot_tracks_all_modules_and_rejects_ambiguous_names();
    package_rejects_invalid_utf8_before_execution();
    python_result_uses_original_preview_undo_and_stale_source_guards();
}

fn large_request_and_numeric_biome_frames() {
    use mcedit2_plugins::{transport, RunRequest, RunnerReply};
    let directory = Directory::new();
    let option_output = RunOutput {
        tool_pick: None,
        object_bindings: None,
        tool_scene: None,
        protocol_version: PROTOCOL_VERSION,
        commands: vec![mcedit2_plugins::PluginCommand {
            id: "large_options".into(),
            name: "大选项".into(),
            preview_on_change: false,
            inspector_ids: Vec::new(),
            inspector_source: None,
            gesture_input: false,
            persistent_tool: false,
            action: None,
            icon: None,
            options: vec![serde_json::json!({"value":"a".repeat(128*1024)})],
        }],
        messages: vec![],
        blocks: None,
        structure: None,
        inspector_operation: None,
        content: None,
    };
    assert!(option_output
        .validate_with_budget(mcedit2_plugins::work_budget(1024 * 1024).unwrap())
        .unwrap_err()
        .contains("预算"));
    let path = directory.0.join("large.py");
    fs::write(&path, "def perform(level,box,options): pass").unwrap();
    let source = PluginSource::read(&path).unwrap();
    let (session, bounds, _, _) = original_selection();
    let mut snapshot = capture(&session, bounds).unwrap();
    snapshot.size = [16, 128, 16];
    snapshot.blocks = vec![snapshot.blocks[0].clone(); 32768];
    let mut request = RunRequest::inspect(&source);
    request.command = Some("perform".into());
    request.snapshot = Some(snapshot.clone());
    request.validate().unwrap();
    let mut wire = Vec::new();
    transport::write(&mut wire, &request, request.work_budget_bytes).unwrap();
    let decoded: RunRequest =
        transport::read(&mut wire.as_slice(), request.work_budget_bytes).unwrap();
    assert_eq!(decoded.snapshot.unwrap().blocks, snapshot.blocks);
    request.work_budget_bytes = 1024 * 1024;
    assert!(request.validate().unwrap_err().contains("预算"));
    snapshot.numeric_biomes = Some(mcedit2_world::numeric_biome::Volume {
        size: snapshot.size,
        cells: vec![7; 32768],
        data_version: snapshot.data_version,
    });
    snapshot
        .numeric_biomes
        .as_ref()
        .unwrap()
        .validate(snapshot.size)
        .unwrap();
    let output = RunOutput {
        protocol_version: PROTOCOL_VERSION,
        tool_pick: None,
        object_bindings: None,
        tool_scene: None,
        commands: vec![],
        messages: vec![],
        blocks: None,
        structure: Some(snapshot.clone()),
        inspector_operation: None,
        content: None,
    };
    let reply = RunnerReply::Success { output };
    let mut wire = Vec::new();
    transport::write(&mut wire, &reply, 256 * 1024 * 1024).unwrap();
    let decoded: RunnerReply = transport::read(&mut wire.as_slice(), 256 * 1024 * 1024).unwrap();
    let RunnerReply::Success { output } = decoded else {
        panic!("传输应成功")
    };
    assert_eq!(output.structure.unwrap(), snapshot);
    wire.pop();
    assert!(transport::read::<serde_json::Value>(&mut wire.as_slice(), 256 * 1024 * 1024).is_err());
}
