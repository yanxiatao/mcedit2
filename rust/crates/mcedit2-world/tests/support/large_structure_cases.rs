//! 大结构的完整捕获、格式交换、载荷、克隆移动及取消；复用既有大类别。
use super::*;
use mcedit2_world::{
    clipboard::{CaptureOptions, PasteOptions},
    editing::{ClonePattern, EditWorkBudget, TransferKind, TransferOptions},
    nbt_document::{self, Document, Field, List, Operation, Value},
    structure_exchange::Format,
    structure_transform::Transform,
};

pub(super) fn fixture() -> NativeWorld {
    let mut world = large_edit_cases::fixture();
    let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
        &world.metadata(),
        "minecraft:overworld",
        0,
        0,
    )
    .unwrap()
    .unwrap();
    let be = Document::new(chunk_with_be("minecraft:stone", (0, -64, 0)))
        .unwrap()
        .inspect("/block_entities")
        .unwrap();
    let raw = nbt_document::patch(
        &raw,
        &Operation::Replace {
            path: "/block_entities".into(),
            value: be,
        },
    )
    .unwrap();
    let entity = Value::Compound(vec![
        Field {
            name: "id".into(),
            value: Value::String("minecraft:pig".into()),
        },
        Field {
            name: "UUID".into(),
            value: Value::IntArray(vec![11, 22, 33, 44]),
        },
        Field {
            name: "Pos".into(),
            value: Value::List(List {
                element_type: 6,
                items: vec![Value::Double(0.5), Value::Double(-63.5), Value::Double(0.5)],
            }),
        },
        Field {
            name: "Motion".into(),
            value: Value::List(List {
                element_type: 6,
                items: vec![Value::Double(0.); 3],
            }),
        },
        Field {
            name: "Rotation".into(),
            value: Value::List(List {
                element_type: 5,
                items: vec![Value::Float(0.); 2],
            }),
        },
        Field {
            name: "OpaqueClonePayload".into(),
            value: Value::String("实体未知字段原样保留".into()),
        },
    ]);
    // 现代实体使用真实独立 entities Region，不能伪造内嵌来源。
    let entity_raw = tag(
        10,
        "",
        &compound(vec![
            tag(3, "DataVersion", &3955_i32.to_be_bytes()),
            tag(
                11,
                "Position",
                &[
                    2_i32.to_be_bytes(),
                    0_i32.to_be_bytes(),
                    0_i32.to_be_bytes(),
                ]
                .concat(),
            ),
            tag(9, "Entities", &[10, 0, 0, 0, 0]),
        ]),
    );
    let entity_raw = nbt_document::patch(
        &entity_raw,
        &Operation::Replace {
            path: "/Entities".into(),
            value: Value::List(List {
                element_type: 10,
                items: vec![entity],
            }),
        },
    )
    .unwrap();
    fs::create_dir_all(world.path.join("entities")).unwrap();
    fs::write(
        world.path.join("entities/r.0.0.mca"),
        region_bytes(&entity_raw),
    )
    .unwrap();
    let mut target = raw.clone();
    for section in 0..8 {
        target = nbt_document::patch(
            &target,
            &Operation::Replace {
                path: format!("/sections/{section}/block_states/palette/0/Name"),
                value: Value::String("minecraft:dirt".into()),
            },
        )
        .unwrap();
    }
    target = nbt_document::patch(
        &target,
        &Operation::Replace {
            path: "/block_entities".into(),
            value: Value::List(List {
                element_type: 10,
                items: vec![],
            }),
        },
    )
    .unwrap();
    target = nbt_document::patch(
        &target,
        &Operation::Replace {
            path: "/xPos".into(),
            value: Value::Int(1),
        },
    )
    .unwrap();
    let mut region = region_bytes(&raw);
    let appended = region_bytes(&target);
    region[4..8].copy_from_slice(&((3_u32 << 8) | 1).to_be_bytes());
    region.extend_from_slice(&appended[8192..]);
    world.original_region = region;
    fs::write(world.path.join("region/r.0.0.mca"), &world.original_region).unwrap();
    world
}
fn options(kind: TransferKind) -> TransferOptions {
    TransferOptions {
        kind,
        entities: true,
        biomes: true,
        proof_budget: 256 * 1024 * 1024,
    }
}
fn state(session: &Session, p: (i32, i32, i32)) -> String {
    let Lookup::Present(info) = session.visible_block(p).unwrap() else {
        panic!("真实方块缺失");
    };
    render_state(&info.state)
}

