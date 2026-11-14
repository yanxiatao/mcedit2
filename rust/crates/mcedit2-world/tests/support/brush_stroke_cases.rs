//! 连续笔画整批验证：整数路径、并集、过滤、源证明与真实会话历史。
use super::*;
use mcedit2_world::editing::{brush_bounds, BrushPath};
pub(super) fn continuous_block_brush_pipeline() {
    let mut path = BrushPath::default();
    path.push([-3, -64, 0]).unwrap();
    path.push([3, -62, 1]).unwrap();
    let points: Vec<_> = path.points().collect();
    assert_eq!(points.len(), 7);
    assert_eq!(points.first(), Some(&[-3, -64, 0]));
    assert_eq!(points.last(), Some(&[3, -62, 1]));
    for pair in points.windows(2) {
        assert_eq!(
            (0..3).map(|i| (pair[1][i] - pair[0][i]).abs()).max(),
            Some(1)
        );
    }
    path.push([3, -62, 1]).unwrap();
    assert_eq!(path.points().count(), 7);
    let before: Vec<_> = path.points().collect();
    assert!(path.push([30_000_000, 0, 0]).is_err());
    assert_eq!(path.points().collect::<Vec<_>>(), before);
    path.break_segment();
    path.push([100, -62, 1]).unwrap();
    assert_eq!(path.points().count(), 8);
    let even = brush_bounds([0, 0, 0], [4, 2, 2]).unwrap();
    assert_eq!(even.minimum, [-2, -1, -1]);
    assert_eq!(even.maximum, [1, 0, 0]);
    assert!(brush_bounds([0, 0, 0], [0, 1, 1]).is_err());
    assert!(brush_bounds([0, 0, 0], [513, 1, 1]).is_err());
    assert!(brush_bounds([i32::MAX, 0, 0], [4, 1, 1]).is_err());
    let fixture = NativeWorld::create(true);
    let world = fixture.metadata();
    let mut session = Session::new(world, "minecraft:overworld").unwrap();
    let mut path = BrushPath::default();
    path.push([1, -64, 1]).unwrap();
    path.push([4, -64, 1]).unwrap();
    let mut strokes = path
        .points()
        .map(|p| brush_bounds(p, [1, 1, 1]).unwrap())
        .collect::<Vec<_>>();
    strokes.push(strokes[0]);
    let stone = parse_state("minecraft:stone").unwrap();
    let log = parse_state("minecraft:oak_log[axis=y]").unwrap();
    let witnesses = vec![([1, -64, 1], stone.clone()), ([4, -64, 1], stone.clone())];
    session.verify_brush_sources(&witnesses).unwrap();
    let preview = session
        .prepare_brush_stroke_cancellable(
            &strokes,
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false,
            },
            &mcedit2_world::editing::BrushPaint::Replace(vec![
                mcedit2_world::editing::BrushReplacement {
                    sources: vec![stone.clone()],
                    target: log.clone(),
                },
            ]),
            || false,
        )
        .unwrap();
    assert_eq!(preview.examined, 4);
    assert_eq!(preview.changed(), 4);
    assert_eq!(preview.source_chunks, 1);
    session.commit(preview).unwrap();
    assert_eq!(session.pending_cells(), 4);
    assert!(session.verify_brush_sources(&witnesses).is_err());
    session.undo().unwrap();
    assert_eq!(session.pending_cells(), 0);
    session.verify_brush_sources(&witnesses).unwrap();
    session.redo().unwrap();
    assert_eq!(session.pending_cells(), 4);
    let filtered = session
        .prepare_brush_stroke_cancellable(
            &strokes,
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false,
            },
            &mcedit2_world::editing::BrushPaint::Replace(vec![
                mcedit2_world::editing::BrushReplacement {
                    sources: vec![stone.clone()],
                    target: stone.clone(),
                },
            ]),
            || false,
        )
        .unwrap();
    assert_eq!(filtered.matched, 0);
    assert_eq!(filtered.changed(), 0);
    assert!(session
        .prepare_brush_stroke_cancellable(
            &strokes,
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false
            },
            &mcedit2_world::editing::BrushPaint::Fill(stone.clone()),
            || true
        )
        .is_err());
    let mut missing = strokes.clone();
    missing.push(Bounds::new((16, -64, 0), (16, -64, 0)).unwrap());
    assert!(session
        .prepare_brush_stroke_cancellable(
            &missing,
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false
            },
            &mcedit2_world::editing::BrushPaint::Fill(stone.clone()),
            || false
        )
        .is_err());
    assert_eq!(session.pending_cells(), 4);
    assert_eq!(fixture.bytes_now(), fixture.original_region);
    session.undo().unwrap();
    let preview = session
        .prepare_brush_stroke_cancellable(
            &strokes,
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false,
            },
            &mcedit2_world::editing::BrushPaint::Fill(log.clone()),
            || false,
        )
        .unwrap();
    overwrite_region(&fixture, &modern_chunk("minecraft:dirt"));
    assert!(session.commit(preview).is_err());
    assert_eq!(session.pending_cells(), 0);
    assert!(session.verify_brush_sources(&witnesses).is_err());
    let numeric = NativeWorld::create(false);
    let mut session = Session::new(numeric.metadata(), "minecraft:overworld").unwrap();
    let p = brush_bounds([0, 0, 0], [1, 1, 1]).unwrap();
    let target = parse_state("numeric:41[data=0]").unwrap();
    let preview = session
        .prepare_brush_stroke_cancellable(
            &[p, p],
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false,
            },
            &mcedit2_world::editing::BrushPaint::Fill(target.clone()),
            || false,
        )
        .unwrap();
    assert_eq!(preview.examined, 1);
    session.commit(preview).unwrap();
    session.undo().unwrap();
    assert_eq!(session.pending_cells(), 0);
    staged_block_entities_are_checked_before_a_stroke_or_commit();
    hollow_union_and_replacement_rules();
}

