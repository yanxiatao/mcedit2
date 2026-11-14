//! 各工具页分别构建控件，页面分派不预留其他页面的临时栈空间。
use crate::*;
use iced::widget::{column, Column};

pub(crate) fn overview<'a>(
    app: &'a App,
    world: &'a WorldSummary,
    mut body: Column<'a, Message>,
) -> Column<'a, Message> {
    let l = app.language;

    if let Some(name) = &world.level_name {
        body = body.push(text(name).size(18));
    }
    body = body.push(text(world.root.display().to_string()).size(13));
    body = body.push(text(format!(
        "{} {} · {} {}",
        world.total_chunks(),
        l.pick("个区块", "chunks"),
        world.total_regions(),
        l.pick("个区域文件", "region files")
    )));
    body = body.push(
        text(format!(
            "{} · {} {}",
            compatibility_label(world),
            l.pick("数据版本", "Data version"),
            world.data_version.map_or_else(
                || l.pick("未记录", "Not recorded").to_owned(),
                |version| version.to_string()
            )
        ))
        .size(13),
    );
    if app.debug {
        body = body.push(text(format!(
            "-debug={} · -log={} · verbosity={} · -view={}",
            app.debug, app.log, app.verbosity, app.readonly
        )));
    }
    body = body.push(
        button(l.pick(
            "世界属性：名称、出生点、时间与难度",
            "World properties: name, spawn, time and difficulty",
        ))
        .on_press(Message::WorldInfo),
    );
    body = body.push(text(l.pick("按坐标查询", "Look up coordinates")).size(20));
    let lookup = row![
        text_input(
            l.pick("X, Y, Z，例如 -32, -64, 16", "X, Y, Z e.g. -32, -64, 16"),
            &app.query_coordinates
        )
        .on_input(Message::Coordinates)
        .on_submit(Message::InspectBlock)
        .width(Length::Fill),
        button(l.pick("查询方块", "Inspect block")).on_press(Message::InspectBlock),
        button(l.pick("查询方块实体 NBT", "Inspect block entity NBT"))
            .on_press(Message::InspectOriginalBlockEntity)
    ]
    .spacing(8)
    .wrap();
    body = body
        .push(
            text(l.pick(
                "查询只读取数据，不会修改世界。",
                "Lookups are read-only and do not change the world.",
            ))
            .size(12),
        )
        .push(lookup);
    if !app.block_result.is_empty() {
        if let Some(name) = app.block_name_info() {
            body = body.push(text(name));
        }
        body = body.push(text(&app.block_result));
    }

    body
}

