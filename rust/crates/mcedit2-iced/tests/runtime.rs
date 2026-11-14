use mcedit2_plugins::{PythonModule, RunLimits, RunRequest, PROTOCOL_VERSION};
use mcedit2_world::{clipboard::BlockClipboard, editing::parse_state};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

fn request(source: &str, command: Option<&str>) -> RunRequest {
    RunRequest {
        protocol_version: PROTOCOL_VERSION,
        work_budget_bytes: 256 * 1024 * 1024,
        source: source.into(),
        filename: "插件测试.py".into(),
        package: false,
        modules: BTreeMap::new(),
        command: command.map(str::to_owned),
        options: BTreeMap::new(),
        selection: None,
        read_only: false,
        form_action: None,
        form_initialize: false,
        form_source: None,
        tool_event: None,
        gesture: None,
        tool_phase: None,
        object_receipt: None,
        native_source: None,
        inspector: None,
        snapshot: command.map(|_| BlockClipboard {
            painting_variants: Default::default(),
            format: "MCEDIT2_RUST_BLOCK_ONLY_CLIPBOARD".into(),
            version: 1,
            source_root: "fixture".into(),
            dimension: "minecraft:overworld".into(),
            entity_dimension: None,
            data_version: Some(3955),
            source_release: Some("1.21.1".into()),
            source_bound: false,
            source_file: None,
            numeric: false,
            source_min: [-2, -64, 3],
            size: [2, 1, 1],
            offset: [0; 3],
            block_entities: Vec::new(),
            scheduled_ticks: Vec::new(),
            entities: Vec::new(),
            biomes: None,
            numeric_biomes: None,
            metadata: None,
            mask: None,
            blocks: vec![parse_state("minecraft:stone").unwrap(); 2],
        }),
    }
}

const LEGACY: &str = r#"
from mcedit2.plugins import registerPluginCommand, SimpleCommandPlugin
from PySide import QtGui
@registerPluginCommand
class Replace(SimpleCommandPlugin):
    displayName = '替换石头'
    options = [{'name': 'target', 'type': 'blocktype', 'value': 'minecraft:glass'}]
    def perform(self, dimension, selection, options):
        stone = dimension.blocktypes['minecraft:stone']
        for position in selection.positions:
            if dimension.getBlock(*position) == stone:
                dimension.setBlock(*position, options['target'])
        QtGui.QMessageBox.information(None, self.tr('完成'), str(selection.volume))
"#;

fn discovers_legacy_command_and_executes_source_selection() {
    let inspected = mcedit2_python::execute(&request(LEGACY, None)).unwrap();
    assert_eq!(inspected.commands[0].id, "Replace");
    assert_eq!(inspected.commands[0].name, "替换石头");
    assert!(inspected.blocks.is_none());
    let output = mcedit2_python::execute(&request(LEGACY, Some("Replace"))).unwrap();
    assert!(output
        .blocks
        .unwrap()
        .iter()
        .all(|v| v.name == "minecraft:glass"));
    assert!(output.messages.iter().any(|v| v.text == "完成: 2"));
}

fn native_decorator_and_classic_filter_share_world_adapter() {
    let native = r#"
from mcedit2.api import register_command
@register_command('原生命令', options=[{'type': 'bool', 'name': 'run', 'value': True}])
def replace(context):
    if context.options['run']:
        context.dimension.fillBlocks(context.selection, 'minecraft:dirt')
    context.log('done')
"#;
    let output = mcedit2_python::execute(&request(native, Some("replace"))).unwrap();
    assert_eq!(output.blocks.unwrap()[0].name, "minecraft:dirt");
    let classic = "displayName='过滤器'\ninputs=[('times', (1, 1, 5))]\ndef perform(level,box,options):\n level.fillBlocks(box,'minecraft:glass')\n";
    let output = mcedit2_python::execute(&request(classic, Some("perform"))).unwrap();
    assert_eq!(output.blocks.unwrap()[1].name, "minecraft:glass");
}

