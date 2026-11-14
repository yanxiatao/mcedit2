//! 大笔画的完整行为批次：真实 NBT、资源预算、取消、历史及群系载荷。
use super::*;
use mcedit2_world::{
    editing::{BrushMask, BrushPaint, BrushReplacement, EditWorkBudget},
    nbt_document::{self, Document, Field, List, Operation, Value},
};

pub(super) fn fixture() -> NativeWorld {
    let mut fixture = NativeWorld::create(true);
    let raw = modern_chunk("minecraft:stone");
    let section = Document::new(raw.clone())
        .unwrap()
        .inspect("/sections/0")
        .unwrap();
    let mut sections = Vec::new();
    for y in -4_i8..4 {
        let Value::Compound(mut fields) = section.clone() else {
            panic!("源区段必须为复合标签");
        };
        fields
            .iter_mut()
            .find(|field| field.name == "Y")
            .unwrap()
            .value = Value::Byte(y);
        fields.push(Field {
            name: "biomes".into(),
            value: Value::Compound(vec![Field {
                name: "palette".into(),
                value: Value::List(List {
                    element_type: 8,
                    items: vec![Value::String("minecraft:plains".into())],
                }),
            }]),
        });
        sections.push(Value::Compound(fields));
    }
    let raw = nbt_document::patch(
        &raw,
        &Operation::Replace {
            path: "/sections".into(),
            value: Value::List(List {
                element_type: 10,
                items: sections,
            }),
        },
    )
    .unwrap();
    fixture.original_region = region_bytes(&raw);
    fs::write(
        fixture.path.join("region/r.0.0.mca"),
        &fixture.original_region,
    )
    .unwrap();
    fixture
}
fn state(session: &Session, p: (i32, i32, i32)) -> mcedit2_world::sections::State {
    let Lookup::Present(info) = session.visible_block(p).unwrap() else {
        panic!("源方块应存在");
    };
    info.state
}
pub(super) fn blocks_and_history() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let fixture = fixture();
    let original = fixture.bytes_now();
    let budget = EditWorkBudget::from_mib("128").unwrap();
    let bounds = Bounds::spatial((0, -64, 0), (15, 63, 15)).unwrap();
    let full = BrushMask::default()
        .select([Ok(bounds), Ok(bounds)], budget, || false, |_| {})
        .unwrap();
    assert_eq!(full.len(), 32768);
    let mut session = Session::new(fixture.metadata(), "minecraft:overworld").unwrap();
    let stone = parse_state("minecraft:stone").unwrap();
    let log = parse_state("minecraft:oak_log[axis=y]").unwrap();
    let dirt = parse_state("minecraft:dirt").unwrap();
    let paint = BrushPaint::Replace(vec![
        BrushReplacement {
            sources: vec![stone.clone()],
            target: log.clone(),
        },
        BrushReplacement {
            sources: vec![log.clone()],
            target: dirt.clone(),
        },
    ]);
    let mut progress = Vec::new();
    let prepared = session
        .prepare_brush_selection_cancellable(
            full.clone(),
            &paint,
            budget,
            || false,
            |p| progress.push(p),
        )
        .unwrap();
    assert_eq!(prepared.changed(), 32768);
    assert_eq!(prepared.source_chunks, 1);
    assert_eq!(prepared.examined, 32768);
    assert!(progress.iter().any(|p| p.examined == 4096));
    assert_eq!(session.pending_cells(), 0);
    let mut calls = 0;
    assert!(session
        .prepare_brush_selection_cancellable(
            full.clone(),
            &paint,
            budget,
            || {
                calls += 1;
                calls > 3
            },
            |_| {}
        )
        .is_err());
    assert_eq!(session.pending_cells(), 0);
    assert!(session
        .prepare_brush_selection_cancellable(
            full.clone(),
            &paint,
            EditWorkBudget::from_mib("1").unwrap(),
            || false,
            |_| {}
        )
        .is_err());
    let canceled = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&canceled);
    let stamp = session.revision();
    assert!(session
        .commit_cancellable(
            prepared.clone(),
            || canceled.load(Ordering::Acquire),
            move |p| {
                if p.stage == "构建整组暂存结果" && p.changed >= 4096 {
                    signal.store(true, Ordering::Release);
                }
            }
        )
        .is_err());
    assert_eq!(session.pending_cells(), 0);
    assert_eq!(session.undo_depth(), 0);
    assert!(Arc::ptr_eq(&stamp, &session.revision()));
    // 原始源在后段变化，也不能发布已通过前段复核的候选。
    let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
        &fixture.metadata(),
        "minecraft:overworld",
        0,
        0,
    )
    .unwrap()
    .unwrap();
    let changed = nbt_document::patch(
        &raw,
        &Operation::Replace {
            path: "/sections/7/block_states/palette/0/Name".into(),
            value: Value::String("minecraft:dirt".into()),
        },
    )
    .unwrap();
    fs::write(
        fixture.path.join("region/r.0.0.mca"),
        region_bytes(&changed),
    )
    .unwrap();
    assert!(session.commit(prepared.clone()).is_err());
    assert_eq!(session.pending_cells(), 0);
    fs::write(fixture.path.join("region/r.0.0.mca"), &original).unwrap();
    assert_eq!(session.commit(prepared).unwrap(), 32768);
    assert_eq!(session.undo_depth(), 1);
    for p in [(0, -64, 0), (15, 63, 15), (7, 0, 8)] {
        assert_eq!(state(&session, p), log);
    }
    let mut calls = 0;
    let before = session.revision();
    assert!(session
        .undo_cancellable(|| {
            calls += 1;
            calls > 3
        })
        .is_err());
    assert_eq!(session.pending_cells(), 32768);
    assert!(Arc::ptr_eq(&before, &session.revision()));
    session.undo().unwrap();
    assert_eq!(session.pending_cells(), 0);
    let mut calls = 0;
    assert!(session
        .redo_cancellable(|| {
            calls += 1;
            calls > 3
        })
        .is_err());
    assert_eq!(session.pending_cells(), 0);
    session.redo().unwrap();
    assert_eq!(session.pending_cells(), 32768);
    session.undo().unwrap();
    let shell = BrushMask {
        shape: None,
        hollow: true,
    }
    .select(
        [
            Ok(bounds),
            Ok(Bounds::spatial((8, -64, 0), (15, 63, 15)).unwrap()),
        ],
        budget,
        || false,
        |_| {},
    )
    .unwrap();
    assert_eq!(shell.len(), 32768 - 14 * 126 * 14);
    let preview = session
        .prepare_brush_selection_cancellable(
            shell,
            &BrushPaint::Fill(log.clone()),
            budget,
            || false,
            |_| {},
        )
        .unwrap();
    session.commit(preview).unwrap();
    assert_eq!(state(&session, (0, -64, 0)), log);
    assert_eq!(state(&session, (1, -63, 1)), stone);
    session.undo().unwrap();
    let connected = session
        .prepare_connected_fill_cancellable(bounds, (0, -64, 0), &log, budget, || false, |_| {})
        .unwrap();
    assert_eq!(connected.changed(), 32768);
    assert_eq!(connected.examined, 32768);
    session.commit(connected).unwrap();
    assert_eq!(session.undo_depth(), 1);
    session.undo().unwrap();
    let outside = Bounds::spatial((0, -64, 0), (31, 63, 15)).unwrap();
    assert!(session
        .prepare_box_cancellable(outside, None, &log, None, budget, || false, |_| {})
        .is_err());
    assert_eq!(session.pending_cells(), 0);
    assert_eq!(fixture.bytes_now(), original);
    // 旧数字世界的完整高度笔画走同一分块、预算及历史通路。
    let legacy = NativeWorld::create(false);
    let raw = legacy.bytes_now();
    let mut old = Session::new(legacy.metadata(), "minecraft:overworld").unwrap();
    let b = Bounds::spatial((0, 0, 0), (15, 127, 15)).unwrap();
    let target = parse_state("numeric:2[data=0]").unwrap();
    let p = old
        .prepare_box_cancellable(b, None, &target, None, budget, || false, |_| {})
        .unwrap();
    assert_eq!(old.commit(p).unwrap(), 32768);
    old.undo().unwrap();
    assert_eq!(old.pending_cells(), 0);
    assert_eq!(legacy.bytes_now(), raw);
}
pub(super) fn biomes_and_payloads() {
    use mcedit2_world::editing::BiomeTarget;
    let fixture = fixture();
    let original = fixture.bytes_now();
    let budget = EditWorkBudget::from_mib("128").unwrap();
    let bounds = Bounds::spatial((0, -64, 0), (15, 63, 15)).unwrap();
    let mut session = Session::new(fixture.metadata(), "minecraft:overworld").unwrap();
    let selection = BrushMask::default()
        .select([Ok(bounds), Ok(bounds)], budget, || false, |_| {})
        .unwrap();
    let paint = session
        .plan_biome_selection_cancellable(
            selection,
            &BiomeTarget::Named("minecraft:forest".into()),
            budget,
            || false,
            |_| {},
        )
        .unwrap()
        .unwrap();
    assert_eq!(paint.quart_cells, 512);
    assert_eq!(paint.transitions.values().sum::<usize>(), 512);
    session.commit_structure(paint.into_structure()).unwrap();
    assert_eq!(session.undo_depth(), 1);
    let staged = session.staged_biome_chunk(0, 0).unwrap().unwrap();
    let decoded = mcedit2_world::sections::decode_sections(staged).unwrap();
    assert_eq!(decoded.biome(15, 63, 15), Some("minecraft:forest"));
    assert_eq!(decoded.biome(0, -64, 0), Some("minecraft:forest"));
    let old = Document::new(
        mcedit2_world::block_lookup::read_world_chunk_bytes(
            &fixture.metadata(),
            "minecraft:overworld",
            0,
            0,
        )
        .unwrap()
        .unwrap(),
    )
    .unwrap();
    let after = Document::new(staged.to_vec()).unwrap();
    for section in 0..8 {
        let path = format!("/sections/{section}/OpaqueModData");
        assert_eq!(old.inspect(&path).unwrap(), after.inspect(&path).unwrap());
    }
    session.undo().unwrap();
    assert!(!session.has_pending_changes());
    session.redo().unwrap();
    assert!(session.has_pending_changes());
    assert_eq!(fixture.bytes_now(), original);
}
