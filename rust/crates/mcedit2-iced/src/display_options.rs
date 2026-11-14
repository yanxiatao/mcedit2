//! 显示选项按操作场景分组，公共命令仍由原有消息执行。
use crate::*;
use iced::widget::{column, pick_list};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Group {
    #[default]
    Distance,
    Camera,
    Layers,
    Region,
    Diagnostics,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Choice(Group, &'static str);
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.1)
    }
}
impl Group {
    fn label(self, l: Language) -> &'static str {
        match self {
            Self::Distance => l.pick("视距与加载", "View distance & loading"),
            Self::Camera => l.pick("相机与定位", "Camera & navigation"),
            Self::Layers => l.pick("对象与图层", "Objects & layers"),
            Self::Region => l.pick("局部与远景预览", "Local & distant previews"),
            Self::Diagnostics => l.pick("加载与渲染详情", "Loading & renderer details"),
        }
    }
}
pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let selected = app.workspace.presentation.display_group;
    let choices = [
        Group::Distance,
        Group::Camera,
        Group::Layers,
        Group::Region,
        Group::Diagnostics,
    ]
    .map(|group| Choice(group, group.label(l)));
    let mut panel = column![
        text(l.pick("设置类别", "Category")).size(12),
        pick_list(
            choices,
            Some(Choice(selected, selected.label(l))),
            |choice| Message::WorkspacePresentation(
                workspace_ui::presentation::Action::DisplayGroup(choice.0)
            )
        )
        .width(Length::Fill),
    ]
    .spacing(14)
    .width(Length::Fill);
    match selected {
        Group::Distance => {
            panel = panel.push(
                iced::widget::checkbox(app.show_textures)
                    .label(l.pick("显示材质", "Show textures"))
                    .on_toggle(|_| Message::ToggleTextures),
            );
            if app.uses_world_mesh() {
                panel = panel.push(camera_distance::view(app));
                panel = panel.push(
                    iced::widget::checkbox(app.spectator_sampled)
                        .label(l.pick(
                            "远景简化（高度采样，适合看大范围轮廓）",
                            "Simplified distant terrain (height samples for wide overviews)",
                        ))
                        .on_toggle(Message::SpectatorSampled),
                );
            }
            panel = panel.push(
                button(l.pick("重新加载当前画面", "Reload scene"))
                    .style(button::secondary)
                    .on_press(Message::ReloadScene),
            );
        }
        Group::Camera => {
            panel = panel.push(spectator::projection_controls(app));
            panel = panel.push(work_plane::controls(app));
            panel = panel.push(view_state::controls(app));
            panel = panel.push(text(l.pick("精确坐标", "Exact coordinates")).size(16));
            panel = panel.push(
                text_input("X, Y, Z", &app.query_coordinates)
                    .on_input(Message::Coordinates)
                    .on_submit(Message::InspectBlock),
            );
            panel = panel.push(
                row![
                    button(l.pick("查询方块", "Inspect block")).on_press(Message::InspectBlock),
                    button(l.pick("居中", "Center")).on_press(Message::Center3DOnPicked)
                ]
                .spacing(5),
            );
            if app.spectator {
                panel = panel.push(
                    row![
                        button("←").on_press(Message::SpectatorLook(-60.0, 0.0)),
                        button("→").on_press(Message::SpectatorLook(60.0, 0.0)),
                        button(l.pick("抬头", "Look up"))
                            .on_press(Message::SpectatorLook(0.0, -60.0)),
                        button(l.pick("低头", "Look down"))
                            .on_press(Message::SpectatorLook(0.0, 60.0)),
                    ]
                    .spacing(4),
                );
            }
            if app.active_view() != view_navigation::Target::Perspective {
                panel = panel.push(view_navigation::pan_controls(app.active_view()));
            }
            panel = panel.push(
                row![
                    button("−")
                        .style(button::secondary)
                        .on_press(Message::ZoomSurface(-1)),
                    button("+")
                        .style(button::secondary)
                        .on_press(Message::ZoomSurface(1)),
                    button(l.pick("重置相机", "Reset camera"))
                        .style(button::secondary)
                        .on_press(Message::ResetSurfaceCamera)
                ]
                .spacing(8)
                .wrap(),
            );
        }
        Group::Layers => {
            panel = panel.push(entity_visibility::block_control(app));
            use mcedit2_world::scene_annotations::Kind;
            panel = panel.push(
                iced::widget::checkbox(app.entity_models.enabled)
                    .label(l.pick(
                        "显示实体模型（含玩家、物品和展示框）",
                        "Show entity models (including players, items and frames)",
                    ))
                    .on_toggle(Message::ShowEntityModels),
            );
            if app.entity_models.enabled {
                panel = panel.push(entity_visibility::controls(app));
                panel = panel.push(
                    iced::widget::checkbox(app.entity_models.online_skins)
                        .label(l.pick(
                            "在线读取 Mojang 玩家皮肤（默认离线）",
                            "Fetch Mojang player skins online (offline by default)",
                        ))
                        .on_toggle(Message::ShowOnlinePlayerSkins),
                );
                panel = panel.push(text(l.pick(
                "点击可见模型、旗帜或地图查看来源，无需开启位置标记点选。",
                "Click a visible model, block entity or map to inspect its source; marker inspection is not required.",
            )).size(12));
            }
            if !app.entity_models.status.is_empty() {
                panel = panel.push(text(&app.entity_models.status).size(12));
            }
            panel = panel.push(scene_layers::toggles(app));
            if app.scene_layers.mask != 0
                || app.entity_models.enabled
                || app
                    .scene_layers
                    .selected
                    .as_ref()
                    .is_some_and(|m| m.marker().block_state.is_some())
            {
                panel = panel.push(
                    text(l.pick(
                        "定位框可穿透地形显示；实体框不是碰撞箱。",
                        "Location markers show through terrain; entity markers are not hitboxes.",
                    ))
                    .size(12),
                );
                panel = panel.push(
                    iced::widget::checkbox(app.scene_layers.inspect)
                        .label(l.pick("只读点选图层来源", "Inspect layer sources (read only)"))
                        .on_toggle(Message::InspectSceneLayers),
                );
                panel = panel.push(
                    button(l.pick("重新读取显示图层", "Reload display layers"))
                        .on_press(Message::ReloadSceneLayers),
                );
                panel = panel.push(text(&app.scene_layers.status).size(12));
                panel = panel.push(
                    button(l.pick("展开/收起图层诊断", "Toggle layer diagnostics"))
                        .on_press(Message::SceneLayerDiagnostics),
                );
                if app.scene_layers.show_diagnostics {
                    let model_diagnostics = app
                        .entity_models
                        .frame
                        .as_ref()
                        .map(|f| f.diagnostics.join("\n"))
                        .unwrap_or_default();
                    panel = panel.push(
                        scrollable(
                            text(format!(
                                "{}\n{}",
                                app.scene_layers.diagnostics(),
                                model_diagnostics
                            ))
                            .size(12),
                        )
                        .height(Length::Fixed(112.)),
                    );
                }
                if let Some(detail) = app.scene_layers.detail(l) {
                    panel =
                        panel.push(scrollable(text(detail).size(12)).height(Length::Fixed(112.)));
                }
                if app
                    .scene_layers
                    .selected
                    .as_ref()
                    .is_some_and(|m| m.marker().kind == Kind::Player)
                {
                    panel = panel.push(
                        row![
                            button(l.pick("跳到玩家", "Go to player"))
                                .on_press(Message::PlayerJump),
                            button(l.pick("打开玩家背包", "Open player inventory"))
                                .on_press(Message::PlayerInventory),
                            button("玩家属性").on_press(Message::PlayerAttributes),
                            button("暂存玩家朝向相机").on_press(Message::PlayerRotateToCamera),
                            button(l.pick("暂存玩家到相机", "Stage player at camera"))
                                .on_press(Message::PlayerMoveToCamera)
                        ]
                        .spacing(5),
                    );
                }
                if let (Some(selected), Some(frame)) = (
                    app.scene_layers.selected.as_ref(),
                    app.entity_models.frame.as_ref(),
                ) {
                    if Some(frame.snapshot.key) == app.current_key() {
                        if let Some(name) = frame.player_names.get(&selected.marker().sha256) {
                            panel = panel.push(text(format!("Mojang profile：{name}")).size(12));
                        }
                    }
                }
            }
        }
        Group::Region => {
            panel=panel.push(text(l.pick("只查看指定范围时使用局部预览；返回世界视图后继续按相机位置加载。", "Use local previews for a limited area; return to world view to load around the camera.")).size(13));
            panel = panel.push(text(app.window_caption()).size(12));
            panel = panel.push(
                row![
                    button(l.pick("表面预览", "Surface"))
                        .style(button::secondary)
                        .on_press(Message::InspectSurface),
                    button(l.pick("剖切预览", "Cutaway"))
                        .style(button::secondary)
                        .on_press(Message::InspectVolume)
                ]
                .spacing(8)
                .wrap(),
            );
            if app.selected_window().is_some() {
                panel = panel.push(
                    button(l.pick("返回世界视图", "Return to world view"))
                        .on_press(Message::FollowWorld),
                );
            }
            panel = panel.push(
                row![
                    button("16×16").on_press(Message::ScenePreset(16)),
                    button("32×32").on_press(Message::ScenePreset(32)),
                    button("64×64").on_press(Message::ScenePreset(64))
                ]
                .spacing(6),
            );
            panel = panel.push(
                row![
                    button(l.pick("X−", "X−")).on_press(Message::Shift3D(-16, 0, 0)),
                    button(l.pick("X+", "X+")).on_press(Message::Shift3D(16, 0, 0)),
                    button(l.pick("Z−", "Z−")).on_press(Message::Shift3D(0, 0, -16)),
                    button(l.pick("Z+", "Z+")).on_press(Message::Shift3D(0, 0, 16)),
                ]
                .spacing(4),
            );
            panel = panel.push(
                row![
                    button(l.pick("Y−", "Y−")).on_press(Message::Shift3D(0, -16, 0)),
                    button(l.pick("Y+", "Y+")).on_press(Message::Shift3D(0, 16, 0)),
                    button(l.pick("定位方块", "Center")).on_press(Message::Center3DOnPicked),
                ]
                .spacing(5),
            );
            panel = panel.push(
                row![
                    button("−").on_press(Message::ZoomSurface(-1)),
                    button("+").on_press(Message::ZoomSurface(1)),
                    button(l.pick("重置视角", "Reset camera"))
                        .on_press(Message::ResetSurfaceCamera),
                ]
                .spacing(5),
            );
            panel = panel.push(text(l.pick("远景采样", "Distant terrain")).size(16));
            panel = panel.push(
                row![
                    button(l.pick("16 区块", "16 chunks")).on_press(Message::StartFarView(16)),
                    button(l.pick("64 区块", "64 chunks")).on_press(Message::StartFarView(64)),
                ]
                .spacing(4),
            );
            panel = panel.push(
                row![
                    button(l.pick("256 区块", "256 chunks")).on_press(Message::StartFarView(256)),
                    button(l.pick("1024 区块", "1024 chunks"))
                        .on_press(Message::StartFarView(1024)),
                ]
                .spacing(4),
            );
        }
        Group::Diagnostics => {
            panel = panel.push(
                row![
                    button(if app.spectator {
                        if app.spectator_camera.perspective {
                            l.pick("GPU 透视相机", "GPU perspective camera")
                        } else {
                            l.pick("GPU 正交相机", "GPU orthographic camera")
                        }
                    } else if app.uses_world_mesh() {
                        l.pick("GPU 世界等轴", "GPU world isometric")
                    } else if app.use_gpu {
                        l.pick("GPU 开", "GPU on")
                    } else {
                        l.pick("GPU 关", "GPU off")
                    })
                    .on_press_maybe(
                        (!app.spectator && !app.uses_world_mesh()).then_some(Message::ToggleGpu)
                    ),
                    button(if app.show_textures {
                        l.pick("纹理开", "Textures on")
                    } else {
                        l.pick("纹理关", "Textures off")
                    })
                    .on_press(Message::ToggleTextures),
                ]
                .spacing(5),
            );
            if app.uses_world_mesh() {
                if let Some(stream) = app.spectator_stream.as_ref() {
                    panel = panel.push(text(stream.status()).size(12));
                    let displayed = app
                        .spectator_presented
                        .lock()
                        .expect("编辑器场景状态锁被毒化")
                        .clone();
                    let pieces = displayed.as_ref().map_or(0, |mesh| mesh.chunks.len());
                    let ready = app
                        .spectator_stream_snapshot
                        .as_ref()
                        .and_then(|snapshot| {
                            snapshot
                                .ready
                                .lock()
                                .expect("编辑器场景状态锁被毒化")
                                .clone()
                        })
                        .or_else(|| {
                            app.spectator_cache
                                .lock()
                                .expect("编辑器场景状态锁被毒化")
                                .clone()
                        });
                    let uploading = ready.as_ref().is_some_and(|mesh| {
                        displayed
                            .as_ref()
                            .is_none_or(|old| !std::sync::Arc::ptr_eq(old, mesh))
                    });
                    let pending = *app
                        .spectator_pending
                        .lock()
                        .expect("编辑器场景状态锁被毒化");
                    panel = panel.push(
                        text(format!(
                            "{}：{} {}",
                            l.pick("已显示网格片段", "Displayed mesh parts"),
                            pieces,
                            if pending {
                                l.pick("（继续建模）", "(meshing)")
                            } else if uploading {
                                l.pick("（正在提交画面）", "(preparing display)")
                            } else {
                                ""
                            }
                        ))
                        .size(12),
                    );
                }
            }
        }
    }
    panel.into()
}