pub(crate) fn viewer_controls<'a>(
    app: &'a App,
    mut body: Column<'a, Message>,
) -> Column<'a, Message> {
    let l = app.language;

    body = body.push(text(l.pick("材质与 GPU", "Texture resources & GPU")).size(22));
    body = body.push(text(l.pick(
                "打开世界后自动发现原版 JAR、同实例 Mod JAR、KubeJS 资源和启用的资源包；手动资源包优先级最高。",
                "Automatically use this world's client, Mod JARs, KubeJS assets and enabled packs; manual overrides take priority."
            )));
    body = body.push(
        button(l.pick(
            "扫描并重新加载实例资源",
            "Rescan and reload instance assets",
        ))
        .on_press(Message::RefreshInstanceAssets),
    );
    body = body.push(
        row![
            text_input(
                l.pick(
                    "资源包低→高优先级，多个路径用 ; 隔开",
                    "Packs low -> high priority; separate .zip/.jar/folders with ;"
                ),
                &app.texture_path
            )
            .on_input(Message::TexturePath)
            .on_submit(Message::LoadTextures)
            .width(Length::Fill),
            button(l.pick("选 ZIP/JAR", "Select ZIP/JAR")).on_press(Message::ChooseTextureFile),
            button(l.pick("选资源目录", "Select folder")).on_press(Message::ChooseTextureDirectory),
            button(l.pick("加载材质", "Load textures")).on_press(Message::LoadTextures),
            button(l.pick("卸载", "Unload")).on_press(Message::UnloadTextures),
            button(if app.show_textures {
                l.pick("仅显示纯色", "Palette only")
            } else {
                l.pick("显示贴图", "Show textures")
            })
            .on_press(Message::ToggleTextures),
            button(if app.use_gpu {
                l.pick("使用 Canvas 预览", "Use Canvas fallback")
            } else {
                l.pick("使用 wgpu 渲染器", "Use wgpu renderer")
            })
            .on_press_maybe(
                (!app.spectator && !app.uses_world_mesh()).then_some(Message::ToggleGpu)
            ),
            button(text(format!(
                "{}: {}",
                l.pick("材质采样", "Texture sampling"),
                app.gpu_quality.label()
            )))
            .on_press(Message::CycleGpuQuality),
        ]
        .spacing(7)
        .wrap(),
    );
    if !app.texture_report.is_empty() {
        body = body.push(text(&app.texture_report));
    }
    body = body.push(crate::client_installations::ui::view(app));
    body = body.push(
        row![
            text_input(
                l.pick(
                    "只读 3D 范围 XYZ（最多 64×64×48）",
                    "Read-only 3D XYZ window (max 64x64x48)"
                ),
                &app.surface_area
            )
            .on_input(Message::SurfaceArea)
            .width(Length::Fill),
            button(l.pick("3D 地表", "Top-surface 3D")).on_press(Message::InspectSurface),
            button(l.pick("3D 多层可见面", "Exposed multi-layer 3D"))
                .on_press(Message::InspectVolume)
        ]
        .spacing(8)
        .wrap(),
    );
    body = body.push(
        row![
            button(l.pick("检查 16×16×32", "16×16×32 inspect")).on_press(Message::ScenePreset(16)),
            button(l.pick("检查 32×32×48", "32×32×48 inspect")).on_press(Message::ScenePreset(32)),
            button(l.pick("检查 64×64×48", "64×64×48 inspect")).on_press(Message::ScenePreset(64)),
            text(l.pick(
                "精确近景预览；大范围编辑按任务预算执行。",
                "Exact near-field preview; larger edits use the task budget."
            ))
        ]
        .spacing(8)
        .wrap(),
    );
    body = body
        .push(text(l.pick("远景与区块流式加载", "Distant terrain and chunk streaming")).size(18));
    body = body.push(
        row![
            text(l.pick("渐进式远景：", "Progressive distant terrain:")),
            button(l.pick("16 区块", "16 chunks")).on_press(Message::StartFarView(16)),
            button(l.pick("32 区块", "32 chunks")).on_press(Message::StartFarView(32)),
            button(l.pick("64 区块", "64 chunks")).on_press(Message::StartFarView(64)),
            button(l.pick("128 区块", "128 chunks")).on_press(Message::StartFarView(128)),
            button(l.pick("256 区块", "256 chunks")).on_press(Message::StartFarView(256)),
            button(l.pick("512 区块", "512 chunks")).on_press(Message::StartFarView(512)),
            button(l.pick("1024 区块", "1024 chunks")).on_press(Message::StartFarView(1024)),
        ]
        .spacing(6)
        .wrap(),
    );
    body = body.push(
        row![
            text(l.pick(
                "稀疏全景（仅现有区块）：",
                "Very sparse panorama (source chunks only):"
            )),
            button(l.pick("1536 区块", "1536 chunks")).on_press(Message::StartFarView(1536)),
            button(l.pick("2047 区块", "2047 chunks")).on_press(Message::StartFarView(2047)),
            text(l.pick(
                "最大跨度 65,520 格，仅显示已验证的区块列。",
                "Maximum span 65,520 blocks across; only verified columns are shown."
            )),
        ]
        .spacing(6)
        .wrap(),
    );
    if let Some(far) = app.far.as_ref() {
        if app.current_key() == Some(far.key) {
            body = body.push(
                row![
                    text(format!(
                        "Far preview {}/{} tiles, errors {}. LOD tiles are non-editable.",
                        far.complete, far.total, far.errors
                    )),
                    button("Far X -16 chunks").on_press(Message::ShiftFarView(-16, 0)),
                    button("Far X +16 chunks").on_press(Message::ShiftFarView(16, 0)),
                    button("Far Z -16 chunks").on_press(Message::ShiftFarView(0, -16)),
                    button("Far Z +16 chunks").on_press(Message::ShiftFarView(0, 16)),
                ]
                .spacing(6)
                .wrap(),
            );
        }
    }
    body = body.push(
        row![
            button("X -16").on_press(Message::Shift3D(-16, 0, 0)),
            button("X +16").on_press(Message::Shift3D(16, 0, 0)),
            button("Z -16").on_press(Message::Shift3D(0, 0, -16)),
            button("Z +16").on_press(Message::Shift3D(0, 0, 16)),
            button("Y -16").on_press(Message::Shift3D(0, -16, 0)),
            button("Y +16").on_press(Message::Shift3D(0, 16, 0)),
            button(l.pick("按 XYZ 居中", "Center on XYZ")).on_press(Message::Center3DOnPicked),
        ]
        .spacing(5)
        .wrap(),
    );
    if let Some((wi, di, view)) = &app.surface {
        if app.current_key() == Some((*wi, *di)) {
            body = body.push(
                row![
                    button(text(format!("Rotate 90° ({})", app.surface_camera))).on_press_maybe(
                        (app.active_view() == view_navigation::Target::Isometric)
                            .then_some(Message::RotateSurface)
                    ),
                    button(if app.show_top_down {
                        l.pick("切换到 3D", "Switch to 3D")
                    } else {
                        l.pick("俯视 2D", "Top-down 2D")
                    })
                    .on_press(Message::ToggleTopDown),
                    button(l.pick("放大", "Zoom +")).on_press(Message::ZoomSurface(1)),
                    button(l.pick("缩小", "Zoom -")).on_press(Message::ZoomSurface(-1)),
                    button(l.pick("左移", "Pan ←")).on_press_maybe(
                        (app.active_view() != view_navigation::Target::Perspective)
                            .then_some(Message::PanSurface(-40, 0))
                    ),
                    button(l.pick("右移", "Pan →")).on_press_maybe(
                        (app.active_view() != view_navigation::Target::Perspective)
                            .then_some(Message::PanSurface(40, 0))
                    ),
                    button(l.pick("上移", "Pan ↑")).on_press_maybe(
                        (app.active_view() != view_navigation::Target::Perspective)
                            .then_some(Message::PanSurface(0, -40))
                    ),
                    button(l.pick("下移", "Pan ↓")).on_press_maybe(
                        (app.active_view() != view_navigation::Target::Perspective)
                            .then_some(Message::PanSurface(0, 40))
                    ),
                    button(l.pick("重置相机", "Reset camera"))
                        .on_press(Message::ResetSurfaceCamera),
                ]
                .spacing(5)
                .wrap(),
            );
            let stats = view.statistics();
            body=body.push(text(format!(
                    "Scene: {} visible voxels / {} exposed source faces / {} inspected original chunk slots; BE={}, staged={}, liquid={}, glass={}, foliage={}, missing columns={}",
                    stats.voxels,stats.exposed_faces,stats.original_chunks,
                    stats.original_be,stats.staged,stats.liquids,
                    stats.transparent,stats.foliage,stats.missing_columns
                )));
            let unknown_tiles = view
                .source_tiles
                .iter()
                .filter(|t| !t.source_present)
                .count();
            let partial_tiles = view
                .source_tiles
                .iter()
                .filter(|t| t.source_present && t.unknown_columns > 0)
                .count();
            body=body.push(text(format!(
                    "Chunk tiles: {} present / {} absent / {} partially unknown. Green=known, amber=unknown columns or Sections, red=missing original chunk; display only.",
                    view.source_tiles.len()-unknown_tiles,unknown_tiles,partial_tiles
                )));
            body=body.push(row![
                    button(if app.surface_frame{"Hide 3D bounds"}else{"Show 3D bounds"})
                        .on_press(Message::Toggle3DFrame),
                    button(if app.surface_axes{"Hide XYZ axes"}else{"Show XYZ axes"})
                        .on_press(Message::Toggle3DAxes),
                    button(if app.surface_chunk_grid{"Hide chunk grid"}else{"Show chunk grid"})
                        .on_press(Message::Toggle3DChunkGrid),
                    button(if app.surface_section_guides{"Hide section Y"}else{"Show section Y"})
                        .on_press(Message::Toggle3DSections),
                    text("Axes: X red / Y green / Z blue. Outlined box is view clipping, not world geometry.")
                ].spacing(6));
            body = body.push(
                row![
                    button(l.pick("所选方块设为顶点 A", "Corner A from picked"))
                        .on_press(Message::Set3DCornerA),
                    button(l.pick("所选方块设为顶点 B", "Corner B from picked"))
                        .on_press(Message::Set3DCornerB),
                    button(l.pick("使用 3D 框选范围", "Use 3D box as edit selection"))
                        .on_press(Message::Use3DBox),
                    button(l.pick("清除 3D 框选", "Clear 3D box")).on_press(Message::Clear3DBox),
                    text(format!("A={:?} / B={:?}", app.corner_a, app.corner_b)),
                ]
                .spacing(5)
                .wrap(),
            );
            if app.surface_exposed {
                body = body.push(
                    row![
                        text(format!(
                            "3D cutaway top: {} (view only)",
                            app.surface_cut_top
                                .map_or_else(|| "full scene".into(), |y| y.to_string())
                        )),
                        button("Cut Y -8").on_press(Message::Cut3DTop(-8)),
                        button("Cut Y -1").on_press(Message::Cut3DTop(-1)),
                        button("Cut Y +1").on_press(Message::Cut3DTop(1)),
                        button("Cut Y +8").on_press(Message::Cut3DTop(8)),
                        button(l.pick("恢复完整高度", "Restore full height"))
                            .on_press(Message::Reset3DCut)
                    ]
                    .spacing(5)
                    .wrap(),
                );
            }
            body = body.push(text(format!(
                "3D renderer: {} / sampler {}. Source exposed faces: {}. {}",
                if app.use_gpu {
                    "wgpu atlas triangles + Canvas interaction"
                } else {
                    "Canvas fallback"
                },
                app.gpu_quality.label(),
                stats.exposed_faces,
                l.pick(
                    "按设备限制分段绘制，无固定面数上限。",
                    "Device-sized draw segments; no fixed face cap."
                )
            )));
            if let Some(bank) = &app.texture_bank {
                let atlas = bank
                    .atlas_size()
                    .map_or_else(|| "none".to_string(), |(w, h)| format!("{w}x{h} RGBA"));
                body=body.push(text(format!(
                        "Texture atlas: {} | {} namespaced PNG tiles | {} animated sprites | {} model/blockstate JSONs | {} warnings; CPU/GPU in sync.",
                        atlas,bank.count(),bank.animation_count(),bank.model_count(),bank.warnings.len())));
                if let Some((x, y, z)) = app.selected_surface {
                    if let Some(v) = view.voxels.iter().find(|v| (v.x, v.y, v.z) == (x, y, z)) {
                        body=body.push(text(format!(
                                "Selected original state {}: {} resource-model cuboids (if supported); no edits applied.",
                                v.state.name,bank.geometry(&v.state).map_or_else(
                                    ||"fallback".to_string(),|boxes|boxes.len().to_string()))));
                    }
                }
            }
            body = body.push(text(format!(
                "{} · {}",
                crate::navigation_bindings::hint(app, app.active_view()),
                if app.active_view() == view_navigation::Target::Perspective {
                    crate::keyboard_bindings::flight_hint(app)
                } else {
                    l.pick(
                        "左键选点或拖动框选。",
                        "Left click to pick or drag a selection.",
                    )
                    .into()
                }
            )));
            if let Some((hx, hy, hz)) = app.surface_hover {
                if let Some(voxel) = view.voxels.iter().find(|v| (v.x, v.y, v.z) == (hx, hy, hz)) {
                    body=body.push(text(format!(
                            "Hover XYZ ({hx}, {hy}, {hz}) / chunk ({},{}) / section Y {} — {} | BE={} | staged={}",
                            hx.div_euclid(16),hz.div_euclid(16),hy.div_euclid(16),
                            voxel.state.name,voxel.original_block_entity,voxel.staged
                        )));
                }
            }
            body = body.push(
                row![
                    button(l.pick("视角左转 15°", "Orbit -15°")).on_press_maybe(
                        (app.active_view() == view_navigation::Target::Isometric)
                            .then_some(Message::OrbitSurface(-15))
                    ),
                    button(l.pick("视角右转 15°", "Orbit +15°")).on_press_maybe(
                        (app.active_view() == view_navigation::Target::Isometric)
                            .then_some(Message::OrbitSurface(15))
                    ),
                    button(l.pick("向下看", "Look lower")).on_press_maybe(
                        (app.active_view() == view_navigation::Target::Isometric)
                            .then_some(Message::PitchSurface(-10))
                    ),
                    button(l.pick("向上看", "Look higher")).on_press_maybe(
                        (app.active_view() == view_navigation::Target::Isometric)
                            .then_some(Message::PitchSurface(10))
                    ),
                    text(format!(
                        "等轴 yaw {}° / pitch {}° / zoom {:.2}x",
                        (app.surface_camera as i32 * 90 + app.surface_yaw).rem_euclid(360),
                        app.surface_pitch,
                        app.surface_navigation.zoom
                    )),
                ]
                .spacing(5)
                .wrap(),
            );
        }
    }
    body
}