fn invalid_options_exception_and_unsupported_dependencies_are_errors() {
    let mut input = request(LEGACY, Some("Replace"));
    input.options.insert("unknown".into(), true.into());
    assert!(mcedit2_python::execute(&input)
        .unwrap_err()
        .contains("未声明的选项"));
    let unsupported = "from PySide.QtGui import QPainter";
    assert!(mcedit2_python::execute(&request(unsupported, None))
        .unwrap_err()
        .contains("QPainter"));
    assert!(mcedit2_python::execute(&request("import numpy", None)).is_err());
    assert!(mcedit2_python::execute(&request("print 'Python 2 only'", None)).is_err());
    let bad = LEGACY.replace(
        "stone = dimension.blocktypes['minecraft:stone']",
        "raise RuntimeError('插件失败')",
    );
    assert!(mcedit2_python::execute(&request(&bad, Some("Replace")))
        .unwrap_err()
        .contains("插件失败"));
}

fn out_of_selection_access_refuses_and_empty_entities_are_readable() {
    let outside = LEGACY.replace(
        "stone = dimension.blocktypes['minecraft:stone']",
        "dimension.setBlock(999, 0, 0, 'minecraft:dirt')",
    );
    assert!(mcedit2_python::execute(&request(&outside, Some("Replace")))
        .unwrap_err()
        .contains("源选区"));
    let entity = LEGACY.replace(
        "stone = dimension.blocktypes['minecraft:stone']",
        "assert list(dimension.getEntities(selection)) == []\n        stone = dimension.blocktypes['minecraft:stone']",
    );
    let output = mcedit2_python::execute(&request(&entity, Some("Replace"))).unwrap();
    assert!(output
        .blocks
        .unwrap()
        .iter()
        .all(|state| state.name == "minecraft:glass"));
}

fn numeric_id_and_metadata_are_preserved_without_inventing_registry_ids() {
    let mut input = request("def perform(level,box,options):\n for x,y,z in box.positions:\n  level.setBlockID(x,y,z,301)\n  level.setBlockData(x,y,z,7)\n", Some("perform"));
    let snapshot = input.snapshot.as_mut().unwrap();
    snapshot.numeric = true;
    snapshot.data_version = None;
    snapshot.source_release = Some("1.12.2".into());
    snapshot.blocks = vec![parse_state("numeric:1[data=0]").unwrap(); 2];
    let output = mcedit2_python::execute(&input).unwrap();
    assert_eq!(
        output.blocks.unwrap(),
        vec![parse_state("numeric:301[data=7]").unwrap(); 2]
    );
}

fn package_request(command: Option<&str>) -> RunRequest {
    let mut input = request(
        r#"
from . import helpers
from .tools.paint import paint
from .helpers import registry
assert registry is helpers.registry
assert helpers.initialized == 1
"#,
        command,
    );
    input.package = true;
    input.filename = "包/__init__.py".into();
    input.modules = [
        (
            "helpers",
            "initialized = 1\nregistry = []\nfrom . import cycle\n",
            false,
        ),
        (
            "cycle",
            "from . import helpers\nassert helpers.initialized == 1\n",
            false,
        ),
        ("tools", "from ..helpers import registry\n", true),
        (
            "tools.paint",
            r#"
from mcedit2.api import register_command
from ..helpers import registry
@register_command('包内绘制')
def paint(context):
    registry.append('used')
    context.dimension.fillBlocks(context.selection, 'minecraft:glass')
    context.log(str(len(registry)))
"#,
            false,
        ),
    ]
    .into_iter()
    .map(|(name, source, is_package)| {
        (
            name.into(),
            PythonModule {
                source: source.into(),
                filename: format!("包/{name}.py"),
                is_package,
            },
        )
    })
    .collect();
    input
}

fn package_relative_imports_nested_modules_cycles_and_process_snapshot() {
    let input = package_request(Some("paint"));
    input.validate().unwrap();
    let output = mcedit2_python::execute(&input).unwrap();
    assert_eq!(output.commands[0].name, "包内绘制");
    assert_eq!(output.blocks.unwrap()[0].name, "minecraft:glass");
    assert_eq!(
        output
            .messages
            .iter()
            .filter(|message| message.text == "1")
            .count(),
        1
    );
    let output = mcedit2_plugins::run(
        Path::new(env!("CARGO_BIN_EXE_mcedit2")),
        &input,
        &RunLimits::default(),
    )
    .unwrap();
    assert_eq!(output.blocks.unwrap()[1].name, "minecraft:glass");
}

