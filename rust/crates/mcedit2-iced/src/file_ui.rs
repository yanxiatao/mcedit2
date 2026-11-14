//! 按任务分离对象、独立文档、结构、插件与保存界面。
use crate::*;
use iced::widget::column;
#[path = "document_workspace.rs"]
mod documents;
/// 文件尚在读取或读取失败时仍保留当前文档身份，命令不能转向后台世界。
pub(crate) fn active_document_id(app: &App) -> Option<u64> {
    if app.desktop.show_recent
        || app.structures.active.is_some()
        || !app.show_advanced
        || !matches!(
            app.advanced_section,
            AdvancedSection::Files | AdvancedSection::Players | AdvancedSection::WorldInfo
        )
    {
        return None;
    }
    app.nbt_documents.active(app.current?).map(|(id, _)| id)
}
/// 文档状态保留在应用中，只有所属页面和世界可以显示它。
pub(crate) fn document(app: &App) -> Option<&nbt_editor::Panel> {
    let panel = app.nbt_document()?;
    match app.advanced_section {
        AdvancedSection::Files => Some(panel),
        AdvancedSection::WorldInfo if panel.world_file() => Some(panel),
        AdvancedSection::Players if panel.player_prefix().is_some() => Some(panel),
        _ => None,
    }
}
pub(crate) fn view<'a>(app: &'a App, world: &'a WorldSummary) -> Element<'a, Message> {
    match app.advanced_section {
        AdvancedSection::MapsEditor => app.map_editor.view(&app.drag_drop),
        AdvancedSection::Players | AdvancedSection::WorldInfo | AdvancedSection::Files => {
            documents::view(app)
        }
        AdvancedSection::Chunks => object_inspector::view(app).map(Message::ObjectInspector),
        AdvancedSection::BlockEntities => objects(app, world),
        AdvancedSection::Structures => structure_exchange::view(app),
        AdvancedSection::Plugins => plugins(app),
        AdvancedSection::Save => save(app, world),
        _ => column![].into(),
    }
}
fn objects<'a>(app: &'a App, world: &'a WorldSummary) -> Element<'a, Message> {
    use object_inspector::WorkspacePage;
    let l = app.language;
    let page = app.object_inspector.workspace_page;
    let mut tabs = row![].spacing(5);
    for (target, zh, en) in [
        (WorkspacePage::Object, "对象详情", "Object"),
        (WorkspacePage::Block, "方块检查", "Block"),
        (WorkspacePage::Entities, "实体操作", "Entity actions"),
        (WorkspacePage::Ray, "射线实体", "Entities on ray"),
        (WorkspacePage::Batch, "批量 NBT", "Batch NBT"),
        (WorkspacePage::Snapshots, "快照与校验", "Snapshots"),
    ] {
        tabs = tabs.push(
            button(l.pick(zh, en))
                .style(if page == target {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press(Message::ObjectInspector(
                    object_inspector::Action::WorkspacePage(target),
                )),
        );
    }
    let mut body = column![].spacing(10).width(Length::Fill);
    match page {
        WorkspacePage::Block => {
            body = body.push(
                object_inspector::cell::view(app)
                    .map(|action| Message::ObjectInspector(object_inspector::Action::Cell(action))),
            )
        }
        WorkspacePage::Object => {
            body = body.push(object_inspector::view(app).map(Message::ObjectInspector))
        }
        WorkspacePage::Ray => {
            body = body.push(
                object_inspector::ray::view(app)
                    .map(|action| Message::ObjectInspector(object_inspector::Action::Ray(action))),
            )
        }
        WorkspacePage::Batch => {
            body = advanced_pages::object_batch(app, world, body);
            body = body.push(text(
                "方块实体：选择位置图层中的方块实体，输入类型化 NBT 操作 JSON，预检后确认暂存。",
            ));
            body = body.push(
                text_input("NBT 操作 JSON", &app.be_tree_operation)
                    .on_input(Message::BeTreeOperation),
            );
            body = body.push(
                row![
                    button("预检方块实体 NBT").on_press(Message::BeTreePrepare),
                    button("确认暂存方块实体").on_press_maybe(
                        app.be_tree_preview
                            .is_some()
                            .then_some(Message::BeTreeApply)
                    )
                ]
                .spacing(6)
                .wrap(),
            );
            body = body.push(text(&app.be_tree_status));
        }
        WorkspacePage::Entities => {
            if let Some(source) = app.scene_layers.detail(l) {
                body = body.push(text(source));
            }
            body = body.push(text(
                "实体修改：先选择已显示实体；位移为整数 x,y,z，预检后确认暂存。",
            ));
            body = body.push(
                text_input("实体位移 x,y,z", &app.entity_edit_delta).on_input(Message::EntityDelta),
            );
            body = body.push(
                row![
                    button("预检删除").on_press(Message::EntityPrepareDelete),
                    button("预检位移").on_press(Message::EntityPrepareTranslate),
                    button("预检克隆").on_press(Message::EntityPrepareClone),
                    button("确认暂存实体").on_press_maybe(
                        app.entity_edit_preview
                            .is_some()
                            .then_some(Message::EntityApplyPreview)
                    )
                ]
                .spacing(6)
                .wrap(),
            );
            body = body.push(text(&app.entity_edit_status));
        }
        WorkspacePage::Snapshots => body = advanced_pages::object_snapshots(app, body),
    }
    body = body.push(block_edit::budget_view(app));
    if page != WorkspacePage::Object && !app.edit_report.is_empty() {
        body = body.push(text(&app.edit_report));
    }
    let content = iced::widget::keyed_column([(
        (page, app.current_key()),
        scrollable(body)
            .id(workspace_ui::OBJECTS_SCROLL)
            .height(Length::Fill)
            .width(Length::Fill)
            .into(),
    )])
    .height(Length::Fill)
    .width(Length::Fill);
    column![
        text(l.pick("NBT 与实体", "NBT & entities")).size(24),
        tabs.wrap(),
        content
    ]
    .spacing(10)
    .height(Length::Fill)
    .width(Length::Fill)
    .into()
}
fn plugins(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let mut body = column![].spacing(12).width(Length::Fill);
    if app.advanced_section == AdvancedSection::Plugins {
        body = body
            .push(plugin_ui::view(
                &app.plugins,
                l,
                app.prepared.is_some(),
                app.readonly,
                &app.edit_area,
                false,
                crate::keyboard_bindings::owner(app),
            ))
            .push(text(l.pick("计划、插件与编辑日志", "Plans, plugins and journals")).size(18));
        body = body.push(
            row![
                text_input(
                    l.pick(
                        "声明式插件 JSON 路径",
                        "Declarative .mcedit2plugin.json file"
                    ),
                    &app.plugin_manifest_path
                )
                .on_input(Message::PluginManifestPath)
                .width(Length::Fill),
                button(l.pick("浏览…", "Browse…")).on_press(Message::ChooseAdvancedFile(
                    AdvancedFile::PluginManifest,
                    false
                )),
                button(l.pick("预览安全插件替换", "Preview safe plugin replacement"))
                    .on_press(Message::PreviewPlugin),
            ]
            .spacing(8)
            .wrap(),
        );
    }
    body.into()
}
fn save<'a>(app: &'a App, world: &'a WorldSummary) -> Element<'a, Message> {
    let l = app.language;
    let mut body = column![].spacing(12).width(Length::Fill);
    if app.advanced_section == AdvancedSection::Save {
        let joint = crate::mixed_save::needs_joint(app, &world.root);
        let grouped = joint
            || !app.nbt_group.is_empty()
            || app.nbt_group_key.is_some()
            || app.mixed_key.is_some();
        if app.readonly {
            body = body.push(text(l.pick(
                "当前存档以只读方式打开，不能保存修改。",
                "This world is open read-only; changes cannot be saved.",
            )));
        }
        if !app.readonly {
            if joint {
                body = body.push(text("此存档包含多个保存来源，请使用下方联合保存。").size(13));
            }
            body=body.push(text(l.pick("保存前请关闭 Minecraft。先检查修改内容，再确认保存并保留备份。", "Close Minecraft before saving. Review the changes, then confirm Save and keep backups.")).size(13));
            body = body.push(
                button(l.pick("1  检查当前维度修改", "1  Review changes in this dimension"))
                    .on_press_maybe(
                        (!app.save_worker && !joint).then_some(Message::PrepareWorldSave),
                    ),
            );
            if let Some(dimension) = world.dimensions.get(app.dimension_index) {
                body = body.push(app.save_postprocess.view(&world.root, &dimension.name));
            }
            if app.save_planning {
                body = body.push(button("取消保存预检").on_press(Message::CancelSavePlanning));
            }
            if let Some(intent) = app.save_preview.as_ref() {
                for summary in intent.candidates.summaries() {
                    body = body.push(text(summary));
                }
                let confirm = if intent.be_mode {
                    Message::ConfirmBeSave
                } else {
                    Message::ConfirmWorldSave
                };
                body = body.push(
                    button(l.pick("确认保存并保留备份", "Save and keep backups"))
                        .style(button::primary)
                        .on_press_maybe((!joint && !app.readonly).then_some(confirm)),
                );
            }
        }
        if grouped {
            body =
                body.push(text(l.pick("联合保存与 NBT 文件", "Joint save & NBT files")).size(18));
            body = save_groups(app, world, body);
        }
        body = body.push(
            button(l.pick(
                if app.workspace.presentation.save_details {
                    "收起保存工具与事务恢复"
                } else {
                    "更多保存工具与事务恢复…"
                },
                if app.workspace.presentation.save_details {
                    "Hide save tools & recovery"
                } else {
                    "More save tools & recovery…"
                },
            ))
            .style(button::secondary)
            .on_press(Message::WorkspacePresentation(
                crate::workspace_ui::presentation::Action::SaveDetails,
            )),
        );
        if app.workspace.presentation.save_details {
            if !grouped {
                body = save_groups(app, world, body);
            }
            body = body
                .push(text(l.pick("保存计划与编辑日志", "Save plans & edit journals")).size(18));
            body = body.push(
                row![
                    text_input(
                        l.pick(
                            "Region 保存计划 JSON 路径",
                            "External .mcedit2regionplan.json"
                        ),
                        &app.region_batch_path
                    )
                    .on_input(Message::RegionBatchPath)
                    .width(Length::Fill),
                    button(l.pick("浏览…", "Browse…"))
                        .on_press(Message::ChooseAdvancedFile(AdvancedFile::RegionPlan, false)),
                    button(l.pick("另存为…", "Save as…"))
                        .on_press(Message::ChooseAdvancedFile(AdvancedFile::RegionPlan, true)),
                    button(l.pick("预检全部 Region", "Preflight all Regions"))
                        .on_press(Message::PreviewRegionBatch),
                    button(l.pick("导出 Region 计划", "Export Region plan"))
                        .on_press(Message::ExportRegionBatch),
                    button(l.pick("验证保存计划", "Verify plan"))
                        .on_press(Message::VerifyRegionBatch),
                ]
                .spacing(8)
                .wrap(),
            );
            body = body.push(
                row![
                    text_input(
                        l.pick(
                            "外部编辑日志 .mcedit2patch 路径",
                            "External .mcedit2patch path (outside the world)"
                        ),
                        &app.patch_path
                    )
                    .on_input(Message::PatchPath)
                    .width(Length::Fill),
                    button(l.pick("浏览…", "Browse…"))
                        .on_press(Message::ChooseAdvancedFile(AdvancedFile::Patch, false)),
                    button(l.pick("另存为…", "Save as…"))
                        .on_press(Message::ChooseAdvancedFile(AdvancedFile::Patch, true)),
                    button(l.pick("导出编辑日志", "Export edit journal"))
                        .on_press(Message::ExportPatch),
                    if !app.readonly {
                        button(l.pick("导入已验证日志", "Import verified journal"))
                            .on_press(Message::ImportPatch)
                    } else {
                        button(l.pick("导入已验证日志", "Import verified journal"))
                    }
                ]
                .spacing(8)
                .wrap(),
            );
            if !app.readonly {
                body = body
                    .push(text(l.pick("恢复未完成的保存", "Recover an interrupted save")).size(18));
                body=body.push(text(l.pick("事务恢复：填写待处理 Region 文件名（如 r.-1.0.mca），再输入 FINALIZE 或 ROLLBACK。不会删除备份。", "Transaction recovery: for a pending Region, enter its exact filename (e.g. r.-1.0.mca) and type FINALIZE or ROLLBACK. Backups are retained.")));
                body = body.push(
                    row![
                        text_input(
                            l.pick(
                                "待恢复文件 r.X.Z.mca / r.X.Z.mcr",
                                "Pending r.X.Z.mca or r.X.Z.mcr"
                            ),
                            &app.recover_region
                        )
                        .on_input(Message::RecoverRegion)
                        .width(Length::Fill),
                        text_input(
                            l.pick("输入 FINALIZE 或 ROLLBACK", "FINALIZE or ROLLBACK"),
                            &app.recovery_phrase
                        )
                        .on_input(Message::RecoveryPhrase)
                        .width(Length::Fill),
                    ]
                    .spacing(8)
                    .wrap(),
                );
                body = body.push(
                    row![
                        if app.recovery_phrase == "FINALIZE" {
                            button(l.pick("完成 Region 事务", "Finalize verified Region"))
                                .on_press(Message::FinalizeRegion)
                        } else {
                            button(l.pick("完成 Region 事务", "Finalize verified Region"))
                        },
                        if app.recovery_phrase == "ROLLBACK" {
                            button(l.pick("恢复原始 Region", "Restore original Region"))
                                .on_press(Message::RollbackRegion)
                        } else {
                            button(l.pick("恢复原始 Region", "Restore original Region"))
                        }
                    ]
                    .spacing(8)
                    .wrap(),
                );
                body = body.push(text(l.pick("多文件保存组：在上方确认栏输入 FINALIZE_GROUP 或 ROLLBACK_GROUP，无需逐个填写 Region。", "Multi-file group: type FINALIZE_GROUP or ROLLBACK_GROUP above; no Region filename is needed.")));
                body = body.push(
                    row![
                        button(l.pick("完成整个保存组", "Finalize whole group")).on_press_maybe(
                            (app.recovery_phrase == "FINALIZE_GROUP")
                                .then_some(Message::FinalizeRegionGroup)
                        ),
                        button(l.pick("回滚整个保存组", "Roll back whole group")).on_press_maybe(
                            (app.recovery_phrase == "ROLLBACK_GROUP")
                                .then_some(Message::RollbackRegionGroup)
                        ),
                    ]
                    .spacing(8)
                    .wrap(),
                );
            }
        }
    }
    body.into()
}

/// 普通保存与展开的高级工具复用同一组文件候选及联合保存操作。
fn save_groups<'a>(
    app: &'a App,
    world: &'a WorldSummary,
    mut body: iced::widget::Column<'a, Message>,
) -> iced::widget::Column<'a, Message> {
    body = body.push(text(format!("NBT 保存组：{} 个候选", app.nbt_group.len())));
    for candidate in &app.nbt_group {
        body = body.push(
            text(format!(
                "{} · {} → {}",
                candidate.path().display(),
                candidate.source_sha(),
                candidate.candidate_sha()
            ))
            .size(12),
        );
    }
    body = body.push(
        row![
            button("准备保存组").on_press_maybe(
                (!app.nbt_group_busy && !app.nbt_group.is_empty())
                    .then_some(Message::NbtGroupPrepare)
            ),
            button("确认提交组")
                .on_press_maybe((!app.nbt_group_busy).then_some(Message::NbtGroupFinish)),
            button("回滚保存组")
                .on_press_maybe((!app.nbt_group_busy).then_some(Message::NbtGroupRollback))
        ]
        .spacing(6)
        .wrap(),
    );
    body = body.push(text(&app.nbt_group_status));
    let dimensions = app
        .sessions
        .values()
        .filter(|session| {
            session.original_world().root == world.root && session.has_pending_changes()
        })
        .map(|session| session.dimension())
        .collect::<Vec<_>>();
    body = body.push(text(format!(
        "联合保存：{} 个已修改维度、{} 个已加入的 NBT 文件、{} 项地图更改。",
        dimensions.len(),
        app.nbt_group.len(),
        app.map_editor.change_count(&world.root)
    )));
    if !dimensions.is_empty() {
        body = body.push(text(dimensions.join("、")));
    }
    body=body.push(text("联合保存包含此存档所有已修改维度，分别采用各维度的高度 / 光照设置；当前 Region 预览继续参与核对。普通保存通过按钮确认；区块删除在区块工作区单独确认，恢复时请核对事务方向。"));

    body = body.push(
        row![
            button("准备联合保存")
                .on_press_maybe((!app.mixed_busy).then_some(Message::MixedPrepare)),
            button("提交联合保存")
                .on_press_maybe((!app.mixed_busy).then_some(Message::MixedFinish)),
            button("回滚联合保存")
                .on_press_maybe((!app.mixed_busy).then_some(Message::MixedRollback))
        ]
        .spacing(6)
        .wrap(),
    );
    body = body.push(text(&app.mixed_status));
    body
}
