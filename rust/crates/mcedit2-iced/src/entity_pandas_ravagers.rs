//! 熊猫实际保存基因、幼年分组与劫掠兽保存计时驱动的颈部 / 下颌姿态。
use super::*;

pub(crate) fn supports(id: &str) -> bool {
    matches!(id, "minecraft:panda" | "minecraft:ravager")
}

#[derive(Clone, Copy, PartialEq)]
enum Gene {
    Normal,
    Lazy,
    Worried,
    Playful,
    Brown,
    Weak,
    Aggressive,
}
impl Gene {
    fn read(visual: &EntityVisual, key: &str) -> Result<Self, String> {
        match visual.source_field_optional(key)? {
            None => Ok(Self::Normal),
            Some(Value::String(name)) => match name.as_str() {
                "normal" => Ok(Self::Normal),
                "lazy" => Ok(Self::Lazy),
                "worried" => Ok(Self::Worried),
                "playful" => Ok(Self::Playful),
                "brown" => Ok(Self::Brown),
                "weak" => Ok(Self::Weak),
                "aggressive" => Ok(Self::Aggressive),
                _ => Err(format!("熊猫 {key}={name} 缺少实际基因定义")),
            },
            _ => Err(format!("熊猫 {key} 必须为 String")),
        }
    }
    fn texture(self) -> &'static str {
        match self {
            Self::Normal => "minecraft:entity/panda/panda",
            Self::Lazy => "minecraft:entity/panda/lazy_panda",
            Self::Worried => "minecraft:entity/panda/worried_panda",
            Self::Playful => "minecraft:entity/panda/playful_panda",
            Self::Brown => "minecraft:entity/panda/brown_panda",
            Self::Weak => "minecraft:entity/panda/weak_panda",
            Self::Aggressive => "minecraft:entity/panda/aggressive_panda",
        }
    }
}

const LEGS: [&str; 4] = [
    "/right_hind_leg",
    "/left_hind_leg",
    "/right_front_leg",
    "/left_front_leg",
];
fn rotate(poses: &mut BTreeMap<String, PartPose>, path: &str, rotation: [f32; 3]) {
    poses.insert(
        path.into(),
        PartPose {
            rotation: Some(rotation),
            ..Default::default()
        },
    );
}
fn panda(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    style: npc::RenderStyle,
    pitch: f32,
) -> Result<Vec<WorldQuad>, String> {
    let main = Gene::read(visual, "MainGene")?;
    let hidden = Gene::read(visual, "HiddenGene")?;
    let variant = if matches!(main, Gene::Brown | Gene::Weak) && main != hidden {
        Gene::Normal
    } else {
        main
    };
    let young = match visual.source_field_optional("Age")? {
        None => false,
        Some(Value::Int(age)) => age < 0,
        _ => return Err("熊猫 Age 必须为 Int".into()),
    };
    let mut poses = BTreeMap::new();
    rotate(&mut poses, "/head", [pitch * 0.017453292, 0., 0.]);
    for leg in LEGS {
        rotate(&mut poses, leg, [0.; 3]);
    }
    // Sit / OnBack / Roll 插值与不高兴、进食、喷嚏计数均未保存，载入初始为零。
    // 原生持物层要求 isSitting；不能仅凭 HandItems 生成坐姿或人形装备层。
    npc::render(
        catalog,
        bank,
        "minecraft:panda#main",
        variant.texture(),
        &poses,
        npc::RenderStyle {
            groups: young.then_some(npc::Groups {
                head_parts: &["/head"],
                head_scale: 1.5_f32 / 2.7_f32,
                head_translate: [0., 23. / 16., 4.8 / 16.],
                body_scale: 1. / 3.,
                body_translate: [0., 49. / 16., 0.],
            }),
            ..style
        },
    )
}

fn tick(visual: &EntityVisual, key: &str) -> Result<i32, String> {
    match visual.source_field_optional(key)? {
        None => Ok(0),
        Some(Value::Int(value)) => Ok(value),
        _ => Err(format!("劫掠兽 {key} 必须为 Int")),
    }
}
fn ravager(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    style: npc::RenderStyle,
    pitch: f32,
) -> Result<Vec<WorldQuad>, String> {
    let attack = tick(visual, "AttackTick")?;
    let stun = tick(visual, "StunTick")?;
    let roar = tick(visual, "RoarTick")?;
    let pi = std::f32::consts::PI;
    let sin = mcedit2_world::entity_physics::native_sin;
    let mut poses = BTreeMap::new();
    let mut neck = catalog.resolved_pose("minecraft:ravager#main", "/neck", &poses)?;
    let mut pivot = neck.pivot.ok_or("劫掠兽实际颈部缺少 pivot")?;
    let initial_pitch = neck.rotation.ok_or("劫掠兽实际颈部缺少 rotation")?[0];
    // 原生 prepareMobModel 先取颈部上一帧旋转；静态载入使用实际构造姿态，
    // 不跨实体复用可变模型，也不虚构过去已经播放的眩晕帧。
    let mouth = if attack > 0 {
        let wave = ((attack as f32 % 10. - 10. * 0.5).abs() - 10. * 0.25) / (10. * 0.25);
        let stretch = (1. + wave) * 0.5;
        let stretch = stretch * stretch * stretch * 12.;
        pivot[2] = -6.5 + stretch;
        pivot[1] = -7. - stretch * sin(initial_pitch);
        if attack > 5 {
            sin((-4 + attack) as f32 / 4.) * pi * 0.4
        } else {
            0.15707964 * sin(pi * attack as f32 / 10.)
        }
    } else {
        pivot = [0., -7. - -sin(initial_pitch), 5.5];
        neck.rotation = Some([if stun > 0 { 0.21991149 } else { 0. }, 0., 0.]);
        if stun > 0 {
            pivot[0] = ((f64::from(stun) / 40.) * 10.).sin() as f32 * 3.;
            pi * 0.05
        } else if roar > 0 {
            let progress = 20_i32.wrapping_sub(roar) as f32 / 20.;
            1.5707964 * sin(progress * pi * 0.25)
        } else {
            pi * 0.01
        }
    };
    neck.pivot = Some(pivot);
    poses.insert("/neck".into(), neck);
    rotate(&mut poses, "/neck/head", [pitch * 0.017453292, 0., 0.]);
    rotate(&mut poses, "/neck/head/mouth", [mouth, 0., 0.]);
    for leg in LEGS {
        rotate(&mut poses, leg, [0.; 3]);
    }
    npc::render(
        catalog,
        bank,
        "minecraft:ravager#main",
        "minecraft:entity/illager/ravager",
        &poses,
        style,
    )
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if !matches!(catalog.version.as_str(), "1.20.1" | "1.21.1") {
        return Err(format!("{} 没有原生熊猫 / 劫掠兽注册定义", catalog.version));
    }
    let [yaw, pitch] = visual.rotation.ok_or("熊猫 / 劫掠兽缺少实际 Rotation")?;
    let style = npc::RenderStyle {
        yaw: npc::frozen_yaw(catalog, visual, yaw)?,
        groups: None,
        outer: if catalog.version == "1.21.1" {
            npc::scale(visual)?
        } else {
            1.
        },
        tint: [1.; 4],
        ordered: false,
    };
    match id {
        "minecraft:panda" => panda(catalog, bank, visual, style, pitch),
        "minecraft:ravager" => ravager(catalog, bank, visual, style, pitch),
        _ => Err(format!("没有绑定熊猫 / 劫掠兽模型 {id}")),
    }
}