fn invalid_package_missing_dependency_and_module_exception_refuse_execution() {
    let mut input = package_request(None);
    input.modules.get_mut("tools.paint").unwrap().source = "from .. import missing".into();
    assert!(mcedit2_python::execute(&input)
        .unwrap_err()
        .contains("missing"));
    input.modules.get_mut("tools.paint").unwrap().source =
        "raise RuntimeError('子模块加载失败')".into();
    assert!(mcedit2_python::execute(&input)
        .unwrap_err()
        .contains("子模块加载失败"));
    input.modules.remove("tools");
    assert!(input.validate().unwrap_err().contains("父包"));
    input = package_request(None);
    input.package = false;
    assert!(input.validate().is_err());
}

fn separate_process_executes_protocol_and_kills_infinite_plugin() {
    let exe = Path::new(env!("CARGO_BIN_EXE_mcedit2"));
    let output = mcedit2_plugins::run(
        exe,
        &request(LEGACY, Some("Replace")),
        &RunLimits::default(),
    )
    .unwrap();
    assert_eq!(output.blocks.unwrap()[0].name, "minecraft:glass");
    let limits = RunLimits {
        timeout: Duration::from_millis(350),
        transfer_budget: 256 * 1024 * 1024,
        ..RunLimits::default()
    };
    assert!(
        mcedit2_plugins::run(exe, &request("while True: pass", None), &limits)
            .unwrap_err()
            .contains("超时")
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let worker = std::thread::spawn(move || {
        mcedit2_plugins::run(
            exe,
            &request("while True: pass", None),
            &RunLimits {
                timeout: Duration::from_secs(5),
                transfer_budget: 256 * 1024 * 1024,
                cancelled: flag,
            },
        )
    });
    std::thread::sleep(Duration::from_millis(200));
    cancelled.store(true, Ordering::Release);
    assert!(worker.join().unwrap().unwrap_err().contains("取消"));
}

#[test]
fn rustpython_commands_and_isolation() {
    unified_entrypoint_audit_and_help();
    large_process_selection_and_generator_budget();
    shaped_process_selection();
    shaped_chunk_grids_share_real_numeric_block_and_biome_storage();
    discovers_legacy_command_and_executes_source_selection();
    native_decorator_and_classic_filter_share_world_adapter();
    invalid_options_exception_and_unsupported_dependencies_are_errors();
    out_of_selection_access_refuses_and_empty_entities_are_readable();
    numeric_id_and_metadata_are_preserved_without_inventing_registry_ids();
    package_relative_imports_nested_modules_cycles_and_process_snapshot();
    invalid_package_missing_dependency_and_module_exception_refuse_execution();
    let walked = mcedit2_python::execute(&request(
        r#"
from mcedit2.api import register_command
@register_command('NBT 遍历检查')
def walk(context):
    from mceditlib import nbt
    root = nbt.TAG_Compound()
    child = nbt.TAG_Compound()
    child['value'] = nbt.TAG_Int(7)
    left = nbt.TAG_Compound()
    right = nbt.TAG_Compound()
    left['shared'] = child
    right['shared'] = child
    root['left'] = left
    root['right'] = right
    rows = list(nbt.walk(root))
    assert len(rows) == 6
    assert [path for name, tag, path in rows if name == 'value'] == [['left', 'shared'], ['right', 'shared']]
    node = root
    for index in range(80):
        node['next'] = nbt.TAG_Compound()
        node = node['next']
    node['leaf'] = nbt.TAG_String('deep')
    assert any(name == 'leaf' and len(path) == 80 for name, tag, path in nbt.walk(root))
    child['cycle'] = root
    try:
        list(nbt.walk(root))
    except ValueError as error:
        assert '循环引用' in str(error)
    else:
        raise AssertionError('循环 NBT 必须正常报错')
"#,
        Some("walk"),
    ))
    .unwrap();
    assert!(walked.messages.is_empty());
    separate_process_executes_protocol_and_kills_infinite_plugin();
}

fn unified_entrypoint_audit_and_help() {
    use std::process::Command;
    let executable = env!("CARGO_BIN_EXE_mcedit2");
    let help = Command::new(executable).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8(help.stdout)
        .unwrap()
        .contains("--inspect"));
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../test_files/AnvilWorld_1.8");
    let world = mcedit2_world::inspect_world(&sample).unwrap();
    let result = Command::new(executable)
        .arg("--inspect")
        .arg(&sample)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains(&world.root.display().to_string()));
    for dimension in &world.dimensions {
        assert!(output.contains(&format!(
            "{} regions={} chunks={}",
            dimension.name, dimension.regions, dimension.chunks
        )));
    }
    let missing = Command::new(executable).arg("--inspect").output().unwrap();
    assert_eq!(missing.status.code(), Some(2));
    let refused = Command::new(executable)
        .arg("--inspect")
        .arg(sample.join("不存在的世界"))
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(1));
}