pub(crate) fn object_batch<'a>(
    app: &'a App,
    _world: &'a WorldSummary,
    mut body: Column<'a, Message>,
) -> Column<'a, Message> {
    let l = app.language;

    body = body.push(text(l.pick("批量 NBT 与标量工具", "Batch NBT & scalar tools")).size(22));
    body = body.push(text(l.pick(
        "默认只读，预览与暂存操作不会直接写入世界。",
        "Read-only by default. Previews and staged changes never write world files.",
    )));
    body = body.push(
        row![
            text_input(
                l.pick(
                    "现有 NBT 字段（不可修改 id/x/y/z）",
                    "Existing Block Entity scalar field (not id/x/y/z)"
                ),
                &app.be_scalar_field
            )
            .on_input(Message::BeScalarField)
            .width(Length::Fill),
            text_input(
                l.pick("替换值（保持原类型）", "Exact replacement scalar value"),
                &app.be_scalar_value
            )
            .on_input(Message::BeScalarValue)
            .width(Length::Fill),
            button(l.pick("预览 NBT 字段", "Preview BE scalar")).on_press(Message::PreviewBeScalar),
            button(l.pick("暂存 NBT 修改", "Stage BE in memory")).on_press(Message::StageBeScalar),
        ]
        .spacing(8),
    );
    body = body.push(
        row![
            button(l.pick("撤销 NBT", "Undo BE")).on_press(Message::UndoBeScalar),
            button(l.pick("重做 NBT", "Redo BE")).on_press(Message::RedoBeScalar),
            button(l.pick("检查已暂存 NBT", "Inspect staged BE"))
                .on_press(Message::InspectStagedBe),
            button(l.pick("浏览原始 NBT 树", "Inspect original nested NBT tree"))
                .on_press(Message::InspectBeTree),
            button(l.pick("校验 NBT 区域（内存）", "Verify BE Region (memory only)"))
                .on_press(Message::PreviewBeRegion),
        ]
        .spacing(8),
    );
    body=body.push(text(l.pick("预检当前维度的全部暂存修改，包括实体与方块实体；旧数字格式在保存页选择光照属性。关闭游戏后按实际预览确认保存。", "Preview all staged changes in this dimension, including entities and block entities. Configure legacy lighting on the Save page and close Minecraft before confirming.")));
    body = body.push(
        row![
            button(l.pick("预检 NBT 保存", "Preview NBT save"))
                .on_press_maybe((!app.save_worker).then_some(Message::PrepareBeSave)),
            button(l.pick("确认保存 NBT", "Confirm NBT save")).on_press(Message::ConfirmBeSave),
        ]
        .spacing(8),
    );
    body = body.push(
        row![
            text_input(
                l.pick(
                    "批量 NBT 修改配置（JSON）",
                    "Declarative BE scalar .mcedit2beops.json"
                ),
                &app.be_batch_path
            )
            .on_input(Message::BeBatchPath)
            .width(Length::Fill),
            button(l.pick("浏览…", "Browse…"))
                .on_press(Message::ChooseAdvancedFile(AdvancedFile::BeBatch, false)),
            button(l.pick("预览批量 NBT", "Preview whole BE batch"))
                .on_press(Message::PreviewBeBatch),
            button(l.pick("暂存全部 NBT", "Stage all BE edits")).on_press(Message::StageBeBatch),
        ]
        .spacing(8),
    );
    body = body.push(
        row![
            text_input(
                l.pick(
                    "世界外部的 NBT 编辑日志路径",
                    "Outside-world BE edit .mcedit2bejournal.json"
                ),
                &app.be_journal_path
            )
            .on_input(Message::BeJournalPath)
            .width(Length::Fill),
            button(l.pick("浏览…", "Browse…"))
                .on_press(Message::ChooseAdvancedFile(AdvancedFile::BeJournal, false)),
            button(l.pick("另存为…", "Save as…"))
                .on_press(Message::ChooseAdvancedFile(AdvancedFile::BeJournal, true)),
            button(l.pick("导出 NBT 编辑检查点", "Checkpoint staged BE edits"))
                .on_press(Message::ExportBeJournal),
            button(l.pick("导入 NBT 编辑（内存）", "Restore BE edits (memory)"))
                .on_press(Message::ImportBeJournal),
        ]
        .spacing(8),
    );
    body
}

