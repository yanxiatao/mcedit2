//! 与三个准确客户端一致的药水、染色、烟火和旧刷怪蛋颜色；无世界状态依赖。
use crate::{
    item_icons::{field, number},
    item_presets::{self, PotionEffect},
};
use mcedit2_world::nbt_document::Value;

fn compound(value: Option<&Value>) -> Result<Option<&Value>, String> {
    match value {
        None => Ok(None),
        Some(v @ Value::Compound(_)) => Ok(Some(v)),
        _ => Err("物品颜色载荷必须为 Compound".into()),
    }
}
fn integer(value: Option<&Value>) -> Result<Option<i32>, String> {
    match value {
        None => Ok(None),
        Some(v) => number(Some(v))
            .map(Some)
            .ok_or("物品颜色数字字段类型无效".into()),
    }
}
fn string(value: Option<&Value>) -> Result<Option<&str>, String> {
    match value {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        _ => Err("物品颜色资源 ID 必须为 String".into()),
    }
}
fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.into()
    } else {
        format!("minecraft:{id}")
    }
}
fn potion(
    version: Option<i32>,
    tag: Option<&Value>,
    components: Option<&Value>,
) -> Result<i32, String> {
    let catalog = item_presets::get(version).ok_or("药水颜色没有准确版本数据")?;
    let modern = version == Some(3955);
    let contents = field(components, "minecraft:potion_contents");
    let shorthand = if modern {
        match contents {
            Some(Value::String(id)) => Some(id.as_str()),
            _ => None,
        }
    } else {
        None
    };
    let data = if modern {
        if shorthand.is_some() {
            None
        } else {
            compound(contents)?
        }
    } else {
        tag
    };
    if let Some(color) = integer(field(
        data,
        if modern {
            "custom_color"
        } else {
            "CustomPotionColor"
        },
    ))? {
        return Ok(color);
    }
    let id = qualified(
        shorthand
            .or(string(field(
                data,
                if modern { "potion" } else { "Potion" },
            ))?)
            .unwrap_or("minecraft:empty"),
    );
    // 旧 PotionUtils 对 EMPTY 在读取自定义效果前直接返回空瓶颜色。
    if !modern && id == "minecraft:empty" {
        return Ok(16253176);
    }
    let mut effects = if id == "minecraft:empty" && modern {
        Vec::new()
    } else {
        catalog
            .potions
            .get(&id)
            .ok_or_else(|| format!("未知药水颜色：{id}"))?
            .clone()
    };
    if let Some(value) = field(
        data,
        if modern {
            "custom_effects"
        } else {
            "CustomPotionEffects"
        },
    ) {
        let Value::List(list) = value else {
            return Err("药水效果必须为 List".into());
        };
        if list.element_type != 10 && !(list.element_type == 0 && list.items.is_empty()) {
            return Err("药水效果列表必须包含 Compound".into());
        }
        for value in &list.items {
            let value = compound(Some(value))?;
            let id = if modern {
                qualified(string(field(value, "id"))?.ok_or("药水效果缺少 id")?)
            } else {
                let n = integer(field(value, "Id"))?.ok_or("药水效果缺少 Id")? & 255;
                catalog
                    .effects
                    .iter()
                    .find(|(_, effect)| effect.numeric == Some(n))
                    .map(|(id, _)| id.clone())
                    .ok_or_else(|| format!("未知药水效果编号 {n}"))?
            };
            let amplifier =
                integer(field(value, if modern { "amplifier" } else { "Amplifier" }))?.unwrap_or(0);
            if !(0..=255).contains(&amplifier) {
                return Err("药水效果等级超出 0..255".into());
            }
            let visible = integer(field(
                value,
                if modern {
                    "show_particles"
                } else {
                    "ShowParticles"
                },
            ))?
            .unwrap_or(1)
                != 0;
            effects.push(PotionEffect {
                id,
                amplifier,
                visible,
            });
        }
    }
    if effects.is_empty() {
        return Ok(3694022);
    }
    let mut sums = [0i32; 3];
    let mut floats = [0f32; 3];
    let mut weight = 0;
    for effect in effects.iter().filter(|e| e.visible) {
        let color = catalog
            .effects
            .get(&effect.id)
            .ok_or_else(|| format!("未知药水效果 {}", effect.id))?
            .color;
        let w = effect.amplifier + 1;
        for (i, shift) in [16, 8, 0].into_iter().enumerate() {
            let channel = w * ((color >> shift) & 255);
            sums[i] += channel;
            floats[i] += channel as f32 / 255.;
        }
        weight += w;
    }
    if weight == 0 {
        return Ok(if modern { 3694022 } else { 0 });
    }
    let channels = if modern {
        sums.map(|n| n / weight)
    } else {
        floats.map(|n| (n / weight as f32 * 255.) as i32)
    };
    Ok((channels[0] << 16) | (channels[1] << 8) | channels[2])
}
/// 佩戴层直接读取实际染色载荷，不依赖物品图标是否注册了某个 tint 索引。
pub(crate) fn dyed_color(version: i32, value: &Value) -> Result<Option<u32>, String> {
    let color = if version == 3955 {
        let components = compound(field(Some(value), "components"))?;
        let dyed = field(components, "minecraft:dyed_color");
        match dyed {
            Some(Value::Int(color)) => Some(*color),
            _ => integer(field(compound(dyed)?, "rgb"))?,
        }
    } else if matches!(version, 1343 | 3465) {
        let tag = compound(field(Some(value), "tag"))?;
        integer(field(compound(field(tag, "display"))?, "color"))?
    } else {
        return Err(format!("未绑定版本 {version} 的物品染色格式"));
    };
    color
        .map(|color| {
            if (0..=0xffffff).contains(&color) {
                Ok(color as u32)
            } else {
                Err("物品染色 RGB 超出范围".into())
            }
        })
        .transpose()
}
pub(crate) fn resolve(
    id: &str,
    version: Option<i32>,
    value: Option<&Value>,
    mut colors: Vec<i32>,
) -> Result<Vec<i32>, String> {
    if value.is_none() {
        return Ok(colors);
    }
    let tag = compound(field(value, "tag"))?;
    let components = compound(field(value, "components"))?;
    let display = compound(field(tag, "display"))?;
    if id.starts_with("minecraft:leather_") || id == "minecraft:leather_horse_armor" {
        let dyed = field(components, "minecraft:dyed_color");
        let color = match dyed {
            Some(Value::Int(c)) => Some(*c),
            _ => integer(field(compound(dyed)?, "rgb"))?,
        };
        if let Some(c) = color.or(integer(field(display, "color"))?) {
            *colors.get_mut(0).ok_or("皮革物品缺少颜色注册表")? = c;
        }
    }
    if matches!(
        id,
        "minecraft:potion"
            | "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:tipped_arrow"
    ) {
        // metadata 目录查询只提供 Damage；真实物品则按其实际空标签语义读取。
        if tag.is_some() || components.is_some() || field(value, "id").is_some() {
            *colors.get_mut(0).ok_or("药水物品缺少颜色注册表")? = potion(version, tag, components)?;
        }
    }
    if id == "minecraft:filled_map" {
        if let Some(c) = integer(field(components, "minecraft:map_color"))?
            .or(integer(field(display, "MapColor"))?)
        {
            if colors.len() < 2 {
                return Err("地图缺少颜色注册表".into());
            }
            for color in colors.iter_mut().skip(1) {
                *color = c;
            }
        }
    }
    if matches!(id, "minecraft:firework_star" | "minecraft:firework_charge") {
        let modern = version == Some(3955);
        let explosion = compound(if modern {
            field(components, "minecraft:firework_explosion")
        } else {
            field(tag, "Explosion")
        })?;
        let palette = match field(explosion, if modern { "colors" } else { "Colors" }) {
            None => &[][..],
            Some(Value::IntArray(v)) if v.is_empty() && !modern => {
                return Err("旧版烟火颜色数组不能为空".into())
            }
            Some(Value::IntArray(v)) => v.as_slice(),
            _ => return Err("烟火颜色必须为 IntArray".into()),
        };
        let c = if palette.is_empty() {
            9079434
        } else {
            let mut sum = [0i64; 3];
            for c in palette {
                for (i, shift) in [16, 8, 0].into_iter().enumerate() {
                    sum[i] += i64::from((c >> shift) & 255);
                }
            }
            let c = sum.map(|n| (n / palette.len() as i64) as i32);
            (c[0] << 16) | (c[1] << 8) | c[2]
        };
        *colors.get_mut(1).ok_or("烟火缺少颜色注册表")? = c;
    }
    if id == "minecraft:spawn_egg" && version == Some(1343) {
        let entity = compound(field(tag, "EntityTag"))?;
        if let Some(entity) = string(field(entity, "id"))? {
            let entity = qualified(entity);
            let catalog = item_presets::get(version).expect("准确旧版清单");
            for preset in catalog.presets.iter().filter(|p| p.id == id) {
                let egg = field(
                    field(field(Some(preset.value()?), "tag"), "EntityTag"),
                    "id",
                );
                if string(egg)?.map(qualified).as_ref() == Some(&entity) {
                    return Ok(preset.tints.clone());
                }
            }
            return Err(format!("未知旧版刷怪蛋颜色：{entity}"));
        }
    }
    Ok(colors)
}
