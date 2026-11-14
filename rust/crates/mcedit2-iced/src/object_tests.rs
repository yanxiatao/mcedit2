//! 对象 Inspector、实体工具与保存的完整生产任务链，沿用后台 / 导航类别。
use crate::{
    metadata_fixture::Fixture,
    metadata_tests::{self, background, deliver},
    nbt_tree,
    object_inspector::{self, Action},
    scene_layers, App, Message,
};
use mcedit2_world::{
    editing::{EditWorkBudget, ObjectTarget, Session},
    nbt_document::{Operation, Value},
    scene_annotations::{Kind, Reader},
};
use std::sync::Arc;
fn select(app: &mut App, chunk: (i32, i32), kind: Kind) {
    let key = app.current_key().unwrap();
    let session = app.display_session(key).unwrap().clone();
    let mut reader = Reader::new(session.original_world(), session.dimension()).unwrap();
    reader.set_display_session(&session).unwrap();
    let part = Arc::new(
        reader
            .read_with_overlays(
                chunk.0,
                chunk.1,
                session.staged_entity_chunk(chunk.0, chunk.1).unwrap(),
                session.staged_be_chunk(chunk.0, chunk.1).unwrap(),
            )
            .unwrap(),
    );
    let snapshot = Arc::new(scene_layers::Snapshot::build(
        key,
        1,
        std::iter::once(part),
        None,
    ));
    let selected = snapshot
        .markers
        .iter()
        .find(|marker| marker.marker().kind == kind)
        .unwrap_or_else(|| {
            panic!(
                "区块 {chunk:?} 缺少 {kind:?}；实际来源标记 {:?}",
                snapshot.markers
            )
        })
        .clone();
    app.scene_layers.snapshot = Some(snapshot);
    app.scene_layers.selected = Some(selected);
}
fn object(app: &mut App, action: Action) {
    let _ = crate::update(app, Message::ObjectInspector(action));
}
fn tree(app: &mut App, action: nbt_tree::Action) {
    object(app, Action::Tree(action));
}
fn scalar(app: &mut App, path: &str, kind: u8, value: &str) {
    tree(app, nbt_tree::Action::Select(path.into(), kind));
    tree(app, nbt_tree::Action::Scalar(value.into()));
    tree(app, nbt_tree::Action::ApplyScalar);
}
fn draft(app: &App) -> &mcedit2_world::editing::ObjectDraft {
    app.object_inspector.panel.as_ref().unwrap().draft()
}
fn open(app: &mut App) {
    let _ = background(app, Message::ObjectInspector(Action::Open));
    assert_eq!(app.advanced_section, crate::AdvancedSection::BlockEntities);
    assert!(
        app.object_inspector.panel.is_some(),
        "{}",
        app.object_inspector.status
    );
}
fn prepare(app: &mut App) {
    let _ = background(app, Message::ObjectInspector(Action::Prepare));
    assert!(
        app.object_inspector.reviewing(),
        "{}",
        app.object_inspector.status
    );
}
fn commit(app: &mut App) {
    let _ = background(app, Message::ObjectInspector(Action::Commit));
    assert!(
        !app.object_inspector.reviewing(),
        "{}",
        app.object_inspector.status
    );
}
fn save(app: &mut App) {
    app.mixed_phrase = "SAVE_BE_NBT".into();
    let _ = background(app, Message::MixedPrepare);
    assert!(
        app.worlds[0]
            .root
            .join(".mcedit2-mixed.pending.json")
            .is_file(),
        "{}",
        app.mixed_status
    );
    let task = background(app, Message::MixedFinish);
    let _ = deliver(app, task);
    assert!(
        !app.sessions[&(0, 0)].has_pending_changes(),
        "{}",
        app.mixed_status
    );
}
fn inspector_edit_preview_and_persistence() {
    for version in [3465, 3955] {
        let mut fixture = Fixture::new(version);
        crate::object_fixture::populate(&mut fixture);
        let terrain = fixture.file("region/r.0.0.mca");
        let entities = fixture.file("entities/r.0.0.mca");
        let mut app = metadata_tests::app(&fixture);
        select(&mut app, (0, 0), Kind::Entity);
        open(&mut app);
        let original = draft(&app).document().bytes().to_vec();
        scalar(&mut app, "/CustomName", 8, "桌面对象名字");
        scalar(&mut app, "/Health", 5, "18");
        let _ = crate::update(
            &mut app,
            Message::Workspace(crate::workspace_tabs::Action::Close(
                crate::workspace_tabs::CloseTarget::World(0),
            )),
        );
        assert_eq!(
            app.workspace.closing,
            Some(crate::workspace_tabs::CloseTarget::World(0))
        );
        let _ = crate::update(
            &mut app,
            Message::Workspace(crate::workspace_tabs::Action::CancelClose),
        );
        assert_eq!(
            draft(&app).document().inspect("/Health").unwrap(),
            Value::Float(18.)
        );
        tree(&mut app, nbt_tree::Action::Undo);
        assert_eq!(
            draft(&app).document().inspect("/Health").unwrap(),
            Value::Float(10.)
        );
        tree(&mut app, nbt_tree::Action::Redo);
        tree(
            &mut app,
            nbt_tree::Action::Json(
                serde_json::to_string(&vec![
                    Operation::Remove {
                        path: "/Queue/0".into(),
                    },
                    Operation::Replace {
                        path: "/Queue/2".into(),
                        value: Value::Int(99),
                    },
                ])
                .unwrap(),
            ),
        );
        tree(&mut app, nbt_tree::Action::ApplyBatch);
        assert_eq!(app.sessions[&(0, 0)].undo_depth(), 0);
        prepare(&mut app);
        assert!(app.reviewing_block_preview());
        let preview = app
            .display_session((0, 0))
            .unwrap()
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (0, 0),
                    path: "Entities/0".into(),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(preview.document().bytes(), draft(&app).document().bytes());
        assert_eq!(
            app.sessions[&(0, 0)]
                .inspect_object(
                    ObjectTarget::Entity {
                        chunk: (0, 0),
                        path: "Entities/0".into()
                    },
                    EditWorkBudget::default()
                )
                .unwrap()
                .document()
                .bytes(),
            original
        );
        object(&mut app, Action::Cancel);
        assert!(!app.object_inspector.reviewing());
        assert!(draft(&app).document().changed());
        prepare(&mut app);
        commit(&mut app);
        assert_eq!(app.sessions[&(0, 0)].undo_depth(), 1);
        assert!(!draft(&app).document().changed());
        assert_eq!(fixture.file("entities/r.0.0.mca"), entities);
        let _ = background(
            &mut app,
            Message::Transfer(crate::transfer_edit::Action::History(false)),
        );
        assert_eq!(app.sessions[&(0, 0)].undo_depth(), 0);
        let _ = background(
            &mut app,
            Message::Transfer(crate::transfer_edit::Action::History(true)),
        );
        select(&mut app, (0, 0), Kind::BlockEntity);
        open(&mut app);
        scalar(&mut app, "/CustomName", 8, "桌面箱子名字");
        scalar(
            &mut app,
            if version >= 3837 {
                "/Items/0/count"
            } else {
                "/Items/0/Count"
            },
            if version >= 3837 { 3 } else { 1 },
            "11",
        );
        prepare(&mut app);
        commit(&mut app);
        assert_eq!(app.sessions[&(0, 0)].undo_depth(), 2);
        assert_eq!(fixture.file("region/r.0.0.mca"), terrain);
        save(&mut app);
        let saved = Session::new(fixture.world.clone(), "minecraft:overworld").unwrap();
        let entity = saved
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (0, 0),
                    path: "Entities/0".into(),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(
            entity.document().inspect("/CustomName").unwrap(),
            Value::String("桌面对象名字".into())
        );
        assert_eq!(
            entity.document().inspect("/Queue/1").unwrap(),
            Value::Int(99)
        );
        let chest = saved
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (0, 0, 0),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(
            chest.document().inspect("/CustomName").unwrap(),
            Value::String("桌面箱子名字".into())
        );
        assert_eq!(
            chest.document().inspect("/Items/0/OpaqueItem").unwrap(),
            Value::ByteArray(vec![-1, 0, 127])
        );
        // 在途结果已取消、会话已变化及只读状态均不能覆盖正式会话。
        select(&mut app, (0, 0), Kind::Entity);
        open(&mut app);
        scalar(&mut app, "/Health", 5, "19");
        let task = crate::update(&mut app, Message::ObjectInspector(Action::Prepare));
        let result = crate::picking_integration_tests::edit_task_output(task);
        object(&mut app, Action::Cancel);
        let _ = crate::update(&mut app, result);
        assert!(!app.object_inspector.reviewing());
        prepare(&mut app);
        select(&mut app, (0, 0), Kind::Entity);
        assert!(app.scene_layers.snapshot.is_some());
        object(&mut app, Action::Close);
        assert!(app.scene_layers.snapshot.is_none());
        assert!(app.object_inspector.panel.is_none());
        assert!(!app.object_inspector.reviewing());
        select(&mut app, (0, 0), Kind::Entity);
        open(&mut app);
        app.readonly = true;
        app.readonly_global = true;
        let before = draft(&app).document().bytes().to_vec();
        scalar(&mut app, "/Health", 5, "1");
        assert_eq!(draft(&app).document().bytes(), before);
        app.readonly = false;
        app.readonly_global = false;
        let task = crate::update(&mut app, Message::ObjectInspector(Action::Open));
        let result = crate::picking_integration_tests::edit_task_output(task);
        let _ = crate::update(&mut app, Message::WorkspaceIdle);
        let _ = crate::update(&mut app, result);
        assert!(app.current.is_none());
        assert!(!app.object_inspector.reviewing());
    }
}
fn entity_tools_and_cross_region_save() {
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        crate::object_fixture::populate(&mut fixture);
        let source = if version == 1343 {
            "region/r.0.0.mca"
        } else {
            "entities/r.0.0.mca"
        };
        let entities = fixture.file(source);
        let mut app = metadata_tests::app(&fixture);
        select(&mut app, (0, 0), Kind::Entity);
        let _ = crate::update(&mut app, Message::EntityDelta("16,0,0".into()));
        let _ = crate::update(&mut app, Message::EntityPrepareTranslate);
        assert!(
            app.entity_edit_preview.is_some(),
            "{}",
            app.entity_edit_status
        );
        let _ = crate::update(&mut app, Message::EntityApplyPreview);
        assert_eq!(app.sessions[&(0, 0)].undo_depth(), 1);
        select(&mut app, (1, 0), Kind::Entity);
        let _ = crate::update(&mut app, Message::EntityDelta("496,0,0".into()));
        let _ = background(&mut app, Message::EntityPrepareClone);
        assert!(
            app.entity_edit_preview.is_some(),
            "{}",
            app.entity_edit_status
        );
        let _ = crate::update(&mut app, Message::EntityApplyPreview);
        assert_eq!(app.sessions[&(0, 0)].undo_depth(), 2);
        let original = app.sessions[&(0, 0)]
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (1, 0),
                    path: "Entities/0".into(),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        let clone = app.sessions[&(0, 0)]
            .inspect_object(
                ObjectTarget::Entity {
                    chunk: (32, 0),
                    path: "Entities/0".into(),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        for parent in ["", "/Passengers/0"] {
            let identity = if version == 1343 {
                vec!["UUIDMost", "UUIDLeast"]
            } else {
                vec!["UUID"]
            };
            let before = identity
                .iter()
                .map(|name| {
                    original
                        .document()
                        .inspect(&format!("{parent}/{name}"))
                        .unwrap()
                })
                .collect::<Vec<_>>();
            let after = identity
                .iter()
                .map(|name| {
                    clone
                        .document()
                        .inspect(&format!("{parent}/{name}"))
                        .unwrap()
                })
                .collect::<Vec<_>>();
            assert_ne!(before, after);
        }
        assert_eq!(
            clone.document().inspect("/Pos/0").unwrap(),
            Value::Double(514.)
        );
        assert_eq!(fixture.file(source), entities);
        assert!(!fixture.root.join("entities/r.1.0.mca").exists());
        save(&mut app);
        assert_eq!(
            fixture.root.join("entities/r.1.0.mca").is_file(),
            version != 1343
        );
        select(&mut app, (32, 0), Kind::Entity);
        let _ = crate::update(&mut app, Message::EntityPrepareDelete);
        assert!(
            app.entity_edit_preview.is_some(),
            "{}",
            app.entity_edit_status
        );
        let _ = crate::update(&mut app, Message::EntityApplyPreview);
        assert_eq!(
            app.sessions[&(0, 0)]
                .inspect_entities(32, 0)
                .unwrap()
                .entities
                .len(),
            0
        );
        save(&mut app);
        assert_eq!(
            Session::new(fixture.world.clone(), "minecraft:overworld")
                .unwrap()
                .inspect_entities(32, 0)
                .unwrap()
                .entities
                .len(),
            0
        );
    }
}
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Background {
        inventory_editor_preview_and_save();
        all_container_transactions();
        special_forms_editor_and_save();
        inspector_edit_preview_and_persistence();
        entity_tools_and_cross_region_save();
        chunk_properties_tree_and_save();
        created_legacy_chunk_background_save();
    }
    if category == crate::test_categories::Category::Navigation {
        chunk_outline_and_view_picking();
        command_form_navigation();
        let mut fixture = Fixture::new(3955);
        crate::object_fixture::populate(&mut fixture);
        let mut app = metadata_tests::app(&fixture);
        select(&mut app, (0, 0), Kind::Entity);
        open(&mut app);
        let _ = object_inspector::view(&app);
        let _ = crate::view(&app);
        scalar(&mut app, "/Health", 5, "20");
        let revision = app.sessions[&(0, 0)].revision();
        let bytes = draft(&app).document().bytes().to_vec();
        for page in [
            object_inspector::WorkspacePage::Entities,
            object_inspector::WorkspacePage::Batch,
            object_inspector::WorkspacePage::Snapshots,
            object_inspector::WorkspacePage::Object,
        ] {
            object(&mut app, Action::WorkspacePage(page));
            let _ = crate::view(&app);
            assert_eq!(app.object_inspector.workspace_page, page);
            assert!(app.object_inspector.has_document(app.current_key()));
            assert_eq!(app.object_inspector.unsaved_world(), Some(0));
            assert_eq!(draft(&app).document().bytes(), bytes);
            assert!(Arc::ptr_eq(&revision, &app.sessions[&(0, 0)].revision()));
            assert_eq!(app.sessions[&(0, 0)].undo_depth(), 0);
        }
        object(&mut app, Action::Open);
        object(&mut app, Action::OpenChunk);
        assert_eq!(draft(&app).document().bytes(), bytes);
        prepare(&mut app);
        let _ = object_inspector::review(&app);
        let _ = crate::view(&app);
        object(&mut app, Action::Close);
        object(&mut app, Action::ChunkCoordinates("0,0".into()));
        let _ = background(&mut app, Message::ObjectInspector(Action::OpenChunk));
        assert_eq!(app.advanced_section, crate::AdvancedSection::Chunks);
        for page in [
            object_inspector::ChunkPage::Properties,
            object_inspector::ChunkPage::Nbt,
        ] {
            object(&mut app, Action::ChunkPage(page));
            let _ = crate::view(&app);
        }
    }
}