fn large_process_selection_and_generator_budget() {
    let exe = Path::new(env!("CARGO_BIN_EXE_mcedit2"));
    let mut input = request(LEGACY, Some("Replace"));
    let snapshot = input.snapshot.as_mut().unwrap();
    snapshot.size = [16, 128, 16];
    snapshot.blocks = vec![parse_state("minecraft:stone").unwrap(); 32768];
    let output = mcedit2_plugins::run(
        exe,
        &input,
        &RunLimits {
            timeout: Duration::from_secs(120),
            ..RunLimits::default()
        },
    )
    .unwrap();
    let blocks = output.blocks.unwrap();
    assert_eq!(blocks.len(), 32768);
    assert!(blocks.iter().all(|state| state.name == "minecraft:glass"));
    assert!(output
        .messages
        .iter()
        .any(|message| message.text == "完成: 32768"));
    input.work_budget_bytes = 1024 * 1024;
    assert!(mcedit2_python::execute(&input)
        .unwrap_err()
        .contains("预算"));
    let legacy_generator = r#"
from mcedit2.plugins import registerGeneratePlugin
from mcedit2.editortools.generate import GeneratePlugin
@registerGeneratePlugin
class LargeGenerator(GeneratePlugin):
    def boundsChanged(self, bounds):
        self.volume = bounds.volume
    def generateInSchematic(self, dimension, originalBounds):
        assert self.volume == originalBounds.volume
        dimension.fillBlocks(dimension.selection, 'minecraft:gold_block')
"#;
    let mut generated = request(legacy_generator, Some("generator:LargeGenerator"));
    generated.snapshot = input.snapshot.clone();
    let generated = mcedit2_python::execute(&generated)
        .unwrap()
        .structure
        .unwrap();
    assert_eq!(generated.blocks.len(), 32768);
    assert!(generated
        .blocks
        .iter()
        .all(|state| state.name == "minecraft:gold_block"));
    let source = r#"
from mcedit2.api import register_generator
@register_generator('大结构')
def generate(context):
    return context.structure((16,128,16), ('minecraft:gold_block' for _ in range(32768)))
"#;
    let input = request(source, Some("generator:generate"));
    let output = mcedit2_plugins::run(
        exe,
        &input,
        &RunLimits {
            timeout: Duration::from_secs(120),
            ..RunLimits::default()
        },
    )
    .unwrap();
    let structure = output.structure.unwrap();
    assert_eq!(structure.size, [16, 128, 16]);
    assert_eq!(structure.blocks.len(), 32768);
    assert!(structure
        .blocks
        .iter()
        .all(|state| state.name == "minecraft:gold_block"));
    let mut small = input;
    small.work_budget_bytes = 1024 * 1024;
    assert!(mcedit2_python::execute(&small)
        .unwrap_err()
        .contains("预算"));
}