pub(super) fn capture_transform_and_transfer() {
    let disk = fixture();
    let terrain = disk.bytes_now();
    let entities = fs::read(disk.path.join("entities/r.0.0.mca")).unwrap();
    let mut editor = Session::new(disk.metadata(), "minecraft:overworld").unwrap();
    let bounds = Bounds::spatial((0, -64, 0), (15, 63, 15)).unwrap();
    let budget = EditWorkBudget::from_mib("256").unwrap();
    let wide = editor
        .capture_structure_with_budget_cancellable(
            Bounds::spatial((0, -64, 0), (31, 63, 15)).unwrap(),
            CaptureOptions {
                entities: false,
                biomes: true,
            },
            budget,
            || false,
        )
        .unwrap();
    assert_eq!(wide.blocks.len(), 65536);
    for y in [0, 63, 127] {
        for z in [0, 7, 15] {
            let row = (y * 16 + z) * 32;
            assert_eq!(wide.blocks[row].name, "minecraft:stone");
            assert_eq!(wide.blocks[row + 15].name, "minecraft:stone");
            assert_eq!(wide.blocks[row + 16].name, "minecraft:dirt");
            assert_eq!(wide.blocks[row + 31].name, "minecraft:dirt");
        }
    }
    let clip = editor
        .capture_structure_with_budget_cancellable(
            bounds,
            CaptureOptions {
                entities: true,
                biomes: true,
            },
            budget,
            || false,
        )
        .unwrap();
    assert_eq!(clip.blocks.len(), 32768);
    assert_eq!(clip.block_entities.len(), 1);
    assert_eq!(clip.entities.len(), 1);
    assert_eq!(clip.biomes.as_ref().unwrap().cells.len(), 32768);
    assert_eq!(clip.blocks.first().unwrap(), clip.blocks.last().unwrap());
    let mut calls = 0;
    assert!(editor
        .capture_structure_with_budget_cancellable(
            bounds,
            CaptureOptions {
                entities: true,
                biomes: true
            },
            budget,
            || {
                calls += 1;
                calls > 3
            }
        )
        .is_err());
    assert!(editor
        .capture_structure_with_budget_cancellable(
            bounds,
            CaptureOptions {
                entities: true,
                biomes: true
            },
            EditWorkBudget::from_mib("1").unwrap(),
            || false
        )
        .is_err());
    let mirrored = clip
        .resampled_with_budget_cancellable(
            Transform {
                rotation: [0., 90., 0.],
                scale: [-1., 1., 1.],
                rotate_states: false,
            },
            budget,
            || false,
        )
        .unwrap();
    assert_eq!(mirrored.blocks.len(), 32768);
    assert_eq!(mirrored.block_entities.len(), 1);
    assert_eq!(mirrored.entities.len(), 1);
    let preview = editor
        .prepare_clone_pattern_cancellable(
            bounds,
            (16, -64, 0),
            options(TransferKind::Clone),
            ClonePattern {
                copy_air: true,
                ..Default::default()
            },
            None,
            || false,
        )
        .unwrap();
    assert_eq!(preview.changed_blocks, 32768);
    assert_eq!(editor.pending_cells(), 0);
    assert_eq!(state(&preview.candidate(), (31, 63, 15)), "minecraft:stone");
    assert_eq!(
        editor
            .commit_transfer_cancellable(preview, || false)
            .unwrap(),
        32768
    );
    assert_eq!(editor.undo_depth(), 1);
    let captured = editor
        .capture_structure_with_budget_cancellable(
            Bounds::spatial((16, -64, 0), (31, 63, 15)).unwrap(),
            CaptureOptions {
                entities: true,
                biomes: true,
            },
            budget,
            || false,
        )
        .unwrap();
    assert_eq!(captured.block_entities.len(), 1);
    assert_eq!(captured.entities.len(), 1);
    let mut old_be = vec![10, 0, 0];
    old_be.extend_from_slice(&clip.block_entities[0].compound);
    let mut new_be = vec![10, 0, 0];
    new_be.extend_from_slice(&captured.block_entities[0].compound);
    assert_eq!(
        Document::new(old_be)
            .unwrap()
            .inspect("/ModdedCompound")
            .unwrap(),
        Document::new(new_be)
            .unwrap()
            .inspect("/ModdedCompound")
            .unwrap()
    );
    let entity_docs = |clip: &mcedit2_world::clipboard::BlockClipboard| {
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&clip.entities[0].compound);
        Document::new(raw).unwrap()
    };
    assert_eq!(
        entity_docs(&clip).inspect("/OpaqueClonePayload").unwrap(),
        entity_docs(&captured)
            .inspect("/OpaqueClonePayload")
            .unwrap()
    );
    assert_ne!(
        entity_docs(&clip).inspect("/UUID").unwrap(),
        entity_docs(&captured).inspect("/UUID").unwrap()
    );
    editor.undo().unwrap();
    assert!(!editor.has_pending_changes());
    editor.redo().unwrap();
    assert_eq!(editor.pending_cells(), 32768);
    editor.undo().unwrap();
    let moved = editor
        .prepare_transfer_cancellable(
            bounds,
            (16, -64, 0),
            options(TransferKind::Move),
            None,
            || false,
        )
        .unwrap();
    let mut calls = 0;
    let stamp = editor.revision();
    assert!(editor
        .commit_transfer_cancellable(moved.clone(), || {
            calls += 1;
            calls > 10
        })
        .is_err());
    assert!(std::sync::Arc::ptr_eq(&stamp, &editor.revision()));
    assert!(!editor.has_pending_changes());
    editor.commit_transfer_cancellable(moved, || false).unwrap();
    assert_eq!(editor.pending_cells(), 65536);
    assert_eq!(editor.undo_depth(), 1);
    assert_eq!(state(&editor, (0, -64, 0)), "minecraft:air");
    let target = editor
        .capture_structure_with_budget_cancellable(
            Bounds::spatial((16, -64, 0), (31, 63, 15)).unwrap(),
            CaptureOptions {
                entities: true,
                biomes: true,
            },
            budget,
            || false,
        )
        .unwrap();
    assert_eq!(
        entity_docs(&clip).inspect("/UUID").unwrap(),
        entity_docs(&target).inspect("/UUID").unwrap()
    );
    let mut calls = 0;
    assert!(editor
        .undo_cancellable(|| {
            calls += 1;
            calls > 10
        })
        .is_err());
    assert_eq!(editor.pending_cells(), 65536);
    editor.undo().unwrap();
    assert!(!editor.has_pending_changes());
    editor.redo().unwrap();
    editor.undo().unwrap();
    // 普通结构提交在应用后段取消时也必须原子，不只依赖组合传输外壳。
    let paste = editor
        .prepare_structure_paste_with_budget_cancellable(
            &clip,
            (16, -64, 0),
            PasteOptions {
                include_biomes: true,
            },
            None,
            budget,
            || false,
        )
        .unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = std::sync::Arc::clone(&stop);
    assert!(editor
        .commit_structure_with_progress_cancellable(
            paste,
            || stop.load(Ordering::Acquire),
            move |p| {
                if p.stage == "构建完整结构暂存" && p.changed >= 4096 {
                    signal.store(true, Ordering::Release);
                }
            }
        )
        .is_err());
    assert!(!editor.has_pending_changes());
    assert_eq!(disk.bytes_now(), terrain);
    assert_eq!(
        fs::read(disk.path.join("entities/r.0.0.mca")).unwrap(),
        entities
    );
}