fn chunk_properties_tree_and_save() {
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        crate::object_fixture::populate(&mut fixture);
        let original = fixture.file("region/r.0.0.mca");
        let mut app = metadata_tests::app(&fixture);
        object(&mut app, Action::ChunkCoordinates("0,0".into()));
        let _ = background(&mut app, Message::ObjectInspector(Action::OpenChunk));
        assert_eq!(app.advanced_section, crate::AdvancedSection::Chunks);
        assert!(matches!(
            draft(&app).target(),
            ObjectTarget::Chunk { chunk: (0, 0) }
        ));
        let prefix = if version == 1343 { "/Level" } else { "" };
        let path = format!("{prefix}/InhabitedTime");
        object(
            &mut app,
            Action::PropertyInput(path.clone(), i64::MAX.to_string()),
        );
        object(&mut app, Action::SetProperty(path.clone()));
        assert_eq!(
            draft(&app).document().inspect(&path).unwrap(),
            Value::Long(i64::MAX)
        );
        let before = draft(&app).document().bytes().to_vec();
        object(
            &mut app,
            Action::PropertyInput(path.clone(), "无效整数".into()),
        );
        object(&mut app, Action::SetProperty(path.clone()));
        assert_eq!(draft(&app).document().bytes(), before);
        assert!(app.object_inspector.status.contains("Long"));
        object(
            &mut app,
            Action::ChunkPage(object_inspector::ChunkPage::Nbt),
        );
        scalar(
            &mut app,
            &format!("{prefix}/OpaqueMetadata/Queue/1"),
            3,
            "77",
        );
        prepare(&mut app);
        assert_eq!(fixture.file("region/r.0.0.mca"), original);
        commit(&mut app);
        save(&mut app);
        let saved = Session::new(fixture.world.clone(), "minecraft:overworld")
            .unwrap()
            .inspect_object(
                ObjectTarget::Chunk { chunk: (0, 0) },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(
            saved.document().inspect(&path).unwrap(),
            Value::Long(i64::MAX)
        );
        assert_eq!(
            saved
                .document()
                .inspect(&format!("{prefix}/OpaqueMetadata/Queue/1"))
                .unwrap(),
            Value::Int(77)
        );
        let _ = background(&mut app, Message::ObjectInspector(Action::OpenChunk));
        app.readonly = true;
        app.readonly_global = true;
        let before = draft(&app).document().bytes().to_vec();
        object(
            &mut app,
            Action::Flag(
                format!(
                    "{prefix}/{}",
                    if version == 1343 {
                        "TerrainPopulated"
                    } else {
                        "isLightOn"
                    }
                ),
                false,
            ),
        );
        assert_eq!(draft(&app).document().bytes(), before);
        let task = crate::update(&mut app, Message::ObjectInspector(Action::OpenChunk));
        let result = crate::picking_integration_tests::edit_task_output(task);
        let _ = crate::update(&mut app, Message::WorkspaceIdle);
        let _ = crate::update(&mut app, result);
        assert!(app.current.is_none());
    }
}

