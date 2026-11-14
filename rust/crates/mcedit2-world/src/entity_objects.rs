//! 小型非生物对象的原生注册身份；显示与光照共用版本边界。
use crate::{entity_catalog::EntityVisual, nbt_document::Value};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Arrow,
    SpectralArrow,
    Trident,
    ShulkerBullet,
    LlamaSpit,
    EvokerFangs,
    LeashKnot,
    EndCrystal,
    FallingBlock,
    PrimedTnt,
    WitherSkull,
    DragonFireball,
    WindCharge,
    FishingBobber,
    AreaEffectCloud,
    Marker,
    Interaction,
    Lightning,
}
impl Object {
    pub fn read(id: &str) -> Option<Self> {
        Some(match id {
            "minecraft:arrow" => Self::Arrow,
            "minecraft:spectral_arrow" => Self::SpectralArrow,
            "minecraft:trident" => Self::Trident,
            "minecraft:shulker_bullet" => Self::ShulkerBullet,
            "minecraft:llama_spit" => Self::LlamaSpit,
            "minecraft:evoker_fangs" | "minecraft:evocation_fangs" => Self::EvokerFangs,
            "minecraft:leash_knot" => Self::LeashKnot,
            "minecraft:end_crystal" | "minecraft:ender_crystal" => Self::EndCrystal,
            "minecraft:falling_block" => Self::FallingBlock,
            "minecraft:tnt" => Self::PrimedTnt,
            "minecraft:wither_skull" => Self::WitherSkull,
            "minecraft:dragon_fireball" => Self::DragonFireball,
            "minecraft:wind_charge" | "minecraft:breeze_wind_charge" => Self::WindCharge,
            "minecraft:fishing_bobber" => Self::FishingBobber,
            "minecraft:area_effect_cloud" => Self::AreaEffectCloud,
            "minecraft:marker" => Self::Marker,
            "minecraft:interaction" => Self::Interaction,
            "minecraft:lightning_bolt" => Self::Lightning,
            _ => return None,
        })
    }
    pub fn check_version(self, id: &str, version: i32) -> Result<(), String> {
        match version {
            1343 | 3465 if self == Self::WindCharge => Err("该版本没有原生风弹".into()),
            1343 if matches!(
                self,
                Self::Marker | Self::Interaction | Self::FishingBobber | Self::Lightning
            ) =>
            {
                Err(format!("旧版没有此原生保存注册 {id}"))
            }
            1343 if self == Self::Trident
                || matches!(id, "minecraft:evoker_fangs" | "minecraft:end_crystal") =>
            {
                Err(format!("旧版没有原生对象注册 {id}"))
            }
            3465 | 3955
                if matches!(id, "minecraft:evocation_fangs" | "minecraft:ender_crystal") =>
            {
                Err(format!("版本 {version} 不使用旧对象注册名 {id}"))
            }
            1343 | 3465 | 3955 => Ok(()),
            _ => Err(format!("尚未绑定 DataVersion={version} 的小型对象")),
        }
    }
    /// 旧客户端构造尺寸；不应用生物年龄或 generic.scale 属性。
    pub(super) fn legacy_dimensions(self) -> [f32; 2] {
        match self {
            Self::Arrow | Self::SpectralArrow | Self::Trident | Self::LeashKnot => [0.5, 0.5],
            Self::ShulkerBullet => [0.3125, 0.3125],
            Self::LlamaSpit => [0.25, 0.25],
            Self::EvokerFangs => [0.5, 0.8],
            Self::EndCrystal => [2., 2.],
            Self::FallingBlock | Self::PrimedTnt => [0.98, 0.98],
            Self::WitherSkull => [0.3125, 0.3125],
            Self::DragonFireball => [1., 1.],
            Self::AreaEffectCloud => [6., 0.5],
            Self::WindCharge
            | Self::FishingBobber
            | Self::Marker
            | Self::Interaction
            | Self::Lightning => unreachable!("旧尺寸只用于已通过版本检查的实际旧实体"),
        }
    }
    pub fn editor_location(self) -> bool {
        matches!(
            self,
            Self::AreaEffectCloud | Self::Marker | Self::Interaction | Self::Lightning
        )
    }
}
fn scalar(visual: &EntityVisual, key: &str, default: f32) -> Result<f32, String> {
    let value = match visual.source_field_optional(key)? {
        None => default,
        Some(Value::Float(value)) => value,
        _ => return Err(format!("实体 {key} 必须为 Float")),
    };
    if !value.is_finite() {
        return Err(format!("实体 {key} 不是有限数"));
    }
    Ok(value)
}
/// 现代 setRadius 的原生范围限制；旧版不借用现代上限。
pub fn cloud_radius(visual: &EntityVisual) -> Result<f32, String> {
    let value = scalar(visual, "Radius", 3.)?;
    if visual.source_version == Some(1343) {
        if value < 0. {
            return Err("效果云半径不能为负".into());
        }
        Ok(value)
    } else {
        Ok(value.clamp(0., 32.))
    }
}
pub fn interaction_size(visual: &EntityVisual) -> Result<[f32; 2], String> {
    let width = scalar(visual, "width", 1.)?;
    let height = scalar(visual, "height", 1.)?;
    if width < 0. || height < 0. {
        return Err("交互范围宽高不能为负".into());
    }
    Ok([width, height])
}
