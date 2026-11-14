//! 使用物品外观的投掷实体：原生注册身份、版本化载荷字段与默认物品。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projectile {
    Snowball,
    Egg,
    EnderPearl,
    EyeOfEnder,
    Potion,
    ExperienceBottle,
    Firework,
    Fireball,
    SmallFireball,
    OminousSpawner,
}
impl Projectile {
    pub fn read(id: &str) -> Option<Self> {
        Some(match id {
            "minecraft:snowball" => Self::Snowball,
            "minecraft:egg" => Self::Egg,
            "minecraft:ender_pearl" => Self::EnderPearl,
            "minecraft:eye_of_ender" | "minecraft:eye_of_ender_signal" => Self::EyeOfEnder,
            "minecraft:potion" => Self::Potion,
            "minecraft:experience_bottle" | "minecraft:xp_bottle" => Self::ExperienceBottle,
            "minecraft:firework_rocket" | "minecraft:fireworks_rocket" => Self::Firework,
            "minecraft:fireball" => Self::Fireball,
            "minecraft:small_fireball" => Self::SmallFireball,
            "minecraft:ominous_item_spawner" => Self::OminousSpawner,
            _ => return None,
        })
    }
    pub fn check_version(self, id: &str, version: i32) -> Result<(), String> {
        match version {
            1343 | 3465 if self == Self::OminousSpawner => {
                Err("该版本没有原生不祥物品生成器".into())
            }
            1343 if matches!(
                id,
                "minecraft:eye_of_ender"
                    | "minecraft:experience_bottle"
                    | "minecraft:firework_rocket"
            ) =>
            {
                Err(format!("旧版没有原生投掷物注册 {id}"))
            }
            3465 | 3955
                if matches!(
                    id,
                    "minecraft:eye_of_ender_signal"
                        | "minecraft:xp_bottle"
                        | "minecraft:fireworks_rocket"
                ) =>
            {
                Err(format!("版本 {version} 不使用旧投掷物注册名 {id}"))
            }
            1343 | 3465 | 3955 => Ok(()),
            _ => Err(format!("尚未绑定 DataVersion={version} 的投掷物格式")),
        }
    }
    /// 原生载入默认值；缺少物品载荷时仍能辨认实际实体类型。
    pub fn default_item(self, version: i32) -> Option<&'static str> {
        Some(match self {
            Self::Snowball => "minecraft:snowball",
            Self::Egg => "minecraft:egg",
            Self::EnderPearl => "minecraft:ender_pearl",
            Self::EyeOfEnder => "minecraft:ender_eye",
            Self::Potion => "minecraft:splash_potion",
            Self::ExperienceBottle => "minecraft:experience_bottle",
            Self::Firework if version == 1343 => "minecraft:fireworks",
            Self::Firework => "minecraft:firework_rocket",
            Self::Fireball | Self::SmallFireball => "minecraft:fire_charge",
            Self::OminousSpawner => return None,
        })
    }
    pub fn payload_field(self, version: i32) -> Option<&'static str> {
        match self {
            Self::OminousSpawner => Some("item"),
            Self::Firework => Some("FireworksItem"),
            Self::Potion if version == 1343 => Some("Potion"),
            _ if version == 1343 => None,
            _ => Some("Item"),
        }
    }
    pub fn fullbright(self) -> bool {
        matches!(self, Self::Fireball | Self::SmallFireball)
    }
    pub(super) fn legacy_dimensions(self) -> [f32; 2] {
        match self {
            Self::OminousSpawner => unreachable!("旧尺寸只用于已通过版本检查的旧实体"),
            Self::Fireball => [1., 1.],
            Self::SmallFireball => [0.3125, 0.3125],
            _ => [0.25, 0.25],
        }
    }
}