// 在现有导航类别中走真实源、异步文档及所有点选消息，不添加测试入口。
fn chunk_outline_and_view_picking() {
    use crate::metadata_fixture::{field, gzip};
    use crate::projected_entities::Mapping;
    let mut fixture = Fixture::new(3955);
    crate::object_fixture::populate(&mut fixture);
    let mut app = metadata_tests::app(&fixture);
    object(&mut app, Action::ChunkCoordinates("0,0".into()));
    let _ = background(&mut app, Message::ObjectInspector(Action::OpenChunk));
    assert!(
        object_inspector::selected_column(&app).is_none(),
        "没有维度定义不能猜测高度"
    );
    assert!(matches!(draft(&app).target(), ObjectTarget::Chunk { .. }));
    let mut doc = mcedit2_world::nbt_file::FileDocument::open(
        &fixture.root,
        &fixture.root.join("level.dat"),
        true,
    )
    .unwrap()
    .document;
    doc.apply_batch(&[Operation::InsertField {
        path: "/Data".into(),
        name: "WorldGenSettings".into(),
        value: Value::Compound(vec![field(
            "dimensions",
            Value::Compound(vec![field(
                "minecraft:overworld",
                Value::Compound(vec![field(
                    "type",
                    Value::Compound(vec![
                        field("min_y", Value::Int(-128)),
                        field("height", Value::Int(1024)),
                        field("has_skylight", Value::Byte(1)),
                    ]),
                )]),
            )]),
        )]),
    }])
    .unwrap();
    fixture.level = gzip(doc.bytes());
    std::fs::write(fixture.root.join("level.dat"), &fixture.level).unwrap();
    fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
    app = metadata_tests::app(&fixture);
    object(&mut app, Action::PickChunks(true));
    assert!(!app.show_advanced);
    assert_eq!(app.workspace_pane, crate::workspace_ui::Pane::Inspect);
    let _ = crate::brush_stroke::update(&mut app, crate::brush_stroke::Action::Enabled(true));
    assert!(!app.object_inspector.picking_chunks);
    assert!(crate::brush_stroke::config(&app).is_some());
    object(&mut app, Action::PickChunks(true));
    assert!(crate::brush_stroke::config(&app).is_none());
    object(&mut app, Action::ChunkCoordinates("0,0".into()));
    let _ = background(&mut app, Message::ObjectInspector(Action::OpenChunk));
    let column = object_inspector::selected_column(&app).unwrap();
    assert_eq!(column.0.minimum, [-0.5, -128.5, -0.5]);
    assert_eq!(column.0.maximum, [15.5, 895.5, 15.5]);
    // 每种正交平面用自身屏幕轴和切片厚度投影；边界外的切片不显示。
    for (right, down, depth, expected) in [
        (
            [1., 0., 0.],
            [0., 0., 1.],
            [0., 1., 0.],
            iced::Size::new(32., 32.),
        ),
        (
            [1., 0., 0.],
            [0., -1., 0.],
            [0., 0., -1.],
            iced::Size::new(32., 2048.),
        ),
        (
            [0., 0., 1.],
            [0., -1., 0.],
            [1., 0., 0.],
            iced::Size::new(32., 2048.),
        ),
    ] {
        let mapping = Mapping::Orthogonal {
            origin: [0.; 3],
            screen: [70., 50.],
            right,
            down,
            depth,
            scale: 2.,
        };
        let (_, size) = column
            .planar(mapping, (column.0.minimum, column.0.maximum))
            .unwrap();
        assert_eq!(size, expected);
        assert!(column.planar(mapping, ([16.; 3], [32.; 3])).is_none());
        // +/- 世界边界仅将局部差转为 f32，屏幕矩形保持完全一致。
        let offset = [29_999_984., 0., -29_999_984.];
        let moved = crate::chunk_outline::Column(crate::orthographic_framing::Bounds {
            minimum: std::array::from_fn(|i| column.0.minimum[i] + offset[i]),
            maximum: std::array::from_fn(|i| column.0.maximum[i] + offset[i]),
        });
        let moved_mapping = Mapping::Orthogonal {
            origin: offset,
            screen: [70., 50.],
            right,
            down,
            depth,
            scale: 2.,
        };
        assert_eq!(
            moved.planar(moved_mapping, (moved.0.minimum, moved.0.maximum)),
            column.planar(mapping, (column.0.minimum, column.0.maximum))
        );
    }
    let camera = crate::spectator::Camera {
        position: [8., 64., -20.],
        yaw: 0.,
        pitch: 0.,
        zoom: 1.,
        perspective: true,
    };
    let lines = column.segments(camera.into(), iced::Size::new(400., 300.));
    assert!(!lines.is_empty());
    assert!(lines
        .iter()
        .flat_map(|(a, b)| [a, b])
        .all(|p| (0. ..=400.).contains(&p.x) && (0. ..=300.).contains(&p.y)));
    let area = "-3,8,-4;7,14,9".to_owned();
    let corners = (Some((-3, 8, -4)), Some((7, 14, 9)));
    app.edit_area = area.clone();
    app.corner_a = corners.0;
    app.corner_b = corners.1;
    let bounds = mcedit2_world::editing::Bounds::new((0, 0, 0), (0, 0, 0)).unwrap();
    let slice = app
        .current_session((0, 0))
        .unwrap()
        .inspect_slice(bounds)
        .unwrap();
    app.slice_grid = Some((0, 0, Arc::new(slice)));
    app.readonly = true;
    app.readonly_global = true;
    let _ = background(&mut app, Message::PickSlice(0, 0, 0));
    assert!(!app.show_advanced);
    assert_eq!(app.edit_area, area);
    assert_eq!((app.corner_a, app.corner_b), corners);
    assert!(object_inspector::selected_column(&app).is_some());
    let vertical = app
        .current_session((0, 0))
        .unwrap()
        .inspect_vertical_slice(bounds)
        .unwrap();
    app.vertical_grid = Some((0, 0, Arc::new(vertical)));
    let _ = background(&mut app, Message::PickVertical(0, 0));
    assert_eq!(app.edit_area, area);
    // 四窗点击保留构图，不为区块检查重新居中读取四视图。
    let _ = crate::update(
        &mut app,
        Message::WorkspaceView(crate::workspace_ui::View::Quad),
    );
    let _ = crate::update(
        &mut app,
        Message::Quad(crate::quad_view::Action::Center("0,0,0".into())),
    );
    let _ = background(&mut app, Message::Quad(crate::quad_view::Action::Load));
    let source = app.quad.source_scope().unwrap();
    for plane in 0..3 {
        let _ = background(
            &mut app,
            Message::Quad(crate::quad_view::Action::Pick(
                plane,
                if plane == 0 { 15 } else { 16 },
                15,
            )),
        );
        assert_eq!(app.quad.source_scope().as_ref(), Some(&source));
        assert!(app.show_advanced);
        assert_eq!(app.advanced_section, crate::AdvancedSection::Quad);
        assert_eq!(app.edit_area, area);
        assert_eq!(object_inspector::selected_column(&app), Some(column));
    }
    let _ = background(
        &mut app,
        Message::Quad(crate::quad_view::Action::Scene(Box::new(
            Message::PickSurfaceFace(0, 0, 0, crate::isometric::Face::Top),
        ))),
    );
    assert_eq!(app.quad.source_scope().as_ref(), Some(&source));
    object(
        &mut app,
        Action::ChunkPage(object_inspector::ChunkPage::Nbt),
    );
    let _ = crate::view(&app);
    app.dimension_index = 1;
    assert!(object_inspector::selected_column(&app).is_none());
    app.dimension_index = 0;
    assert_eq!(object_inspector::selected_column(&app), Some(column));
    object(&mut app, Action::Close);
    assert!(!app.object_inspector.picking_chunks);
    assert!(object_inspector::selected_column(&app).is_none());
    // 真实旧世界的生产网格点选，包含负区块坐标，复用已呈现的源请求。
    let (mut app, _, _, mut request) = crate::picking_integration_tests::fixture();
    app.manual_textures = true;
    let [x, _, z] = request.hit.owner.as_ref().unwrap().position;
    object(&mut app, Action::PickChunks(true));
    app.edit_area = area.clone();
    app.corner_a = corners.0;
    app.corner_b = corners.1;
    request.intent = crate::picking::Intent::CornerA;
    let _ = background(&mut app, Message::SpectatorPick(request));
    assert_eq!(
        draft(&app).target(),
        &ObjectTarget::Chunk {
            chunk: (x.div_euclid(16), z.div_euclid(16))
        }
    );
    assert!(!app.show_advanced);
    assert_eq!(app.edit_area, area);
    assert_eq!((app.corner_a, app.corner_b), corners);
    assert!(object_inspector::selected_column(&app).is_some());
    // 按实际导航读取俯视源；完整世界透视的流式源不能冒充限定 Canvas 源。
    let cx = x.div_euclid(16) * 16;
    let cz = z.div_euclid(16) * 16;
    let _ = crate::update(
        &mut app,
        Message::SurfaceArea(format!("{cx},0,{cz};{},15,{}", cx + 15, cz + 15)),
    );
    let _ = background(
        &mut app,
        Message::WorkspaceView(crate::workspace_ui::View::Overhead),
    );
    let source = &app.surface.as_ref().unwrap().2;
    let voxel = source.voxels.iter().find(|voxel| voxel.span == 1).unwrap();
    let (px, py, pz) = (voxel.x, voxel.y, voxel.z);
    let _ = background(
        &mut app,
        Message::PickSurfaceFace(px, py, pz, crate::isometric::Face::Top),
    );
    assert_eq!(app.edit_area, area);
    assert!(!app.sessions[&(0, 0)].has_pending_changes());
    let _ = crate::update(&mut app, Message::WorkspaceIdle);
    assert!(!app.object_inspector.picking_chunks);
    assert!(object_inspector::selected_column(&app).is_none());
}

