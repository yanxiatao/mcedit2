//! Biome 刷子整批行为验证，接入已有交换与编辑类别。
use super::*;
use mcedit2_world::{
    editing::{BiomeTarget, BrushShape},
    nbt_document::{self, Field, List, Operation, Value},
    numeric_biome::Storage,
    sections::decode_sections,
};

pub(super) fn complete_biome_brush_pipeline() {
    assert!(BiomeTarget::parse("Forest").is_err());
    assert!(BiomeTarget::parse("unknown:").is_err());
    for storage in [
        Storage::ByteColumns,
        Storage::IntColumns,
        Storage::IntQuarts,
    ] {
        let fixture = NativeWorld::create(true);
        let (raw, version) = numeric_biome_cases::source(storage);
        overwrite_region(&fixture, &raw);
        let mut world = fixture.metadata();
        world.data_version = Some(version);
        let session = Session::new(world.clone(), "minecraft:overworld").unwrap();
        let target = BiomeTarget::parse("42").unwrap();
        let bounds = Bounds::new((0, 1, 0), (1, 2, 1)).unwrap();
        let preview = session
            .prepare_biome_brush_cancellable(
                &[bounds, bounds],
                mcedit2_world::editing::BrushMask {
                    shape: None,
                    hollow: false,
                },
                &target,
                || false,
            )
            .unwrap();
        let count = if storage == Storage::IntQuarts { 1 } else { 4 };
        assert_eq!(preview.count(), count);
        assert_eq!(preview.transitions.values().sum::<usize>(), count);
        let mut edited = session.clone();
        edited.commit_structure(preview.into_structure()).unwrap();
        assert_eq!(edited.pending_cells(), 0);
        let changed = edited.staged_biome_chunk(0, 0).unwrap().unwrap().to_vec();
        let data = decode_sections(&changed).unwrap();
        assert_eq!(data.numeric_biome(0, 1, 0), Some(42));
        if storage == Storage::IntQuarts {
            assert_eq!(data.numeric_biome(3, 3, 3), Some(42));
            assert_eq!(
                data.numeric_biome(0, 4, 0),
                decode_sections(&raw).unwrap().numeric_biome(0, 4, 0)
            );
        } else {
            assert_eq!(data.numeric_biome(1, 200, 1), Some(42));
            let ellipse = session
                .prepare_biome_brush_cancellable(
                    &[Bounds::new((0, 1, 0), (4, 1, 4)).unwrap()],
                    mcedit2_world::editing::BrushMask {
                        shape: Some(BrushShape::Ellipsoid),
                        hollow: false,
                    },
                    &target,
                    || false,
                )
                .unwrap();
            assert_eq!(ellipse.columns, 21);
        }
        assert!(edited
            .prepare_biome_brush_cancellable(
                &[bounds],
                mcedit2_world::editing::BrushMask {
                    shape: None,
                    hollow: false
                },
                &target,
                || false
            )
            .is_err());
        assert!(edited.undo().unwrap());
        assert!(!edited.has_pending_changes());
        assert!(edited.redo().unwrap());
        assert_eq!(edited.staged_biome_chunk(0, 0).unwrap().unwrap(), changed);
        let old_preview = edited
            .prepare_biome_brush_cancellable(
                &[bounds],
                mcedit2_world::editing::BrushMask {
                    shape: None,
                    hollow: false,
                },
                &BiomeTarget::Numeric(43),
                || false,
            )
            .unwrap();
        let next = edited
            .prepare_biome_brush_cancellable(
                &[bounds],
                mcedit2_world::editing::BrushMask {
                    shape: None,
                    hollow: false,
                },
                &BiomeTarget::Numeric(44),
                || false,
            )
            .unwrap();
        edited.commit_structure(next.into_structure()).unwrap();
        assert!(edited
            .commit_structure(old_preview.into_structure())
            .is_err());
        edited.undo().unwrap();
        assert_eq!(edited.staged_biome_chunk(0, 0).unwrap().unwrap(), changed);
        assert!(session
            .prepare_biome_brush_cancellable(
                &[bounds],
                mcedit2_world::editing::BrushMask {
                    shape: None,
                    hollow: false
                },
                &target,
                || true
            )
            .is_err());
        assert!(session
            .prepare_biome_brush_cancellable(
                &[bounds, Bounds::new((16, 0, 0), (16, 0, 0)).unwrap()],
                mcedit2_world::editing::BrushMask {
                    shape: None,
                    hollow: false
                },
                &target,
                || false
            )
            .is_err());
        assert!(session
            .prepare_biome_brush_cancellable(
                &[bounds],
                mcedit2_world::editing::BrushMask {
                    shape: None,
                    hollow: false
                },
                &BiomeTarget::Named("minecraft:desert".into()),
                || false
            )
            .is_err());
        if storage == Storage::ByteColumns {
            assert!(session
                .prepare_biome_brush_cancellable(
                    &[bounds],
                    mcedit2_world::editing::BrushMask {
                        shape: None,
                        hollow: false
                    },
                    &BiomeTarget::Numeric(256),
                    || false
                )
                .is_err());
        }
        for shape in [
            BrushShape::HollowBox,
            BrushShape::Ellipsoid,
            BrushShape::EllipsoidShell,
            BrushShape::CylinderY,
            BrushShape::CylinderShellY,
            BrushShape::Diamond,
            BrushShape::ParabolicDome,
        ] {
            let p = session
                .prepare_biome_brush_cancellable(
                    &[Bounds::new((0, 0, 0), (4, 4, 4)).unwrap()],
                    mcedit2_world::editing::BrushMask {
                        shape: Some(shape),
                        hollow: false,
                    },
                    &target,
                    || false,
                )
                .unwrap();
            assert!(p.count() > 0);
        }
        save_and_rollback(&fixture, &world, &edited, &changed);
        assert!(!session.has_pending_changes());
    }
    modern_brush();
    hollow_modern_quarts();
}