pub(crate) fn object_snapshots<'a>(
    app: &'a App,
    mut body: Column<'a, Message>,
) -> Column<'a, Message> {
    let l = app.language;
    body = body.push(
        row![
            text_input(
                l.pick(
                    "原始实体快照路径",
                    "External .mcedit2entities original source snapshot"
                ),
                &app.entity_snapshot_path
            )
            .on_input(Message::EntitySnapshotPath)
            .width(Length::Fill),
            button(l.pick("浏览…", "Browse…")).on_press(Message::ChooseAdvancedFile(
                AdvancedFile::EntitySnapshot,
                false
            )),
            button(l.pick("另存为…", "Save as…")).on_press(Message::ChooseAdvancedFile(
                AdvancedFile::EntitySnapshot,
                true
            )),
            button(l.pick("查看原始实体", "Inspect original entities"))
                .on_press(Message::InspectEntities),
            button(l.pick("导出实体 NBT", "Export entity NBT")).on_press(Message::ExportEntities),
            button(l.pick("验证实体来源", "Verify entity source"))
                .on_press(Message::VerifyEntities),
        ]
        .spacing(8),
    );
    body = body.push(
        row![
            text_input(
                l.pick(
                    "原始方块实体 NBT 快照路径",
                    "Outside-world .mcedit2be original NBT snapshot"
                ),
                &app.be_snapshot_path
            )
            .on_input(Message::BeSnapshotPath)
            .width(Length::Fill),
            button(l.pick("浏览…", "Browse…"))
                .on_press(Message::ChooseAdvancedFile(AdvancedFile::BeSnapshot, false)),
            button(l.pick("另存为…", "Save as…"))
                .on_press(Message::ChooseAdvancedFile(AdvancedFile::BeSnapshot, true)),
            button(l.pick("导出原始 NBT 快照", "Export exact original BE NBT"))
                .on_press(Message::ExportOriginalBe),
            button(l.pick("验证 NBT 未变更", "Verify BE source unchanged"))
                .on_press(Message::VerifyOriginalBe),
        ]
        .spacing(8),
    );

    body
}

