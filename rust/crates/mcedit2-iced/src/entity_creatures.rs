//! 按准确客户端目录装配旧版僵尸猪人与潜影贝静态身体。
use super::npc;
use super::*;

/// 外层按实际源版本选择目录并筛选可见性；此处只装配已核对的实体 ID。
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
    yaw: f32,
    pitch: f32,
) -> Result<Vec<WorldQuad>, String> {
    if !yaw.is_finite() || !pitch.is_finite() {
        return Err("实体 Rotation 包含非有限角度".into());
    }
    let scale = if catalog.version == "1.21.1" {
        npc::scale(visual)?
    } else {
        1.
    };
    match id {
        "minecraft:zombie_pigman" if catalog.version == "1.12.2" => {
            let rig = Rig::read(catalog, Kind::Entity, Some(id), visual, None)?;
            npc::render(
                catalog,
                bank,
                rig.layer,
                "minecraft:entity/zombie_pigman",
                &rig.poses,
                rig.style,
            )
        }
        // Shulker.sanitizeScale 将通用属性上限进一步限制到 3。
        "minecraft:shulker" => shulker(catalog, bank, visual, yaw, pitch, scale.min(3.)),
        _ => Err(format!("未支持的生物模型：{id}")),
    }
}

fn nbt_byte(visual: &EntityVisual, key: &str, default: i8) -> Result<i8, String> {
    match visual.appearance.get(key) {
        None => Ok(default),
        Some(Value::Byte(value)) => Ok(*value),
        _ => Err(format!("潜影贝 {key} 不是 Byte")),
    }
}

fn shulker(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    yaw: f32,
    pitch: f32,
    scale: f32,
) -> Result<Vec<WorldQuad>, String> {
    let face = nbt_byte(visual, "AttachFace", 0)?;
    if !(0..=5).contains(&face) {
        return Err(format!("潜影贝 AttachFace 超出范围：{face}"));
    }
    let legacy = catalog.version == "1.12.2";
    let color = nbt_byte(visual, "Color", if legacy { 0 } else { 16 })?;
    let colors = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let texture = match color {
        16 if !legacy => "minecraft:entity/shulker/shulker".to_owned(),
        0..=15 => format!(
            "minecraft:entity/shulker/shulker_{}",
            if legacy && color == 8 {
                "silver"
            } else {
                colors[color as usize]
            }
        ),
        _ => return Err(format!("潜影贝 Color 超出范围：{color}")),
    };
    // 静态快照使用持久目标开合值，无法恢复客户端插值和开合摆动。
    let (opening, s) = mcedit2_world::entity_physics::shulker_opening(visual)?;
    let lid_y = 16. + s * 8.;
    let lid_yaw = if opening > 0.3 {
        (s - 1.).powi(4) * std::f32::consts::PI * 0.125
    } else {
        0.
    };
    let mut poses: BTreeMap<String, PartPose> = BTreeMap::new();
    poses.insert(
        "/lid".into(),
        PartPose {
            pivot: Some([0., lid_y, 0.]),
            rotation: Some([0., lid_yaw, 0.]),
            ..Default::default()
        },
    );
    // 头部和身体的相对朝向没有持久化；选取静态对齐快照。
    poses.insert(
        "/head".into(),
        PartPose {
            rotation: Some([pitch.to_radians(), -std::f32::consts::PI, 0.]),
            ..Default::default()
        },
    );
    let atlas = bank.atlas().ok_or("实体模型缺少图集")?;
    let params = serde_json::json!({"texture":texture,"poses":poses,"attach_face":face,"yaw":yaw,"outer":scale});
    crate::layer_renderer::builtin_quads_with_catalog(
        "shulker_entity",
        params.as_object().ok_or("潜影贝外观参数必须为对象")?,
        atlas,
        true,
        Some(catalog),
    )
}