fn save_and_rollback(
    fixture: &NativeWorld,
    world: &WorldSummary,
    session: &Session,
    expected: &[u8],
) {
    let before = fixture.bytes_now();
    let candidates = session.preview_be_region_candidates(927).unwrap();
    assert_eq!(candidates.len(), 1);
    let receipt = mcedit2_world::region_transaction::apply_single_region(
        world,
        "minecraft:overworld",
        &candidates[0],
    )
    .unwrap();
    assert_eq!(fs::read(&receipt.backup_path).unwrap(), before);
    assert_eq!(
        mcedit2_world::block_lookup::read_world_chunk_bytes(world, "minecraft:overworld", 0, 0)
            .unwrap()
            .unwrap(),
        expected
    );
    mcedit2_world::region_transaction::rollback_pending_region(
        world,
        "minecraft:overworld",
        0,
        0,
        "mca",
    )
    .unwrap();
    assert_eq!(fixture.bytes_now(), before);
}

fn modern_brush() {
    let fixture = NativeWorld::create(true);
    let raw = nbt_document::patch(
        &modern_chunk("minecraft:stone"),
        &Operation::InsertField {
            path: "/sections/0".into(),
            name: "biomes".into(),
            value: Value::Compound(vec![Field {
                name: "palette".into(),
                value: Value::List(List {
                    element_type: 8,
                    items: vec![Value::String("minecraft:plains".into())],
                }),
            }]),
        },
    )
    .unwrap();
    overwrite_region(&fixture, &raw);
    let world = fixture.metadata();
    let mut session = Session::new(world.clone(), "minecraft:overworld").unwrap();
    let bounds = Bounds::new((3, -61, 3), (4, -60, 4)).unwrap();
    let target = BiomeTarget::parse("custom:alpine").unwrap();
    let p = session
        .prepare_biome_brush_cancellable(
            &[bounds, bounds],
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false,
            },
            &target,
            || false,
        )
        .unwrap();
    assert_eq!(p.quart_cells, 8);
    assert_eq!(p.columns, 0);
    session.commit_structure(p.into_structure()).unwrap();
    let changed = session.staged_biome_chunk(0, 0).unwrap().unwrap().to_vec();
    let decoded = decode_sections(&changed).unwrap();
    assert_eq!(decoded.biome(0, -64, 0), Some("custom:alpine"));
    assert_eq!(decoded.biome(7, -57, 7), Some("custom:alpine"));
    assert_eq!(decoded.biome(8, -64, 0), Some("minecraft:plains"));
    assert_eq!(
        decoded.block(0, -64, 0),
        decode_sections(&raw).unwrap().block(0, -64, 0)
    );
    assert!(session
        .prepare_biome_brush_cancellable(
            &[bounds],
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false
            },
            &BiomeTarget::Numeric(1),
            || false
        )
        .is_err());
    assert!(session
        .prepare_biome_brush_cancellable(
            &[Bounds::new((0, -48, 0), (0, -48, 0)).unwrap()],
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false
            },
            &target,
            || false
        )
        .is_err());
    session.undo().unwrap();
    assert!(!session.has_pending_changes());
    session.redo().unwrap();
    save_and_rollback(&fixture, &world, &session, &changed);
}

fn hollow_modern_quarts() {
    use mcedit2_world::editing::BrushMask;
    let fixture = NativeWorld::create(true);
    let raw = nbt_document::patch(
        &modern_chunk("minecraft:stone"),
        &Operation::InsertField {
            path: "/sections/0".into(),
            name: "biomes".into(),
            value: Value::Compound(vec![Field {
                name: "palette".into(),
                value: Value::List(List {
                    element_type: 8,
                    items: vec![Value::String("minecraft:plains".into())],
                }),
            }]),
        },
    )
    .unwrap();
    overwrite_region(&fixture, &raw);
    let before = fixture.bytes_now();
    let mut session = Session::new(fixture.metadata(), "minecraft:overworld").unwrap();
    let bounds = Bounds::new((0, -64, 0), (11, -53, 11)).unwrap();
    let p = session
        .prepare_biome_brush_cancellable(
            &[bounds],
            BrushMask {
                shape: None,
                hollow: true,
            },
            &BiomeTarget::Named("custom:alpine".into()),
            || false,
        )
        .unwrap();
    assert_eq!(
        p.quart_cells, 26,
        "3×3×3 四格范围的空心笔画不得改动中心单元"
    );
    session.commit_structure(p.into_structure()).unwrap();
    let data = decode_sections(session.staged_biome_chunk(0, 0).unwrap().unwrap()).unwrap();
    assert_eq!(data.biome(4, -60, 4), Some("minecraft:plains"));
    assert_eq!(data.biome(0, -60, 4), Some("custom:alpine"));
    session.undo().unwrap();
    assert!(!session.has_pending_changes());
    session.redo().unwrap();
    assert_eq!(fixture.bytes_now(), before);
}
