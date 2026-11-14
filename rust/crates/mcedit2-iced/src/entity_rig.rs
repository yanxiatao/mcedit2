//! 静态身体与装备共同使用的真实模型层、保存姿态和年龄变换。
use super::*;
use mcedit2_world::entity_equipment::Arm;
pub(super) fn crossed_transform() -> crate::root_transform::Affine {
    // CrossedArmsItemLayer：T(0,.4,-.4) * RX(180)。
    crate::root_transform::Affine([[1., 0., 0., 0.], [0., -1., 0., 0.4], [0., 0., -1., -0.4]])
}
#[derive(Clone, Copy)]
enum MouthSlot {
    MainHand,
    MainArmSide,
}
pub(crate) struct Rig {
    pub(super) layer: &'static str,
    pub(super) poses: BTreeMap<String, PartPose>,
    pub(super) style: npc::RenderStyle,
    young: bool,
    villager: bool,
    trader: bool,
    witch: bool,
    vex: bool,
    allay: bool,
    mouth_item: Option<(crate::root_transform::Affine, MouthSlot)>,
    villager_head: bool,
    head_items: bool,
    hand_items: bool,
    armor_layers: bool,
    elytra_layer: bool,
    hand_offset: f32,
    head_scale: [f32; 3],
}
impl Rig {
    pub(crate) fn read(
        catalog: &Catalog,
        kind: Kind,
        id: Option<&str>,
        visual: &EntityVisual,
        skin_slim: Option<bool>,
    ) -> Result<Self, String> {
        let [yaw, pitch] = visual.rotation.ok_or("装备挂接缺少实际 Rotation")?;
        let spectator = kind == Kind::Player && visual.player_spectator()?;
        let mut poses = BTreeMap::new();
        let mut groups = None;
        let mut young = false;
        let villager = kind != Kind::Player && id == Some("minecraft:villager");
        let trader = kind != Kind::Player && id == Some("minecraft:wandering_trader");
        let witch = kind != Kind::Player && id == Some("minecraft:witch");
        let vex = kind != Kind::Player && id == Some("minecraft:vex");
        let allay = kind != Kind::Player && id == Some("minecraft:allay");
        let dolphin = kind != Kind::Player && id == Some("minecraft:dolphin");
        let fox = kind != Kind::Player && id == Some("minecraft:fox");
        let giant = kind != Kind::Player && id == Some("minecraft:giant");
        let piglin = kind != Kind::Player && id.is_some_and(piglins::supports);
        let mut mouth_item =
            dolphin.then(|| (swimmers::mouth_transform(pitch), MouthSlot::MainArmSide));
        let golem = kind != Kind::Player && id.is_some_and(golems::supports);
        let legacy_vex = vex && catalog.version == "1.12.2";
        let illager = kind != Kind::Player && id.is_some_and(illagers::supports);
        let head_items = !spectator
            && !golem
            && !witch
            && !allay
            && !dolphin
            && !fox
            && !giant
            && (!vex || legacy_vex)
            && (!illager || catalog.version != "1.12.2");
        let mut hand_items = !spectator && !villager && !trader && !witch && !golem;
        let armor_layers = !spectator
            && !villager
            && !trader
            && !illager
            && !witch
            && !vex
            && !allay
            && !golem
            && !dolphin
            && !fox;
        let elytra_layer = (armor_layers || legacy_vex) && !giant;
        let villager_head = kind != Kind::Player
            && matches!(id, Some("minecraft:villager" | "minecraft:zombie_villager"));
        let mut hand_offset = 0.;
        let layer = if kind == Kind::Player {
            let Some(Value::IntArray(words)) = visual.appearance.get("mcedit2:player_uuid") else {
                return Err("玩家 UUID 缺失，无法确定装备所用手臂模型".into());
            };
            if words.len() != 4 {
                return Err("玩家 UUID 无效".into());
            }
            let hash = words[0] ^ words[1] ^ words[2] ^ words[3];
            let legacy = catalog.version == "1.12.2";
            let slim = skin_slim.unwrap_or(if legacy {
                hash & 1 != 0
            } else {
                hash.rem_euclid(18) < 9
            });
            hand_offset = if slim { 0.5 } else { 0. };
            for head in ["/head", "/hat"] {
                poses.insert(
                    head.into(),
                    PartPose {
                        rotation: Some([pitch.to_radians(), 0., 0.]),
                        ..Default::default()
                    },
                );
            }
            if !legacy {
                for part in ["/cloak", "/ear"] {
                    poses.insert(
                        part.into(),
                        PartPose {
                            visible: Some(false),
                            ..Default::default()
                        },
                    );
                }
            }
            if slim {
                "minecraft:player_slim#main"
            } else {
                "minecraft:player#main"
            }
        } else {
            match id.ok_or("装备挂接缺少实体 ID")? {
                "minecraft:fox" => {
                    let binding = foxes_goats::fox_binding(catalog, visual, pitch)?;
                    poses = binding.poses;
                    groups = binding.groups;
                    young = binding.young;
                    mouth_item = Some((binding.mouth, MouthSlot::MainHand));
                    "minecraft:fox#main"
                }
                "minecraft:dolphin" => {
                    poses = swimmers::dolphin_poses(catalog, visual, pitch)?;
                    "minecraft:dolphin#main"
                }
                id if golems::supports(id) => {
                    let (layer, parts) = golems::binding(catalog, id, pitch)?;
                    poses = parts;
                    layer
                }
                "minecraft:allay" => {
                    if catalog.version == "1.12.2" {
                        return Err("旧客户端没有悦灵".into());
                    }
                    poses = allay::poses(pitch);
                    "minecraft:allay#main"
                }
                "minecraft:vex" => {
                    poses = vex::poses(catalog, pitch);
                    "minecraft:vex#main"
                }
                "minecraft:witch" => {
                    poses = witch::poses(visual, pitch)?;
                    "minecraft:witch#main"
                }
                "minecraft:villager" | "minecraft:wandering_trader" => {
                    young = match visual.appearance.get("Age") {
                        None => false,
                        Some(Value::Int(age)) => *age < 0,
                        _ => return Err("村民 Age 必须为 Int".into()),
                    };
                    poses.insert(
                        "/head".into(),
                        PartPose {
                            rotation: Some([pitch.to_radians(), 0., 0.]),
                            ..Default::default()
                        },
                    );
                    if trader {
                        if catalog.version == "1.12.2" {
                            return Err("旧客户端没有流浪商人".into());
                        }
                        "minecraft:wandering_trader#main"
                    } else {
                        "minecraft:villager#main"
                    }
                }
                "minecraft:armor_stand" => {
                    byte(visual, "Marker")?;
                    poses = npc::poses(visual)?;
                    let arms = byte(visual, "ShowArms")?;
                    for part in ["/left_arm", "/right_arm"] {
                        poses.entry(part.into()).or_default().visible = Some(arms);
                    }
                    poses.entry("/base_plate".into()).or_default().visible =
                        Some(!byte(visual, "NoBasePlate")?);
                    poses.entry("/base_plate".into()).or_default().rotation =
                        Some([0., -yaw.to_radians(), 0.]);
                    young = byte(visual, "Small")?;
                    "minecraft:armor_stand#main"
                }
                skeleton if monsters::is_skeleton(skeleton) => {
                    hand_offset = 1.;
                    for part in ["/head", "/hat"] {
                        poses.insert(
                            part.into(),
                            PartPose {
                                rotation: Some([pitch.to_radians(), 0., 0.]),
                                ..Default::default()
                            },
                        );
                    }
                    monsters::skeleton_layer(catalog, skeleton)?
                }
                id if illagers::supports(id) => {
                    let binding = illagers::binding(catalog, id, visual, pitch)?;
                    poses = binding.poses;
                    hand_items = binding.hand_items;
                    binding.layer
                }
                id if piglins::supports(id) => {
                    let binding = piglins::binding(catalog, id, visual, pitch)?;
                    poses = binding.poses;
                    young = binding.young;
                    binding.layer
                }
                "minecraft:zombie"
                | "minecraft:zombie_villager"
                | "minecraft:husk"
                | "minecraft:drowned"
                | "minecraft:giant"
                | "minecraft:zombie_pigman" => {
                    let id = id.expect("实体 ID 已读取");
                    young = !giant && byte(visual, "IsBaby")?;
                    poses.insert(
                        "/head".into(),
                        PartPose {
                            rotation: Some([pitch.to_radians(), 0., 0.]),
                            ..Default::default()
                        },
                    );
                    monsters::zombie_stance(&mut poses);
                    poses.insert(
                        "/hat".into(),
                        PartPose {
                            rotation: Some([pitch.to_radians(), 0., 0.]),
                            ..Default::default()
                        },
                    );
                    match id {
                        "minecraft:zombie" | "minecraft:husk" | "minecraft:drowned" => {
                            monsters::zombie_layer(catalog, id)?
                        }
                        "minecraft:giant" => {
                            if catalog.version == "1.12.2" {
                                "minecraft:zombie#main"
                            } else {
                                "minecraft:giant#main"
                            }
                        }
                        "minecraft:zombie_villager" => "minecraft:zombie_villager#main",
                        "minecraft:zombie_pigman" if catalog.version == "1.12.2" => {
                            "minecraft:zombie_pigman#main"
                        }
                        _ => {
                            return Err(format!("实体 {id} 与源模型版本 {} 不同", catalog.version))
                        }
                    }
                }
                id => return Err(format!("{id} 尚未绑定装备挂接模型")),
            }
        };
        if spectator {
            for path in catalog.parts(layer)? {
                if !matches!(path, "" | "/head" | "/hat") {
                    poses.entry(path.into()).or_default().visible = Some(false);
                }
            }
        }
        if young && !villager && !trader && !fox {
            groups = Some(npc::Groups {
                head_parts: &["/head"],
                head_scale: 0.75,
                head_translate: [0., 1., 0.],
                body_scale: 0.5,
                body_translate: [0., 1.5, 0.],
            });
        }
        let renderer_scale = if legacy_vex {
            0.4
        } else if giant {
            6.
        } else if kind == Kind::Player || trader || illager || witch {
            0.9375
        } else if villager {
            0.9375 * if young { 0.5 } else { 1. }
        } else {
            1.
        };
        Ok(Self {
            layer,
            poses,
            style: npc::RenderStyle {
                yaw: if kind != Kind::Player && id == Some("minecraft:zombie_villager") {
                    villagers::zombie_yaw(catalog, visual, yaw)?
                } else if piglin {
                    npc::frozen_yaw(catalog, visual, yaw)?
                } else if kind == Kind::Player
                    || illager
                    || witch
                    || vex
                    || allay
                    || golem
                    || dolphin
                    || fox
                    || giant
                {
                    npc::frozen_yaw(catalog, visual, yaw)?
                } else {
                    yaw
                },
                groups,
                outer: (if catalog.version == "1.21.1" {
                    npc::scale(visual)?
                } else {
                    1.
                }) * renderer_scale
                    * id.map_or(1., monsters::renderer_scale),
                tint: [1.; 4],
                ordered: false,
            },
            young,
            villager,
            trader,
            witch,
            vex,
            allay,
            mouth_item,
            villager_head,
            head_items,
            hand_items,
            armor_layers,
            elytra_layer,
            hand_offset,
            head_scale: if piglin {
                [1.0019531, 1., 1.0019531]
            } else {
                [1.; 3]
            },
        })
    }
    pub(crate) fn head_items(&self) -> bool {
        self.head_items
    }
    pub(crate) fn hand_items(&self) -> bool {
        self.hand_items
    }
    pub(crate) fn armor_layers(&self) -> bool {
        self.armor_layers
    }
    pub(crate) fn elytra_layer(&self) -> bool {
        self.elytra_layer
    }
    pub(super) fn young(&self) -> bool {
        self.young
    }
    pub(crate) fn has_single_item_layer(&self) -> bool {
        self.trader || self.witch || self.mouth_item.is_some()
    }
    pub(crate) fn single_item_slot(&self, main: Arm) -> mcedit2_world::entity_equipment::Slot {
        use mcedit2_world::entity_equipment::Slot;
        if matches!(self.mouth_item, Some((_, MouthSlot::MainArmSide))) && main == Arm::Left {
            Slot::OffHand
        } else {
            Slot::MainHand
        }
    }
    pub(crate) fn single_item_transform(
        &self,
        catalog: &Catalog,
        stack: &mcedit2_world::entity_equipment::Stack,
    ) -> Result<(&'static str, crate::root_transform::Affine), String> {
        if self.witch {
            witch::held_transform(catalog, self, stack)
        } else if let Some((transform, _)) = self.mouth_item {
            Ok(("ground", transform))
        } else {
            Ok(("ground", crossed_transform()))
        }
    }
    /// 独立单物品层已经过原生局部变换，此处只应用共享身体坐标系。
    pub(crate) fn single_item_point(&self, point: [f32; 3]) -> Result<[f32; 3], String> {
        mcedit2_world::entity_model::living_offset(point, self.style.yaw)
            .map(|point| point.map(|value| value * self.style.outer))
    }
    /// CustomHeadLayer 的外层独立于身体的幼年分组；头颅不应用物品 JSON 的 head 显示变换。
    pub(crate) fn head_point(
        &self,
        catalog: &Catalog,
        point: [f32; 3],
        skull: bool,
    ) -> Result<[f32; 3], String> {
        let local = if skull {
            [
                1.1875 * point[0],
                -1.1875 * (point[1] + 0.5 + if self.villager_head { 0.0625 } else { 0. }),
                -1.1875 * point[2],
            ]
        } else {
            [
                -0.625 * point[0],
                -0.625 * (point[1] + if self.villager_head { 0.1875 } else { 0. })
                    - if self.layer == "minecraft:snow_golem#main" {
                        0.34375
                    } else {
                        0.25
                    },
                0.625 * point[2],
            ]
        };
        let mut model = catalog.attachment_point(self.layer, "/head", &self.poses, local)?;
        if self.young && !self.villager {
            for i in 0..3 {
                model[i] = (model[i] + if i == 1 { 1. } else { 0. }) * 0.7
                    + if i == 1 { 0.03125 } else { 0. };
            }
        }
        for i in 0..3 {
            model[i] *= self.head_scale[i];
        }
        Ok(
            mcedit2_world::entity_model::living_offset(model, self.style.yaw)?
                .map(|value| value * self.style.outer),
        )
    }
    pub(crate) fn elytra_model(
        &self,
        catalog: &Catalog,
        visual: &EntityVisual,
    ) -> Result<Vec<mcedit2_world::entity_model::ModelQuad>, String> {
        let mut pitch = 0.2617994f32;
        let mut roll = -0.2617994f32;
        if byte(visual, "FallFlying")? {
            let velocity = visual.loaded_motion()?;
            let mut factor = 1f32;
            if velocity[1] < 0. {
                let length = velocity
                    .iter()
                    .map(|value| value * value)
                    .sum::<f64>()
                    .sqrt();
                let y = if length < 1.0e-4 {
                    0.
                } else {
                    velocity[1] / length
                };
                factor = 1. - (-y).powf(1.5) as f32;
            }
            pitch = factor * 0.34906584 + (1. - factor) * pitch;
            roll = factor * -1.5707964 + (1. - factor) * roll;
        }
        let poses = BTreeMap::from([
            (
                "/left_wing".into(),
                PartPose {
                    rotation: Some([pitch, 0., roll]),
                    ..Default::default()
                },
            ),
            (
                "/right_wing".into(),
                PartPose {
                    rotation: Some([pitch, 0., -roll]),
                    ..Default::default()
                },
            ),
        ]);
        let mut quads = catalog.bake("minecraft:elytra#main", &poses)?;
        for face in &mut quads {
            for point in &mut face.corners {
                let mut model = [point[0], point[1], point[2]];
                if self.young {
                    for i in 0..3 {
                        model[i] = (model[i]
                            + match i {
                                1 => 1.5,
                                2 if catalog.version == "1.12.2" => -0.1,
                                _ => 0.,
                            })
                            * 0.5;
                    }
                }
                model[2] += 0.125;
                let model = mcedit2_world::entity_model::living_offset(model, self.style.yaw)?
                    .map(|value| value * self.style.outer);
                point[..3].copy_from_slice(&model);
            }
        }
        Ok(quads)
    }
    pub(crate) fn armor_model(
        &self,
        catalog: &Catalog,
        leggings: bool,
        parts: &[&str],
    ) -> Result<Vec<mcedit2_world::entity_model::ModelQuad>, String> {
        let legacy = catalog.version == "1.12.2";
        let native_legacy_villager = legacy && self.layer == "minecraft:zombie_villager#main";
        let layer = if native_legacy_villager {
            format!(
                "minecraft:zombie_villager#{}",
                if leggings {
                    "inner_armor"
                } else {
                    "outer_armor"
                }
            )
        } else if legacy {
            "minecraft:zombie#main".into()
        } else {
            format!(
                "{}#{}",
                self.layer.split_once('#').expect("原生层已绑定").0,
                if leggings {
                    "inner_armor"
                } else {
                    "outer_armor"
                }
            )
        };
        let mut poses = BTreeMap::new();
        for path in catalog.parts(&layer)? {
            if path.is_empty() {
                continue;
            }
            let source = if path == "/hat" { "/head" } else { path };
            let mut pose = catalog.resolved_pose(self.layer, source, &self.poses)?;
            pose.visible = Some(parts.contains(&path));
            poses.insert(path.to_owned(), pose);
        }
        let mut quads = catalog.bake_deformed(
            &layer,
            &poses,
            if legacy && !native_legacy_villager {
                if leggings {
                    0.5
                } else {
                    1.
                }
            } else {
                0.
            },
        )?;
        for quad in &mut quads {
            for point in &mut quad.corners {
                let mut xyz = [point[0], point[1], point[2]];
                if let Some(groups) = self.style.groups {
                    let (scale, translate) = if quad.part == "/head" {
                        (groups.head_scale, groups.head_translate)
                    } else {
                        (groups.body_scale, groups.body_translate)
                    };
                    for i in 0..3 {
                        xyz[i] = (xyz[i] + translate[i]) * scale;
                    }
                }
                let xyz = mcedit2_world::entity_model::living_offset(xyz, self.style.yaw)?
                    .map(|value| value * self.style.outer);
                point[..3].copy_from_slice(&xyz);
                // ModelZombie 的默认 64 高皮肤和 ModelBiped 护甲的 32 高 UV。
                if legacy && !native_legacy_villager {
                    point[4] *= 2.;
                }
            }
        }
        Ok(quads)
    }
    /// Native ItemInHandLayer 的年轻体变换、部件 translateToHand 与物品局部变换。
    /// 不使用身体可见性决定手持物是否存在；盔甲架隐藏手臂也保留挂接坐标。
    pub(crate) fn hand_point(
        &self,
        catalog: &Catalog,
        arm: Arm,
        point: [f32; 3],
    ) -> Result<[f32; 3], String> {
        let left = arm == Arm::Left;
        let side = if left { -1. } else { 1. };
        let [x, y, z] = [point[0] + side / 16., point[1] + 0.125, point[2] - 0.625];
        // PoseStack 顺序 X(-90)、Y(180)、translate；点的应用顺序相反。
        let mut point = [-x, -z, -y];
        let modern_vex = self.vex && catalog.version != "1.12.2";
        let path = if self.allay {
            // AllayModel 不使用 arm 的完整父链。原生矩阵顺序为 root / body、
            // T(0,.0625,.1875)、RX(0)、S(.7)、T(.0625,0,0)；点按相反顺序变换。
            point = [
                0.7 * (point[0] + 0.0625),
                0.7 * point[1] + 0.0625,
                0.7 * point[2] + 0.1875,
            ];
            "/root/body"
        } else if modern_vex {
            // VexModel.translateToHand：完整 root / body / arm 父链后 S(.55) 与专用平移。
            for i in 0..3 {
                point[i] = 0.55 * (point[i] + [side * 0.046875, -0.15625, 0.078125][i]);
            }
            if left {
                "/root/body/left_arm"
            } else {
                "/root/body/right_arm"
            }
        } else if left {
            "/left_arm"
        } else {
            "/right_arm"
        };
        let mut model = catalog.attachment_point(self.layer, path, &self.poses, point)?;
        model[0] += side * self.hand_offset / 16.;
        if self.young {
            for i in 0..3 {
                model[i] = (model[i] + if i == 1 { 1.5 } else { 0. }) * 0.5;
            }
        }
        Ok(
            mcedit2_world::entity_model::living_offset(model, self.style.yaw)?
                .map(|v| v * self.style.outer),
        )
    }
}
