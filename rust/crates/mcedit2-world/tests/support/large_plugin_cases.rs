//! 大选区插件的依赖步骤、原生差异、单组撤销及原子取消。
use super::*;
use mcedit2_world::{
    editing::EditWorkBudget,
    plugins::{PluginManifestV2, PluginOperation},
};

pub(super) fn complete_large_plugin_pipeline() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let fixture = large_edit_cases::fixture();
    let original = fixture.bytes_now();
    let budget = EditWorkBudget::from_mib("128").unwrap();
    let small = EditWorkBudget::from_mib("1").unwrap();
    let mut session = Session::new(fixture.metadata(), "minecraft:overworld").unwrap();
    let bounds = Bounds::spatial((0, -64, 0), (15, 63, 15)).unwrap();
    let manifest = PluginManifestV2 {
        schema_version: 2,
        name: "整区依赖替换".into(),
        description: None,
        only_data_version: None,
        only_dimension: None,
        operations: (0..24)
            .map(|index| PluginOperation::ReplaceBlocks {
                minimum: [0, -64, 0],
                maximum: [15, 63, 15],
                with_state: if index % 2 == 0 {
                    "minecraft:dirt"
                } else {
                    "minecraft:gold_block"
                }
                .into(),
                only_if_state: Some(
                    if index == 0 {
                        "minecraft:stone"
                    } else if index % 2 == 0 {
                        "minecraft:gold_block"
                    } else {
                        "minecraft:dirt"
                    }
                    .into(),
                ),
            })
            .collect(),
    };
    assert!(manifest
        .prepare_with_budget(&session, small, || false, |_| {})
        .is_err());
    let stop = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&stop);
    assert!(manifest
        .prepare_with_budget(
            &session,
            budget,
            || stop.load(Ordering::Acquire),
            move |p| {
                if p.stage == "完成声明式插件步骤" {
                    signal.store(true, Ordering::Release);
                }
            }
        )
        .is_err());
    assert_eq!(session.pending_cells(), 0);
    assert_eq!(session.undo_depth(), 0);
    let mut steps = 0;
    let preview = manifest
        .prepare_with_budget(
            &session,
            budget,
            || false,
            |p| {
                if p.stage == "完成声明式插件步骤" {
                    steps = p.slices;
                }
            },
        )
        .unwrap();
    assert_eq!(steps, 24);
    assert_eq!(preview.changed(), 32768);
    session.commit(preview).unwrap();
    assert_eq!(session.pending_cells(), 32768);
    assert_eq!(session.undo_depth(), 1);
    let Lookup::Present(info) = session.visible_block((15, 63, 15)).unwrap() else {
        panic!("末格必须存在")
    };
    assert_eq!(info.state.name, "minecraft:gold_block");
    session.undo().unwrap();
    assert!(!session.has_pending_changes());
    session.redo().unwrap();
    session.undo().unwrap();
    let snapshot = session
        .capture_block_states_with_budget_cancellable(bounds, budget, || false)
        .unwrap();
    let blocks = vec![parse_state("minecraft:glass").unwrap(); 32768];
    assert!(session
        .prepare_block_difference_with_budget_cancellable(&snapshot, &blocks, small, || false)
        .is_err());
    let mut calls = 0;
    assert!(session
        .prepare_block_difference_with_budget_cancellable(&snapshot, &blocks, budget, || {
            calls += 1;
            calls > 12
        })
        .is_err());
    assert!(!session.has_pending_changes());
    let difference = session
        .prepare_block_difference_with_budget_cancellable(&snapshot, &blocks, budget, || false)
        .unwrap()
        .unwrap();
    session
        .commit_structure_cancellable(difference, || false)
        .unwrap();
    assert_eq!(session.pending_cells(), 32768);
    assert_eq!(session.undo_depth(), 1);
    assert!(session
        .prepare_block_difference_with_budget_cancellable(&snapshot, &blocks, budget, || false)
        .is_err());
    session.undo().unwrap();
    assert!(!session.has_pending_changes());
    assert!(session
        .prepare_block_difference_with_budget_cancellable(
            &snapshot,
            &snapshot.blocks,
            budget,
            || false
        )
        .unwrap()
        .is_none());
    assert_eq!(fixture.bytes_now(), original);
    payload_difference(budget);
}

fn payload_difference(budget: EditWorkBudget) {
    use mcedit2_world::clipboard::CaptureOptions;
    let fixture = large_structure_cases::fixture();
    let original = fixture.bytes_now();
    let entity_bytes = fs::read(fixture.path.join("entities/r.0.0.mca")).unwrap();
    let mut session = Session::new(fixture.metadata(), "minecraft:overworld").unwrap();
    let bounds = Bounds::spatial((0, -64, 0), (15, 63, 15)).unwrap();
    let capture = |session: &Session| {
        session
            .capture_structure_with_budget_cancellable(
                bounds,
                CaptureOptions {
                    entities: true,
                    biomes: true,
                },
                budget,
                || false,
            )
            .unwrap()
    };
    let source = capture(&session);
    assert_eq!(source.block_entities.len(), 1);
    let snapshot = session
        .capture_block_states_with_budget_cancellable(bounds, budget, || false)
        .unwrap();
    let mut blocks = snapshot.blocks.clone();
    blocks[32767] = parse_state("minecraft:glass").unwrap();
    let preview = session
        .prepare_block_difference_with_budget_cancellable(&snapshot, &blocks, budget, || false)
        .unwrap()
        .unwrap();
    session
        .commit_structure_cancellable(preview, || false)
        .unwrap();
    let result = capture(&session);
    assert_eq!(
        result.block_entities[0].compound,
        source.block_entities[0].compound
    );
    assert_eq!(
        serde_json::to_value(&result.entities).unwrap(),
        serde_json::to_value(&source.entities).unwrap()
    );
    session.undo().unwrap();
    blocks.fill(parse_state("minecraft:glass").unwrap());
    let preview = session
        .prepare_block_difference_with_budget_cancellable(&snapshot, &blocks, budget, || false)
        .unwrap()
        .unwrap();
    session
        .commit_structure_cancellable(preview, || false)
        .unwrap();
    assert!(capture(&session).block_entities.is_empty());
    session.undo().unwrap();
    assert_eq!(
        capture(&session).block_entities[0].compound,
        source.block_entities[0].compound
    );
    assert_eq!(fixture.bytes_now(), original);
    assert_eq!(
        fs::read(fixture.path.join("entities/r.0.0.mca")).unwrap(),
        entity_bytes
    );
}
