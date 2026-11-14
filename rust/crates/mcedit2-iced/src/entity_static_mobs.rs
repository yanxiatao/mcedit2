//! 以辨认与定位为目标的生物展示：复用原生几何、主要外观和静态保存姿态。
//! 不重建客户端动画时钟；过渡状态选择可辨认的静态形态。
use super::*;
use mcedit2_world::{
    entity_animation::{Animations, Sample},
    entity_equipment::{Slot, Source},
};

#[derive(Clone, Copy)]
enum Mob {
    Camel,
    Sniffer,
    Armadillo,
    Axolotl,
    Tadpole,
    Breeze,
}
impl Mob {
    fn read(id: &str) -> Option<Self> {
        match id {
            "minecraft:camel" => Some(Self::Camel),
            "minecraft:sniffer" => Some(Self::Sniffer),
            "minecraft:armadillo" => Some(Self::Armadillo),
            "minecraft:axolotl" => Some(Self::Axolotl),
            "minecraft:tadpole" => Some(Self::Tadpole),
            "minecraft:breeze" => Some(Self::Breeze),
            _ => None,
        }
    }
    fn layer(self) -> &'static str {
        match self {
            Self::Camel => "minecraft:camel#main",
            Self::Sniffer => "minecraft:sniffer#main",
            Self::Armadillo => "minecraft:armadillo#main",
            Self::Axolotl => "minecraft:axolotl#main",
            Self::Tadpole => "minecraft:tadpole#main",
            Self::Breeze => "minecraft:breeze#main",
        }
    }
    fn baby_transform(self) -> Option<(f32, f32)> {
        // 模型空间先缩放再平移；不能只缩小脚部相对坐标而把模型抬离地面。
        match self {
            Self::Camel => Some((0.45, 1.834375)),
            Self::Sniffer | Self::Axolotl => Some((0.5, 24. / 16.)),
            Self::Armadillo => Some((0.6, 16.02 / 16.)),
            Self::Tadpole | Self::Breeze => None,
        }
    }
}
pub(crate) fn supports(id: &str) -> bool {
    Mob::read(id).is_some()
}
fn young(visual: &EntityVisual) -> Result<bool, String> {
    match visual.source_field_optional("Age")? {
        None => Ok(false),
        Some(Value::Int(age)) => Ok(age < 0),
        _ => Err("生物 Age 必须为 Int".into()),
    }
}
fn visible(poses: &mut BTreeMap<String, PartPose>, path: &str, value: bool) {
    poses.entry(path.into()).or_default().visible = Some(value);
}
fn head(poses: &mut BTreeMap<String, PartPose>, path: &str, pitch: f32) {
    poses.entry(path.into()).or_default().rotation = Some([pitch.to_radians(), 0., 0.]);
}
fn static_pose(
    catalog: &Catalog,
    layer: &str,
    poses: &mut BTreeMap<String, PartPose>,
    version: i32,
    animation: &str,
    root: &str,
) -> Result<(), String> {
    Animations::builtin(version)?.apply(
        catalog,
        layer,
        poses,
        Sample {
            animation,
            root,
            elapsed_ms: 0,
            weight: 1.,
        },
    )
}
fn camel(
    catalog: &Catalog,
    visual: &EntityVisual,
    poses: &mut BTreeMap<String, PartPose>,
    version: i32,
    pitch: f32,
) -> Result<(), String> {
    let sitting = match visual.source_field_optional("LastPoseTick")? {
        None => false,
        Some(Value::Long(tick)) => tick < 0,
        _ => return Err("骆驼 LastPoseTick 必须为 Long".into()),
    };
    if sitting {
        // 只取坐姿的固定形态，不再追踪坐下 / 起身过渡的精确播放时间。
        static_pose(
            catalog,
            Mob::Camel.layer(),
            poses,
            version,
            "minecraft:camel/sit_pose",
            "",
        )?;
    }
    head(poses, "/body/head", pitch.clamp(-25., 45.));
    let source = Source::read(visual, false)?;
    let saddle = source
        .items
        .get(&Slot::Saddle)
        .is_some_and(|stack| stack.id == "minecraft:saddle");
    let riding = match visual.source_field_optional("Passengers")? {
        None => false,
        Some(Value::List(list)) => !list.items.is_empty(),
        _ => return Err("骆驼 Passengers 必须为 List".into()),
    };
    visible(poses, "/body/saddle", saddle);
    visible(poses, "/body/head/bridle", saddle);
    visible(poses, "/body/head/reins", saddle && riding);
    Ok(())
}
fn armadillo(
    visual: &EntityVisual,
    poses: &mut BTreeMap<String, PartPose>,
    pitch: f32,
) -> Result<(), String> {
    let rolled = match visual.source_field_optional("state")? {
        None => false,
        Some(Value::String(state)) => match state.as_str() {
            "idle" | "unrolling" => false,
            "rolling" | "scared" => true,
            _ => return Err(format!("未知犰狳 state={state}")),
        },
        _ => return Err("犰狳 state 必须为 String".into()),
    };
    // 卷起时直接展示壳形态，省略蜷缩、窥探与伸展的中间帧。
    visible(poses, "/cube", rolled);
    for path in [
        "/body",
        "/right_hind_leg",
        "/left_hind_leg",
        "/right_front_leg",
        "/left_front_leg",
    ] {
        visible(poses, path, !rolled);
    }
    head(poses, "/body/head", pitch.clamp(-22.5, 25.));
    Ok(())
}
fn axolotl_texture(visual: &EntityVisual) -> Result<&'static str, String> {
    match visual.source_field_optional("Variant")? {
        None | Some(Value::Int(0)) => Ok("minecraft:entity/axolotl/axolotl_lucy"),
        Some(Value::Int(1)) => Ok("minecraft:entity/axolotl/axolotl_wild"),
        Some(Value::Int(2)) => Ok("minecraft:entity/axolotl/axolotl_gold"),
        Some(Value::Int(3)) => Ok("minecraft:entity/axolotl/axolotl_cyan"),
        Some(Value::Int(4)) => Ok("minecraft:entity/axolotl/axolotl_blue"),
        Some(Value::Int(value)) => Err(format!("未知美西螈 Variant={value}")),
        _ => Err("美西螈 Variant 必须为 Int".into()),
    }
}
fn breeze(
    catalog: &Catalog,
    bank: &TextureBank,
    style: npc::RenderStyle,
    pitch: f32,
) -> Result<Vec<WorldQuad>, String> {
    let mut poses = BTreeMap::new();
    head(&mut poses, "/body/head", pitch);
    visible(&mut poses, "/wind_body", false);
    visible(&mut poses, "/body/head/eyes", false);
    let mut out = npc::render(
        catalog,
        bank,
        Mob::Breeze.layer(),
        "minecraft:entity/breeze/breeze",
        &poses,
        style,
    )?;
    visible(&mut poses, "/body", false);
    visible(&mut poses, "/wind_body", true);
    out.extend(npc::render_material(
        catalog, bank, "minecraft:breeze_wind#main", "minecraft:entity/breeze/breeze_wind", &poses, style,
        &serde_json::json!({"alpha":"BLEND", "alpha_test":true, "cutoff":0.1, "double_sided":true, "mipmap":false}),
    )?);
    // 眼睛使用真实独立图片和已有部件，不复制风体或重复覆盖整张身体图片。
    for path in catalog.parts(Mob::Breeze.layer())? {
        poses.entry(path.into()).or_default().skip_draw = Some(path != "/body/head/eyes");
    }
    visible(&mut poses, "/body", true);
    visible(&mut poses, "/wind_body", false);
    visible(&mut poses, "/body/head/eyes", true);
    out.extend(npc::render(
        catalog,
        bank,
        Mob::Breeze.layer(),
        "minecraft:entity/breeze/breeze_eyes",
        &poses,
        style,
    )?);
    Ok(out)
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let mob = Mob::read(id).ok_or_else(|| format!("没有绑定静态生物模型 {id}"))?;
    let version = match catalog.version.as_str() {
        "1.20.1" if !matches!(mob, Mob::Armadillo | Mob::Breeze) => 3465,
        "1.21.1" => 3955,
        _ => return Err(format!("{} 没有已绑定的原生实体 {id}", catalog.version)),
    };
    let [yaw, pitch] = visual.rotation.ok_or("生物缺少实际 Rotation")?;
    let mut style = npc::RenderStyle {
        yaw: npc::frozen_yaw(catalog, visual, yaw)?,
        outer: if version == 3955 {
            npc::scale(visual)?
        } else {
            1.
        },
        groups: None,
        tint: [1.; 4],
        ordered: false,
    };
    let young = if let Some((scale, translation)) = mob.baby_transform() {
        let young = young(visual)?;
        if young {
            style.groups = Some(npc::Groups {
                head_parts: &[],
                head_scale: scale,
                head_translate: [0., translation, 0.],
                body_scale: scale,
                body_translate: [0., translation, 0.],
            });
        }
        young
    } else {
        false
    };
    let mut poses = BTreeMap::new();
    let texture = match mob {
        Mob::Camel => {
            camel(catalog, visual, &mut poses, version, pitch)?;
            "minecraft:entity/camel/camel"
        }
        Mob::Sniffer => {
            head(&mut poses, "/root/bone/body/head", pitch);
            if young {
                static_pose(
                    catalog,
                    mob.layer(),
                    &mut poses,
                    version,
                    "minecraft:sniffer/baby_transform",
                    "/root",
                )?;
            }
            "minecraft:entity/sniffer/sniffer"
        }
        Mob::Armadillo => {
            armadillo(visual, &mut poses, pitch)?;
            "minecraft:entity/armadillo"
        }
        Mob::Axolotl => {
            head(&mut poses, "/body/head", pitch);
            axolotl_texture(visual)?
        }
        Mob::Tadpole => "minecraft:entity/tadpole/tadpole",
        Mob::Breeze => return breeze(catalog, bank, style, pitch),
    };
    npc::render(catalog, bank, mob.layer(), texture, &poses, style)
}