pub(super) fn format_roundtrips() {
    let disk = fixture();
    let world = disk.metadata();
    let original = disk.bytes_now();
    let editor = Session::new(world.clone(), "minecraft:overworld").unwrap();
    let budget = EditWorkBudget::from_mib("256").unwrap();
    let bounds = Bounds::spatial((0, -64, 0), (15, 63, 15)).unwrap();
    let mut clip = editor
        .capture_structure_with_budget_cancellable(
            bounds,
            CaptureOptions {
                entities: false,
                biomes: false,
            },
            budget,
            || false,
        )
        .unwrap();
    clip.source_bound = false;
    clip.block_entities.clear();
    let files = std::env::temp_dir().join(format!(
        "mcedit2-large-structures-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&files).unwrap();
    let mut outputs = Vec::new();
    for format in [
        Format::Clipboard,
        Format::SpongeV2,
        Format::SpongeV3,
        Format::Structure,
    ] {
        let path = files.join(format!("named-{}.{}", format.label(), format.extension()));
        format
            .write_with_budget(&world, &clip, &path, budget)
            .unwrap();
        outputs.push(path.clone());
        let loaded = format
            .read_with_budget(&world, "minecraft:overworld", &path, budget)
            .unwrap();
        assert_eq!(loaded.blocks, clip.blocks);
        assert_eq!(loaded.size, clip.size);
        assert!(format
            .read_with_budget(
                &world,
                "minecraft:overworld",
                &path,
                EditWorkBudget::from_mib("1").unwrap()
            )
            .is_err());
    }
    let mut large = clip.clone();
    large.size[1] *= 4;
    large.blocks = clip.blocks.iter().cycle().take(131072).cloned().collect();
    let path = files.join("large.mcedit2clip");
    Format::Clipboard
        .write_with_budget(&world, &large, &path, budget)
        .unwrap();
    outputs.push(path.clone());
    assert!(fs::metadata(&path).unwrap().len() > 8 * 1024 * 1024);
    let loaded = Format::Clipboard
        .read_with_budget(&world, "minecraft:overworld", &path, budget)
        .unwrap();
    assert_eq!(loaded.blocks, large.blocks);
    let old = NativeWorld::create(false);
    let old_editor = Session::new(old.metadata(), "minecraft:overworld").unwrap();
    let numeric = old_editor
        .capture_structure_with_budget_cancellable(
            Bounds::spatial((0, 0, 0), (15, 127, 15)).unwrap(),
            CaptureOptions {
                entities: false,
                biomes: false,
            },
            budget,
            || false,
        )
        .unwrap();
    let path = files.join("legacy.schematic");
    Format::Legacy
        .write_with_budget(&old.metadata(), &numeric, &path, budget)
        .unwrap();
    outputs.push(path.clone());
    let roundtrip = Format::Legacy
        .read_with_budget(&old.metadata(), "minecraft:overworld", &path, budget)
        .unwrap();
    assert_eq!(roundtrip.blocks, numeric.blocks);
    assert_eq!(disk.bytes_now(), original);
    for path in outputs {
        fs::remove_file(path).unwrap();
    }
    fs::remove_dir(files).unwrap();
}