fn shaped_process_selection() {
    let source = r#"
from mcedit2.api import register_command
@register_command('选区形状')
def shaped(context):
    box = context.selection
    assert box.volume == 2
    assert list(box.positions) == [(-2, -64, 3)]
    assert (-2, -64, 3) in box and (-1, -64, 3) not in box
    context.dimension.fillBlocks(box, 'minecraft:gold_block')
"#;
    let mut input = request(source, Some("shaped"));
    input.snapshot.as_mut().unwrap().mask = Some(vec![true, false]);
    let exe = Path::new(env!("CARGO_BIN_EXE_mcedit2"));
    let output = mcedit2_plugins::run(exe, &input, &RunLimits::default()).unwrap();
    let blocks = output.blocks.unwrap();
    assert_eq!(blocks[0].name, "minecraft:gold_block");
    assert_eq!(blocks[1].name, "minecraft:stone");
    let source = source.replace(
        "context.dimension.fillBlocks(box, 'minecraft:gold_block')",
        "context.dimension.setBlock(-1, -64, 3, 'minecraft:gold_block')",
    );
    input.source = source;
    let error = mcedit2_plugins::run(exe, &input, &RunLimits::default()).unwrap_err();
    assert!(error.contains("源选区"), "{error}");
}

/// 同一插件子进程完成形状、区段网格与群系笔刷，再交回真实候选数据。
fn shaped_chunk_grids_share_real_numeric_block_and_biome_storage() {
    let source = r#"
from mcedit2.api import register_command
from mceditlib.selection import SelectionBox, ShapeFuncSelection
@register_command('形状与群系笔刷')
def paint(context):
    box = SelectionBox((0, 0, 0), (16, 16, 16))
    def sphere(coords, size):
        assert coords.shape == (3, 16, 16, 16)
        relative = (coords + 0.5) / size[:, None, None, None] - 0.5
        return (relative * relative).sum(axis=0) <= 0.25
    shape = ShapeFuncSelection(box, sphere)
    mask = shape.box_mask(box)
    assert mask.shape == (16, 16, 16) and mask.any() and not mask.all()
    ys, zs, xs = mask.nonzero()
    assert len(xs) == len(list(shape.positions))
    chunk = context.dimension.getChunk(0, 0)
    section = chunk.getSection(0)
    section.Blocks[mask] = 4
    section.Data[mask] = 3
    selected = context.dimension.getBlocks(xs, ys, zs, return_Data=True)
    assert selected.Blocks.tolist() == [4] * len(xs)
    assert selected.Data.tolist() == [3] * len(xs)
    line = section.Blocks[8, 8, :]
    before = line.tolist()
    copied = line[[0, 2, 4]]
    copied[:] = 2
    assert line.tolist() == before
    line[1:] = line[:-1]
    assert line.tolist() == [before[0]] + before[:-1]
    line[[0, 0]] = [5, 6]
    assert section.Blocks[8, 8, 0] == 6
    chunk.Biomes[zs, xs] = 7
    assert chunk.Biomes[zs, xs].tolist() == [7] * len(xs)
    assert (chunk.Biomes == 7).any(axis=0).any()
    context.log('区段、形状与群系交接完成')
"#;
    let mut input = request(source, Some("paint"));
    let snapshot = input.snapshot.as_mut().unwrap();
    snapshot.source_min = [0; 3];
    snapshot.size = [16; 3];
    snapshot.numeric = true;
    snapshot.data_version = Some(1343);
    snapshot.source_release = Some("1.12.2".into());
    snapshot.blocks = vec![parse_state("numeric:1[data=0]").unwrap(); 4096];
    snapshot.numeric_biomes = Some(mcedit2_world::numeric_biome::Volume {
        size: [16; 3],
        cells: vec![1; 4096],
        data_version: Some(1343),
    });
    let exe = Path::new(env!("CARGO_BIN_EXE_mcedit2"));
    let output = mcedit2_plugins::run(
        exe,
        &input,
        &RunLimits {
            timeout: Duration::from_secs(120),
            ..RunLimits::default()
        },
    )
    .unwrap();
    let blocks = output.blocks.unwrap();
    assert_eq!(blocks.len(), 4096);
    assert!(blocks.iter().any(|state| state.name == "numeric:4"));
    assert!(blocks.iter().any(|state| state.name == "numeric:6"));
    assert!(output
        .messages
        .iter()
        .any(|message| message.text == "区段、形状与群系交接完成"));
}