/// 切片的实际来源输入与报告归工作区显示选项，画布独立占满可用区域。
pub(crate) fn slice_controls(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let mut body = column![].spacing(8).width(Length::Fill);
    body = body.push(text(l.pick("二维与纵向切片", "2D and vertical source slices")).size(22));
    body = body.push(
        row![
            text_input(
                l.pick(
                    "二维 X/Z 地图：X1,Y,Z1; X2,Y,Z2",
                    "2D X/Z map: X1,Y,Z1; X2,Y,Z2"
                ),
                &app.slice_area
            )
            .on_input(Message::SliceText)
            .width(Length::Fill),
            button(l.pick("查看二维切片", "Inspect 2D slice")).on_press(Message::InspectSlice)
        ]
        .spacing(8)
        .wrap(),
    );
    body = body.push(
        row![
            button("← 16").on_press(Message::MoveSlice(-16, 0, 0)),
            button("16 →").on_press(Message::MoveSlice(16, 0, 0)),
            button("↑ Z").on_press(Message::MoveSlice(0, 0, -16)),
            button("↓ Z").on_press(Message::MoveSlice(0, 0, 16)),
            button("Y + 1").on_press(Message::MoveSlice(0, 1, 0)),
            button("Y - 1").on_press(Message::MoveSlice(0, -1, 0)),
        ]
        .spacing(5)
        .wrap(),
    );
    body = body.push(
        row![
            text_input(
                l.pick(
                    "纵向 X/Y 或 Z/Y 切片：X1,Y1,Z1; X2,Y2,Z2",
                    "Vertical X/Y or Z/Y plane: X1,Y1,Z1; X2,Y2,Z2"
                ),
                &app.vertical_area
            )
            .on_input(Message::VerticalText)
            .width(Length::Fill),
            button(l.pick("查看纵向切片", "Inspect vertical plane"))
                .on_press(Message::InspectVertical)
        ]
        .spacing(8)
        .wrap(),
    );

    body.into()
}