fn hollow_union_and_replacement_rules() {
    use mcedit2_world::{
        block_lookup::Lookup,
        editing::{BrushMask, BrushPaint, BrushReplacement, BrushShape},
    };
    let fixture = NativeWorld::create(true);
    let raw = fixture.bytes_now();
    let mut session = Session::new(fixture.metadata(), "minecraft:overworld").unwrap();
    let stone = parse_state("minecraft:stone").unwrap();
    let gold = parse_state("minecraft:gold_block").unwrap();
    let state = |s: &Session, p: (i32, i32, i32)| match s.visible_block(p).unwrap() {
        Lookup::Present(value) => value.state,
        _ => panic!("测试源方块必须存在"),
    };
    let stamps = [
        Bounds::new((0, -64, 0), (2, -62, 2)).unwrap(),
        Bounds::new((1, -64, 0), (3, -62, 2)).unwrap(),
    ];
    let mask = BrushMask {
        shape: None,
        hollow: true,
    };
    let p = session
        .prepare_brush_stroke_cancellable(&stamps, mask, &BrushPaint::Fill(gold.clone()), || false)
        .unwrap();
    assert_eq!(
        p.changed(),
        34,
        "4×3×3 并集只保留外壳，两个内部格不能因笔触重叠而留下隔板"
    );
    session.commit(p).unwrap();
    assert_eq!(session.undo_depth(), 1);
    for x in [1, 2] {
        assert_eq!(state(&session, (x, -63, 1)), stone);
    }
    assert_eq!(state(&session, (0, -63, 1)), gold);
    session.undo().unwrap();
    session.redo().unwrap();
    session.undo().unwrap();
    for shape in [
        None,
        Some(BrushShape::Ellipsoid),
        Some(BrushShape::CylinderY),
        Some(BrushShape::Diamond),
        Some(BrushShape::ParabolicDome),
    ] {
        let p = session
            .prepare_brush_stroke_cancellable(
                &[Bounds::new((0, -64, 0), (4, -60, 4)).unwrap()],
                BrushMask {
                    shape,
                    hollow: true,
                },
                &BrushPaint::Fill(gold.clone()),
                || false,
            )
            .unwrap();
        assert!(p.changed() > 0);
        session.commit(p).unwrap();
        assert_eq!(
            state(&session, (2, -62, 2)),
            stone,
            "每种实心形状的内部不能被空心笔刷改动"
        );
        assert_eq!(
            state(&session, (2, -60, 2)),
            gold,
            "每种形状的顶部中心应属于外边界"
        );
        session.undo().unwrap();
    }
    let log = parse_state("minecraft:oak_log[axis=y]").unwrap();
    let dirt = parse_state("minecraft:dirt").unwrap();
    let log_z = parse_state("minecraft:oak_log[axis=z]").unwrap();
    for (x, target) in [(0, &log), (2, &gold)] {
        let p = session
            .prepare(Bounds::new((x, -64, 0), (x, -64, 0)).unwrap(), target, None)
            .unwrap();
        session.commit(p).unwrap();
    }
    let depth = session.undo_depth();
    let paint = BrushPaint::Replace(vec![
        BrushReplacement {
            sources: vec![stone.clone(), log.clone()],
            target: dirt.clone(),
        },
        BrushReplacement {
            sources: vec![stone.clone()],
            target: gold.clone(),
        },
        BrushReplacement {
            sources: vec![gold.clone()],
            target: log_z.clone(),
        },
    ]);
    let bounds = Bounds::new((0, -64, 0), (3, -64, 0)).unwrap();
    let p = session
        .prepare_brush_stroke_cancellable(&[bounds], BrushMask::default(), &paint, || false)
        .unwrap();
    assert_eq!(p.changed(), 4);
    assert_eq!(p.matched, 4);
    session.commit(p).unwrap();
    assert_eq!(session.undo_depth(), depth + 1);
    for (x, expected) in [(0, &dirt), (1, &gold), (2, &log_z), (3, &gold)] {
        assert_eq!(&state(&session, (x, -64, 0)), expected);
    }
    session.undo().unwrap();
    assert_eq!(state(&session, (0, -64, 0)), log);
    assert_eq!(state(&session, (1, -64, 0)), stone);
    session.redo().unwrap();
    for paint in [
        BrushPaint::Replace(vec![]),
        BrushPaint::Replace(vec![BrushReplacement {
            sources: vec![],
            target: gold.clone(),
        }]),
        BrushPaint::Replace(vec![BrushReplacement {
            sources: vec![parse_state("numeric:1[data=0]").unwrap()],
            target: gold.clone(),
        }]),
    ] {
        assert!(session
            .prepare_brush_stroke_cancellable(&[bounds], BrushMask::default(), &paint, || false)
            .is_err());
    }
    assert!(session
        .prepare_brush_stroke_cancellable(&stamps, mask, &paint, || true)
        .is_err());
    assert_eq!(fixture.bytes_now(), raw);
}