fn created_legacy_chunk_background_save() {
    use mcedit2_world::{
        chunk_creation::{BiomeSource, ChunkCreationProfile},
        clipboard::{CaptureOptions, PasteOptions},
        editing::Bounds,
    };
    for existing in [false, true] {
        let mut fixture = Fixture::new(1343);
        crate::object_fixture::populate(&mut fixture);
        if existing {
            use mcedit2_world::{
                block_lookup::read_world_chunk_bytes,
                nbt_document::{Document, Operation},
            };
            let mut chunks = Vec::new();
            for chunk in [(0, 0), (1, 0)] {
                let mut doc = Document::new(
                    read_world_chunk_bytes(&fixture.world, "minecraft:overworld", chunk.0, chunk.1)
                        .unwrap()
                        .unwrap(),
                )
                .unwrap();
                doc.apply(&Operation::Replace {
                    path: "/Level/LightPopulated".into(),
                    value: Value::Byte(1),
                })
                .unwrap();
                chunks.push((chunk, doc.bytes().to_vec()));
            }
            crate::object_fixture::write_region(&fixture.root.join("region/r.0.0.mca"), &chunks);
            fixture.world = mcedit2_world::inspect_world(&fixture.root).unwrap();
        }
        let mut app = metadata_tests::app(&fixture);
        let session = app.current_session((0, 0)).unwrap();
        let clip = session
            .capture_structure_cancellable(
                Bounds::new((0, 0, 0), (3, 4, 3)).unwrap(),
                CaptureOptions {
                    entities: true,
                    biomes: false,
                },
                || false,
            )
            .unwrap();
        let profile = ChunkCreationProfile {
            data_version: 1343,
            min_y: 0,
            height: 256,
            has_skylight: true,
            dimension_source: "user:桌面旧区块创建".into(),
            initial_biome: "numeric:1".into(),
            biome_source: BiomeSource::LegacyNumeric {
                origin: "桌面验证已确认数值 biome".into(),
                id: 1,
            },
        };
        let prepared = session
            .prepare_structure_paste_with_creation_cancellable(
                &clip,
                if existing { (8, 85, 8) } else { (1024, 5, 0) },
                PasteOptions {
                    include_biomes: false,
                },
                Some(profile),
                || false,
            )
            .unwrap();
        app.sessions
            .get_mut(&(0, 0))
            .unwrap()
            .commit_structure(prepared)
            .unwrap();
        let lighting = crate::save_postprocess::Action::new(
            &app.worlds[0].root,
            "minecraft:overworld",
            crate::save_postprocess::Change::Enabled(true),
        );
        let _ = crate::update(&mut app, Message::SavePostprocess(lighting));
        object(
            &mut app,
            Action::ChunkCoordinates(if existing { "0,0" } else { "64,0" }.into()),
        );
        let _ = background(&mut app, Message::ObjectInspector(Action::OpenChunk));
        let chunk = if existing { (0, 0) } else { (64, 0) };
        assert_eq!(draft(&app).target(), &ObjectTarget::Chunk { chunk });
        save(&mut app);
        let loaded = mcedit2_world::inspect_world(&fixture.root).unwrap();
        let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
            &loaded,
            "minecraft:overworld",
            chunk.0,
            chunk.1,
        )
        .unwrap()
        .unwrap();
        let doc = mcedit2_world::nbt_document::Document::new(raw).unwrap();
        assert_eq!(
            doc.inspect("/Level/LightPopulated").unwrap(),
            Value::Byte(1)
        );
        assert!(fixture
            .root
            .join(if existing {
                "region/r.0.0.mca"
            } else {
                "region/r.2.0.mca"
            })
            .is_file());
        if existing {
            let saved =
                mcedit2_world::block_lookup::read_world_chunk(&loaded, "minecraft:overworld", 0, 0)
                    .unwrap()
                    .unwrap();
            assert!(saved.has_block_entity(8, 85, 8));
            assert_eq!(saved.block(8, 85, 8).unwrap().name, "numeric:54");
        }
        let saved = Session::new(loaded, "minecraft:overworld").unwrap();
        let entity = saved
            .inspect_object(
                ObjectTarget::Entity {
                    chunk,
                    path: format!("Entities/{}", if existing { 1 } else { 0 }),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(
            entity.document().inspect("/OpaqueEntity").unwrap(),
            Value::LongArray(vec![i64::MIN, i64::MAX])
        );
        assert_eq!(
            entity
                .document()
                .inspect("/Passengers/0/OpaquePassenger")
                .unwrap(),
            Value::ByteArray(vec![-128, -1, 0, 127])
        );
    }
}

fn inventory_editor_preview_and_save() {
    use crate::inventory_ui::{self, Action as ItemAction, Field as ItemField};
    use mcedit2_world::inventory::{Layout, Snapshot};
    for version in [1343, 3465, 3955] {
        let mut fixture = Fixture::new(version);
        crate::object_fixture::populate(&mut fixture);
        let mut app = metadata_tests::app(&fixture);
        select(&mut app, (0, 0), Kind::BlockEntity);
        open(&mut app);
        let binding = Arc::new(
            Snapshot::capture(
                draft(&app).document(),
                inventory_ui::spec("/Items", Layout::Chest, &fixture.root, Some(version)),
            )
            .unwrap(),
        );
        object(&mut app, Action::Inventory(ItemAction::Select(binding, 0)));
        let token = app
            .object_inspector
            .panel
            .as_ref()
            .unwrap()
            .inventory_token();
        let original = draft(&app).document().bytes().to_vec();
        app.readonly = true;
        app.readonly_global = true;
        object(&mut app, Action::Inventory(ItemAction::Copy(token.clone())));
        let frozen = app.item_clipboard.clone().unwrap();
        object(&mut app, Action::Inventory(ItemAction::Cut(token.clone())));
        assert_eq!(draft(&app).document().bytes(), original);
        app.readonly = false;
        app.readonly_global = false;
        object(
            &mut app,
            Action::Inventory(ItemAction::Input(
                token.clone(),
                ItemField::Id,
                "minecraft:diamond".into(),
            )),
        );
        object(
            &mut app,
            Action::Inventory(ItemAction::Input(
                token.clone(),
                ItemField::Count,
                "7".into(),
            )),
        );
        object(
            &mut app,
            Action::Inventory(ItemAction::Apply(token.clone())),
        );
        assert_eq!(
            draft(&app).document().inspect("/Items/0/id").unwrap(),
            Value::String("minecraft:diamond".into())
        );
        assert_eq!(draft(&app).document().undo_depth(), 1);
        object(&mut app, Action::Inventory(ItemAction::Delete(token)));
        assert_eq!(draft(&app).document().undo_depth(), 1);
        assert!(app.object_inspector.status.contains("已改变"));
        let token = app
            .object_inspector
            .panel
            .as_ref()
            .unwrap()
            .inventory_token();
        object(&mut app, Action::Inventory(ItemAction::MoveMode(token)));
        let target = Arc::new(
            Snapshot::capture(
                draft(&app).document(),
                inventory_ui::spec("/Items", Layout::Chest, &fixture.root, Some(version)),
            )
            .unwrap(),
        );
        object(&mut app, Action::Inventory(ItemAction::Select(target, 26)));
        assert_eq!(
            draft(&app).document().inspect("/Items/0/Slot").unwrap(),
            Value::Byte(26)
        );
        prepare(&mut app);
        tree(&mut app, nbt_tree::Action::Undo);
        assert!(!app.object_inspector.reviewing());
        assert_eq!(
            draft(&app).document().inspect("/Items/0/Slot").unwrap(),
            Value::Byte(0)
        );
        tree(&mut app, nbt_tree::Action::Redo);
        prepare(&mut app);
        commit(&mut app);
        assert!(Arc::ptr_eq(app.item_clipboard.as_ref().unwrap(), &frozen));
        save(&mut app);
        let saved = Session::new(
            mcedit2_world::inspect_world(&fixture.root).unwrap(),
            "minecraft:overworld",
        )
        .unwrap();
        let source = saved
            .inspect_object(
                ObjectTarget::BlockEntity {
                    position: (0, 0, 0),
                },
                EditWorkBudget::default(),
            )
            .unwrap();
        assert_eq!(
            source.document().inspect("/Items/0/Slot").unwrap(),
            Value::Byte(26)
        );
        assert_eq!(
            source.document().inspect("/Items/0/id").unwrap(),
            Value::String("minecraft:diamond".into())
        );
        assert_eq!(
            source.document().inspect("/Items/0/OpaqueItem").unwrap(),
            Value::ByteArray(vec![-1, 0, 127])
        );
        let _ = crate::update(
            &mut app,
            Message::NbtPath("playerdata/00000000-0000-0000-0000-000000000001.dat".into()),
        );
        let _ = background(&mut app, Message::NbtOpen);
        let token = metadata_tests::inventory_select(&mut app, "/Inventory", Layout::Player, 8);
        metadata_tests::inventory_action(&mut app, ItemAction::Paste(token, frozen.clone()));
        assert_eq!(
            app.nbt_document()
                .unwrap()
                .source
                .document
                .inspect("/Inventory/1/Slot")
                .unwrap(),
            Value::Byte(8)
        );
        assert_eq!(
            app.nbt_document()
                .unwrap()
                .source
                .document
                .inspect("/Inventory/1/OpaqueItem")
                .unwrap(),
            Value::ByteArray(vec![-1, 0, 127])
        );
        let _ = object_inspector::view(&app);
        let _ = crate::file_ui::document(&app);
        let _ = crate::view(&app);
    }
}

fn special_forms_editor_and_save() {
    use crate::block_forms::Action as Form;
    use mcedit2_world::{
        container_data,
        inspector_data::{Mode, Snapshot},
    };
    for version in [1343, 3465, 3955] {
        for id in [
            "minecraft:command_block",
            "minecraft:sign",
            "minecraft:hanging_sign",
        ] {
            let Some(reference) = container_data::references(Some(version)).unwrap().get(id) else {
                continue;
            };
            let mut fixture = Fixture::new(version);
            crate::object_fixture::container(&mut fixture, reference);
            let mut app = metadata_tests::app(&fixture);
            select(&mut app, (0, 0), Kind::BlockEntity);
            open(&mut app);
            let source = Arc::new(
                Snapshot::capture(draft(&app).document(), Some(version))
                    .unwrap()
                    .unwrap(),
            );
            let original = draft(&app).document().bytes().to_vec();
            let _ = object_inspector::view(&app);
            if id == "minecraft:command_block" {
                let paste = iced::widget::text_editor::Action::Edit(
                    iced::widget::text_editor::Edit::Paste(Arc::new(
                        "say 第一行\nsay 第二行".into(),
                    )),
                );
                object(
                    &mut app,
                    Action::Form(Form::Edit(Arc::clone(&source), "/Command".into(), paste)),
                );
                object(
                    &mut app,
                    Action::Form(Form::Input(
                        Arc::clone(&source),
                        "/SuccessCount".into(),
                        "9".into(),
                    )),
                );
                object(
                    &mut app,
                    Action::Form(Form::Input(
                        Arc::clone(&source),
                        "/LastExecution".into(),
                        i64::MAX.to_string(),
                    )),
                );
            } else {
                let path = if version == 1343 {
                    "/Text1"
                } else {
                    "/front_text/messages/0"
                };
                object(
                    &mut app,
                    Action::Form(Form::Input(
                        Arc::clone(&source),
                        path.into(),
                        "表单中文 😀".into(),
                    )),
                );
                object(
                    &mut app,
                    Action::Form(Form::Mode(Arc::clone(&source), path.into(), Mode::Json)),
                );
                object(
                    &mut app,
                    Action::Form(Form::Mode(Arc::clone(&source), path.into(), Mode::Plain)),
                );
                let path = if version == 1343 {
                    "/Text4"
                } else {
                    "/front_text/messages/3"
                };
                object(
                    &mut app,
                    Action::Form(Form::Mode(Arc::clone(&source), path.into(), Mode::Json)),
                );
                object(
                    &mut app,
                    Action::Form(Form::Input(
                        Arc::clone(&source),
                        path.into(),
                        "{\"text\":\"粗体\",\"bold\":true}".into(),
                    )),
                );
                object(
                    &mut app,
                    Action::Form(Form::Mode(Arc::clone(&source), path.into(), Mode::Plain)),
                );
                object(
                    &mut app,
                    Action::Form(Form::Mode(Arc::clone(&source), path.into(), Mode::Json)),
                );
                if version != 1343 {
                    object(
                        &mut app,
                        Action::Form(Form::Filtered(
                            Arc::clone(&source),
                            "/back_text/filtered_messages".into(),
                            true,
                        )),
                    );
                    object(
                        &mut app,
                        Action::Form(Form::Input(
                            Arc::clone(&source),
                            "/back_text/filtered_messages/2".into(),
                            "背面过滤".into(),
                        )),
                    );
                    object(
                        &mut app,
                        Action::Form(Form::Input(
                            Arc::clone(&source),
                            "/back_text/color".into(),
                            "blue".into(),
                        )),
                    );
                    object(
                        &mut app,
                        Action::Form(Form::Input(
                            Arc::clone(&source),
                            "/back_text/has_glowing_text".into(),
                            "1".into(),
                        )),
                    );
                    object(
                        &mut app,
                        Action::Form(Form::Input(
                            Arc::clone(&source),
                            "/is_waxed".into(),
                            "1".into(),
                        )),
                    );
                    object(
                        &mut app,
                        Action::Form(Form::Face(Arc::clone(&source), true)),
                    );
                    let _ = object_inspector::view(&app);
                }
            }
            assert_eq!(draft(&app).document().bytes(), original);
            assert_eq!(draft(&app).document().undo_depth(), 0);
            app.readonly = true;
            app.readonly_global = true;
            object(&mut app, Action::Form(Form::Apply(Arc::clone(&source))));
            assert_eq!(draft(&app).document().bytes(), original);
            app.readonly = false;
            app.readonly_global = false;
            object(&mut app, Action::Form(Form::Apply(Arc::clone(&source))));
            assert_eq!(
                draft(&app).document().undo_depth(),
                1,
                "{}",
                app.object_inspector.status
            );
            let expected = draft(&app).document().bytes().to_vec();
            object(
                &mut app,
                Action::Form(Form::Input(
                    Arc::clone(&source),
                    "/Command".into(),
                    "say 过期事件".into(),
                )),
            );
            assert_eq!(draft(&app).document().bytes(), expected);
            assert!(app.object_inspector.status.contains("已变化"));
            tree(&mut app, nbt_tree::Action::Undo);
            assert_eq!(draft(&app).document().bytes(), original);
            tree(&mut app, nbt_tree::Action::Redo);
            assert_eq!(draft(&app).document().bytes(), expected);
            for page in [
                object_inspector::ObjectPage::Nbt,
                object_inspector::ObjectPage::Properties,
            ] {
                object(&mut app, Action::ObjectPage(page));
                let _ = object_inspector::view(&app);
            }
            prepare(&mut app);
            commit(&mut app);
            assert_eq!(app.sessions[&(0, 0)].undo_depth(), 1);
            save(&mut app);
            let session = Session::new(
                mcedit2_world::inspect_world(&fixture.root).unwrap(),
                "minecraft:overworld",
            )
            .unwrap();
            let saved = session
                .inspect_object(
                    ObjectTarget::BlockEntity {
                        position: (0, 0, 0),
                    },
                    EditWorkBudget::default(),
                )
                .unwrap();
            assert_eq!(saved.document().bytes(), expected);
            if id == "minecraft:command_block" {
                let Value::String(command) = saved.document().inspect("/Command").unwrap() else {
                    panic!("命令必须为 String")
                };
                assert_eq!(command.replace("\r\n", "\n"), "say 第一行\nsay 第二行");
                assert_eq!(
                    saved.document().inspect("/LastExecution").unwrap(),
                    Value::Long(i64::MAX)
                );
            } else {
                let state = mcedit2_world::editing::parse_state(&reference.block).unwrap();
                let rendered = mcedit2_world::sign::parse(
                    &saved.document().bytes()[3..],
                    &state,
                    Some(version),
                )
                .unwrap();
                assert_eq!(rendered.front.messages[0]["text"], "表单中文 😀");
                assert_eq!(rendered.front.messages[3]["bold"], true);
                if version != 1343 {
                    assert!(rendered.waxed && rendered.back.glowing);
                    assert_eq!(
                        rendered.back.filtered_messages.unwrap()[2]["text"],
                        "背面过滤"
                    );
                }
            }
            assert_eq!(
                saved.document().inspect("/OpaqueContainer").unwrap(),
                Value::LongArray(vec![i64::MIN, i64::MAX])
            );
        }
    }
}
fn command_form_navigation() {
    use crate::block_forms::Action as Form;
    use mcedit2_world::{container_data, inspector_data::Snapshot};
    let mut fixture = Fixture::new(3955);
    crate::object_fixture::container(
        &mut fixture,
        &container_data::references(Some(3955)).unwrap()["minecraft:command_block"],
    );
    let mut app = metadata_tests::app(&fixture);
    select(&mut app, (0, 0), Kind::BlockEntity);
    open(&mut app);
    app.spectator_camera.yaw = -73.;
    app.spectator_camera.pitch = 26.;
    app.spectator_camera.zoom = 1.75;
    app.readonly = true;
    app.readonly_global = true;
    let source = Arc::new(
        Snapshot::capture(draft(&app).document(), Some(3955))
            .unwrap()
            .unwrap(),
    );
    let original = draft(&app).document().bytes().to_vec();
    object(
        &mut app,
        Action::Form(Form::Goto(Arc::clone(&source), None)),
    );
    assert!(app.show_advanced, "定位后仍应显示 Inspector");
    assert_eq!(app.query_coordinates, "0,0,0");
    assert_eq!(app.spectator_camera.yaw, -73.);
    assert_eq!(app.spectator_camera.pitch, 26.);
    assert_eq!(app.spectator_camera.zoom, 1.75);
    let (_, _, forward) = app.spectator_camera.basis();
    for axis in 0..3 {
        assert!(
            (app.spectator_camera.position[axis] + f64::from(forward[axis]) * 12. - 0.5).abs()
                < 1e-5
        );
    }
    object(
        &mut app,
        Action::Form(Form::Input(
            Arc::clone(&source),
            "/Command".into(),
            "execute positioned 10 20 30 run setblock ~1 ~2 ~3 minecraft:stone".into(),
        )),
    );
    object(
        &mut app,
        Action::Form(Form::Goto(Arc::clone(&source), Some(1))),
    );
    assert_eq!(app.query_coordinates, "11,22,33");
    assert_eq!(draft(&app).document().bytes(), original);
    assert_eq!(app.sessions[&(0, 0)].undo_depth(), 0);
    object(
        &mut app,
        Action::Form(Form::Input(
            Arc::clone(&source),
            "/Command".into(),
            "execute positioned ^ ^ ^ run setblock ~ ~ ~ minecraft:stone".into(),
        )),
    );
    let before = app.spectator_camera;
    object(
        &mut app,
        Action::Form(Form::Goto(Arc::clone(&source), Some(0))),
    );
    assert_eq!(app.spectator_camera, before);
    assert!(app.object_inspector.status.contains("局部坐标"));
    object(&mut app, Action::Form(Form::Reset(Arc::clone(&source))));
    let _ = object_inspector::view(&app);
    assert_eq!(draft(&app).document().undo_depth(), 0);
}
fn all_container_transactions() {
    use crate::inventory_ui::{self, Action as ItemAction, Field as ItemField};
    use mcedit2_world::{container_data, inventory::Snapshot};
    for version in [1343, 3465, 3955] {
        for id in [
            "minecraft:furnace",
            "minecraft:brewing_stand",
            "minecraft:jukebox",
            "minecraft:lectern",
            "minecraft:chiseled_bookshelf",
            "minecraft:crafter",
        ] {
            let Some(reference) = container_data::references(Some(version)).unwrap().get(id) else {
                continue;
            };
            let Some(layout) = container_data::layout(id, Some(version)) else {
                continue;
            };
            let mut fixture = Fixture::new(version);
            crate::object_fixture::container(&mut fixture, reference);
            let mut app = metadata_tests::app(&fixture);
            select(&mut app, (0, 0), Kind::BlockEntity);
            open(&mut app);
            let slot = *layout
                .rows()
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .last()
                .unwrap();
            let binding = Arc::new(
                Snapshot::capture(
                    draft(&app).document(),
                    inventory_ui::spec(
                        format!("/{}", layout.field()),
                        layout,
                        &fixture.root,
                        Some(version),
                    ),
                )
                .unwrap(),
            );
            object(
                &mut app,
                Action::Inventory(ItemAction::Select(binding, slot)),
            );
            let token = app
                .object_inspector
                .panel
                .as_ref()
                .unwrap()
                .inventory_token();
            let item = if layout == mcedit2_world::inventory::Layout::Jukebox {
                if version == 1343 {
                    "minecraft:record_13"
                } else {
                    "minecraft:music_disc_13"
                }
            } else if layout == mcedit2_world::inventory::Layout::Lectern {
                "minecraft:writable_book"
            } else {
                "minecraft:book"
            };
            object(
                &mut app,
                Action::Inventory(ItemAction::Input(token.clone(), ItemField::Id, item.into())),
            );
            let original = draft(&app).document().bytes().to_vec();
            app.readonly = true;
            app.readonly_global = true;
            object(
                &mut app,
                Action::Inventory(ItemAction::Apply(token.clone())),
            );
            assert_eq!(draft(&app).document().bytes(), original);
            app.readonly = false;
            app.readonly_global = false;
            object(&mut app, Action::Inventory(ItemAction::Apply(token)));
            assert_eq!(
                draft(&app).document().undo_depth(),
                1,
                "{id}: {}",
                app.object_inspector.status
            );
            let metadata = Arc::new(
                container_data::Snapshot::capture(draft(&app).document(), Some(version))
                    .unwrap()
                    .unwrap(),
            );
            let property = metadata.properties.first().cloned();
            if let Some(property) = &property {
                let depth = draft(&app).document().undo_depth();
                object(
                    &mut app,
                    Action::Container(crate::container_ui::Action::Input(
                        Arc::clone(&metadata),
                        property.name.clone(),
                        "1".into(),
                    )),
                );
                assert_eq!(draft(&app).document().undo_depth(), depth);
                app.readonly = true;
                app.readonly_global = true;
                object(
                    &mut app,
                    Action::Container(crate::container_ui::Action::Apply(Arc::clone(&metadata))),
                );
                assert_eq!(draft(&app).document().undo_depth(), depth);
                app.readonly = false;
                app.readonly_global = false;
                object(
                    &mut app,
                    Action::Container(crate::container_ui::Action::Apply(Arc::clone(&metadata))),
                );
                assert_eq!(draft(&app).document().undo_depth(), depth + 1);
            }
            for page in [
                object_inspector::ObjectPage::Properties,
                object_inspector::ObjectPage::Inventory,
                object_inspector::ObjectPage::Nbt,
            ] {
                object(&mut app, Action::ObjectPage(page));
                let _ = object_inspector::view(&app);
            }
            let expected = draft(&app).document().bytes().to_vec();
            prepare(&mut app);
            commit(&mut app);
            assert_eq!(
                app.current_session(app.current_key().unwrap())
                    .unwrap()
                    .undo_depth(),
                1
            );
            if version == 1343 {
                let lighting = crate::save_postprocess::Action::new(
                    &app.worlds[0].root,
                    "minecraft:overworld",
                    crate::save_postprocess::Change::Enabled(true),
                );
                let _ = crate::update(&mut app, Message::SavePostprocess(lighting));
            }
            save(&mut app);
            let saved = Session::new(
                mcedit2_world::inspect_world(&fixture.root).unwrap(),
                "minecraft:overworld",
            )
            .unwrap();
            let saved = saved
                .inspect_object(
                    ObjectTarget::BlockEntity {
                        position: (0, 0, 0),
                    },
                    EditWorkBudget::default(),
                )
                .unwrap();
            assert_eq!(saved.document().bytes(), expected, "{version} {id}");
            assert_eq!(
                saved.document().inspect("/OpaqueContainer").unwrap(),
                Value::LongArray(vec![i64::MIN, i64::MAX])
            );
            let inventory = Snapshot::capture(
                saved.document(),
                inventory_ui::spec(
                    format!("/{}", layout.field()),
                    layout,
                    &fixture.root,
                    Some(version),
                ),
            )
            .unwrap();
            assert_eq!(inventory.items[&slot].id, item);
        }
    }
    // 名称未改变时保留完整样式；明确改名和清空通过相同属性事务完成。
    let reference =
        &mcedit2_world::container_data::references(Some(3955)).unwrap()["minecraft:chest"];
    let mut doc = mcedit2_world::nbt_document::Document::new(reference.nbt.clone()).unwrap();
    let styled = r#"{"text":"旧名","bold":true}"#;
    doc.apply(&Operation::InsertField {
        path: "".into(),
        name: "CustomName".into(),
        value: Value::String(styled.into()),
    })
    .unwrap();
    let mut ui = crate::container_ui::State::default();
    for name in ["旧名", "新名", ""] {
        let source = Arc::new(
            mcedit2_world::container_data::Snapshot::capture(&doc, Some(3955))
                .unwrap()
                .unwrap(),
        );
        ui.update(
            &doc,
            crate::container_ui::Action::Input(
                Arc::clone(&source),
                "CustomName".into(),
                name.into(),
            ),
            true,
            None,
            crate::language::Language::Chinese,
        )
        .unwrap();
        let operations = ui
            .update(
                &doc,
                crate::container_ui::Action::Apply(source),
                true,
                None,
                crate::language::Language::Chinese,
            )
            .unwrap();
        if name == "旧名" {
            assert!(operations.is_empty());
            assert_eq!(
                doc.inspect("/CustomName").unwrap(),
                Value::String(styled.into())
            );
        } else {
            doc.apply_batch(&operations).unwrap();
            ui.invalidate();
        }
    }
    assert!(doc.inspect_field("", "CustomName").unwrap().is_none());
}