/// 每个切片保留自己的真实源、相机和缓存，同时呈现在有限高的工作区。
pub(crate) fn slices(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let lighting = &crate::lighting::Table::unlit();
    let mut horizontal = column![text(l.pick("水平切片", "Horizontal slice")).size(17)]
        .spacing(6)
        .width(Length::Fill)
        .height(Length::Fill);
    let mut vertical = column![text(l.pick("纵向切片", "Vertical slice")).size(17)]
        .spacing(6)
        .width(Length::Fill)
        .height(Length::Fill);
    if let Some((world_index, dimension_index, slice)) = app.slice_grid.as_ref() {
        if app.current_key() == Some((*world_index, *dimension_index)) {
            horizontal=horizontal.push(text(
                    l.pick("灰：空气 · 紫：缺失区块 · 橙斜线：缺失区段 · 金：方块实体", "Gray: air · Purple: absent chunk · Amber hatch: absent section · Gold: block entity")
                ));
            horizontal = horizontal
                .push(view_navigation::controls(view_navigation::Target::Horizontal).wrap());
            horizontal = horizontal.push(crate::world_ruler::plane(
                crate::navigation_bindings::wrap(
                    app,
                    view_navigation::Target::Horizontal,
                    brush_cursor::overlay(
                        Map2D {
                            selection_handle: crate::selection_handle::config(app),
                            chunk_column: object_inspector::selected_column(app),
                            brush: brush_cursor::with_assets(
                                brush_stroke::config(app),
                                app.texture_bank.clone(),
                                app.show_textures,
                            ),
                            brush_key: app.current_key(),
                            brush_source: Some(brush_stroke::Source::Horizontal(
                                std::sync::Arc::clone(slice),
                                view_navigation::Target::Horizontal,
                            )),
                            layers: app.scene_layers.view_for(
                                app.current_key(),
                                scene_layers::SourceRef::Horizontal(slice),
                            ),
                            plane: Plane::Horizontal(slice),
                            navigation: app.slice_navigation,
                            navigation_target: view_navigation::Target::Horizontal,
                            selected: parse_xyz(&app.query_coordinates).ok(),
                            texture_bank: app.texture_bank.as_deref(),
                            show_textures: app.show_textures,
                            block_layers: app.entity_models.visibility.block_layers(),
                            entity_frame: app
                                .entity_models
                                .frame
                                .as_ref()
                                .filter(|frame| {
                                    frame.displays(
                                        app.entity_models.enabled,
                                        app.entity_models.visibility,
                                    ) && frame.show_textures == app.show_textures
                                        && Some(frame.snapshot.key) == app.current_key()
                                        && app.scene_layers.snapshot.as_ref().is_some_and(|s| {
                                            std::sync::Arc::ptr_eq(s, &frame.snapshot)
                                        })
                                        && app
                                            .texture_bank
                                            .as_ref()
                                            .and_then(|b| b.atlas())
                                            .is_some_and(|atlas| {
                                                textures::atlas_compatible(&frame.atlas, atlas)
                                            })
                                })
                                .cloned(),
                            grid_cache: std::sync::Arc::clone(&app.grid_horizontal),
                            lighting: std::sync::Arc::clone(lighting),
                            quality: app.gpu_quality,
                            source_revision: app.grid_revision,
                        }
                        .element(app, Length::Fill),
                        brush_stroke::config(app),
                        view_navigation::Target::Horizontal,
                        Length::Fill,
                    ),
                ),
                Plane::Horizontal(slice),
                app.slice_navigation,
                Length::Fill,
                app.language,
                app.minimap
                    .capture(app.current_key(), view_navigation::Target::Horizontal),
                1.,
            ));
        }
    }
    if let Some((wi, di, plane)) = app.vertical_grid.as_ref() {
        if app.current_key() == Some((*wi, *di)) {
            vertical=vertical.push(text(
                    l.pick("上方为高处 · 金：方块实体 · 橙：缺失区段 · 紫：缺失区块", "Higher Y at top · Gold: block entity · Amber: absent section · Purple: absent chunk")
                ));
            vertical =
                vertical.push(view_navigation::controls(view_navigation::Target::Vertical).wrap());
            vertical = vertical.push(crate::world_ruler::plane(
                crate::navigation_bindings::wrap(
                    app,
                    view_navigation::Target::Vertical,
                    brush_cursor::overlay(
                        Map2D {
                            selection_handle: crate::selection_handle::config(app),
                            chunk_column: object_inspector::selected_column(app),
                            brush: brush_cursor::with_assets(
                                brush_stroke::config(app),
                                app.texture_bank.clone(),
                                app.show_textures,
                            ),
                            brush_key: app.current_key(),
                            brush_source: Some(brush_stroke::Source::Vertical(
                                std::sync::Arc::clone(plane),
                                view_navigation::Target::Vertical,
                            )),
                            layers: app.scene_layers.view_for(
                                app.current_key(),
                                scene_layers::SourceRef::Vertical(plane),
                            ),
                            plane: Plane::Vertical(plane),
                            navigation: app.vertical_navigation,
                            navigation_target: view_navigation::Target::Vertical,
                            selected: parse_xyz(&app.query_coordinates).ok(),
                            texture_bank: app.texture_bank.as_deref(),
                            show_textures: app.show_textures,
                            block_layers: app.entity_models.visibility.block_layers(),
                            entity_frame: app
                                .entity_models
                                .frame
                                .as_ref()
                                .filter(|frame| {
                                    frame.displays(
                                        app.entity_models.enabled,
                                        app.entity_models.visibility,
                                    ) && frame.show_textures == app.show_textures
                                        && Some(frame.snapshot.key) == app.current_key()
                                        && app.scene_layers.snapshot.as_ref().is_some_and(|s| {
                                            std::sync::Arc::ptr_eq(s, &frame.snapshot)
                                        })
                                        && app
                                            .texture_bank
                                            .as_ref()
                                            .and_then(|b| b.atlas())
                                            .is_some_and(|atlas| {
                                                textures::atlas_compatible(&frame.atlas, atlas)
                                            })
                                })
                                .cloned(),
                            grid_cache: std::sync::Arc::clone(&app.grid_vertical),
                            lighting: std::sync::Arc::clone(lighting),
                            quality: app.gpu_quality,
                            source_revision: app.grid_revision,
                        }
                        .element(app, Length::Fill),
                        brush_stroke::config(app),
                        view_navigation::Target::Vertical,
                        Length::Fill,
                    ),
                ),
                Plane::Vertical(plane),
                app.vertical_navigation,
                Length::Fill,
                app.language,
                app.minimap
                    .capture(app.current_key(), view_navigation::Target::Vertical),
                1.,
            ));
        }
    }

    if !app
        .slice_grid
        .as_ref()
        .is_some_and(|(wi, di, _)| app.current_key() == Some((*wi, *di)))
    {
        horizontal = horizontal.push(
            container(
                button(l.pick("读取水平切片", "Read horizontal slice"))
                    .on_press(Message::InspectSlice),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        );
    }
    if !app
        .vertical_grid
        .as_ref()
        .is_some_and(|(wi, di, _)| app.current_key() == Some((*wi, *di)))
    {
        vertical = vertical.push(
            container(
                button(l.pick("读取纵向切片", "Read vertical slice"))
                    .on_press(Message::InspectVertical),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill),
        );
    }
    row![horizontal, vertical]
        .spacing(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// 三维场景不进入选项页的滚动区域，保持原来的真实源和完整渲染链。
pub(crate) fn viewer(app: &App) -> Element<'_, Message> {
    if let Some((wi, di, view)) = &app.surface {
        if app.current_key() == Some((*wi, *di)) {
            return render_surface(app, view, Length::Fill);
        }
    }
    container(
        button(app.language.pick("读取三维场景", "Read 3D scene")).on_press(Message::InspectVolume),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}
pub(crate) fn editing<'a>(
    app: &'a App,
    _world: &'a WorldSummary,
    mut body: Column<'a, Message>,
) -> Column<'a, Message> {
    let l = app.language;

    body = body.push(text(l.pick("方块编辑与操作预览", "Block editing and previews")).size(22));
    body = body.push(text(l.pick(
        "先预览，再暂存；可撤销或重做。",
        "Preview before staging; Undo and Redo are available.",
    )));
    let editing_inputs = column![
        text(l.pick(
            "3D 方块编辑：预览、暂存、撤销/重做（尚未写盘）",
            "3D block edit overlay — preview, stage and Undo/Redo (NOT SAVED)"
        ))
        .size(19),
        crate::selection_edit::view(app),
        crate::deletion_ui::view(app),
        row![
            text_input(
                l.pick(
                    "目标方块 minecraft:stone 或 numeric:5[data=2]",
                    "Target minecraft:stone or numeric:5[data=2]"
                ),
                &app.edit_target
            )
            .on_input(Message::TargetText)
            .width(Length::Fill),
            crate::block_picker::choose(crate::block_picker::Target::WorldTarget),
            text_input(
                l.pick(
                    "原始方块过滤条件（留空表示全部）",
                    "Optional original state filter (blank means all)"
                ),
                &app.edit_filter
            )
            .on_input(Message::FilterText)
            .width(Length::Fill),
            crate::block_picker::choose(crate::block_picker::Target::WorldFilter)
        ]
        .spacing(8),
    ]
    .spacing(8);
    body = body.push(editing_inputs);
    body = body.push(
        row![
            crate::transfer_edit::position_view(app),
            button(l.pick("预览完整克隆", "Preview complete clone"))
                .on_press(Message::PreviewClone),
            button(l.pick("预览完整移动", "Preview complete move")).on_press(Message::PreviewMove)
        ]
        .spacing(8),
    );
    body = body.push(crate::flood_fill::view(app));
    body = body.push(
        row![
            button(l.pick("空心盒", "Hollow box"))
                .on_press(Message::PreviewBrush(BrushShape::HollowBox)),
            button(l.pick("椭球", "Ellipsoid"))
                .on_press(Message::PreviewBrush(BrushShape::Ellipsoid)),
            button(l.pick("椭球外壳", "Ellipsoid shell"))
                .on_press(Message::PreviewBrush(BrushShape::EllipsoidShell)),
            button(l.pick("Y 轴圆柱", "Cylinder Y"))
                .on_press(Message::PreviewBrush(BrushShape::CylinderY)),
            button(l.pick("Y 轴圆柱壳", "Cylinder shell Y"))
                .on_press(Message::PreviewBrush(BrushShape::CylinderShellY)),
            button("菱形").on_press(Message::PreviewBrush(BrushShape::Diamond)),
            button("抛物面圆顶").on_press(Message::PreviewBrush(BrushShape::ParabolicDome)),
        ]
        .spacing(6),
    );
    body = body.push(transfer_edit::view(&app.transfer).map(Message::Transfer));
    body = body.push(block_edit::biome_view(app));
    body = body.push(brush_stroke::view(app));
    body = body.push(block_edit::view(app));
    let apply = if (app.prepared.is_some()
        || app.block_edit.has_prepared()
        || app.transfer.preview.is_some())
        && !app.readonly
        && !app.transfer.busy
        && !app.block_edit.busy
    {
        button(l.pick("暂存已审核修改", "Stage approved preview (memory)"))
            .on_press(Message::ApplyEdit)
    } else {
        button(l.pick("暂存已审核修改", "Stage approved preview (memory)"))
    };
    body = body.push(
        row![
            button(l.pick("预览完整 3D 修改", "Preview full 3D edit"))
                .on_press(Message::PreviewEdit),
            apply,
            button(l.pick("撤销", "Undo")).on_press(Message::UndoEdit),
            button(l.pick("重做", "Redo")).on_press(Message::RedoEdit),
            button(l.pick("放弃修改", "Discard edits")).on_press(Message::DiscardEdits)
        ]
        .spacing(8),
    );
    body = body.push(
        row![
            button(l.pick("分析无损 NBT 修改", "Analyze lossless native NBT preview"))
                .on_press(Message::AnalyzeNativePatch),
            button(l.pick(
                "分析 Region 追加保存计划",
                "Analyze append-only Region save plan"
            ))
            .on_press(Message::AnalyzeRegionPlan)
        ]
        .spacing(8),
    );

    body
}