fn staged_block_entities_are_checked_before_a_stroke_or_commit() {
    use mcedit2_world::{
        clipboard::{CaptureOptions, PasteOptions},
        structure_transform::Transform,
    };
    let fixture = NativeWorld::create(true);
    overwrite_region(&fixture, &chunk_with_be("minecraft:stone", (0, -64, 0)));
    let mut session = Session::new(fixture.metadata(), "minecraft:overworld").unwrap();
    let target = Bounds::new((1, -64, 0), (1, -64, 0)).unwrap();
    let replacement = parse_state("minecraft:oak_log[axis=y]").unwrap();
    let old_preview = session.prepare(target, &replacement, None).unwrap();
    let clip = session
        .capture_structure_cancellable(
            Bounds::new((0, -64, 0), (0, -64, 0)).unwrap(),
            CaptureOptions {
                entities: false,
                biomes: false,
            },
            || false,
        )
        .unwrap();
    let (p, _) = session
        .prepare_resampled_structure_paste_cancellable(
            &clip,
            Transform::default(),
            (1, -64, 0),
            PasteOptions {
                include_biomes: false,
            },
            None,
            || false,
        )
        .unwrap();
    session.commit_structure(p).unwrap();
    assert!(session
        .prepare_brush_stroke_cancellable(
            &[target],
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false
            },
            &mcedit2_world::editing::BrushPaint::Fill(replacement.clone()),
            || false
        )
        .is_err());
    assert!(session.commit(old_preview).is_err());
    assert_eq!(session.pending_cells(), 0);
    assert!(session.has_pending_changes());
}
