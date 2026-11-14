//! 无持有者 / 世界的原版物品属性和附魔光效身份，读取实际物品载荷。
use crate::{
    item_icons::{field, number},
    item_presets,
};
use mcedit2_world::nbt_document::Value;
use std::collections::BTreeMap;
fn list(value: Option<&Value>) -> Result<&[Value], String> {
    match value {
        None => Ok(&[]),
        Some(Value::List(v)) => Ok(&v.items),
        _ => Err("物品属性列表类型无效".into()),
    }
}
fn int(value: Option<&Value>, default: i32) -> Result<i32, String> {
    match value {
        None => Ok(default),
        Some(v) => number(Some(v)).ok_or("物品属性数字类型无效".into()),
    }
}
fn text(value: Option<&Value>) -> Result<Option<&str>, String> {
    match value {
        None => Ok(None),
        Some(Value::String(v)) => Ok(Some(v)),
        _ => Err("物品属性字符串类型无效".into()),
    }
}
fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.into()
    } else {
        format!("minecraft:{id}")
    }
}
pub(crate) fn foil(
    id: &str,
    version: Option<i32>,
    value: Option<&Value>,
    damage: i32,
) -> Result<bool, String> {
    let catalog = item_presets::get(version).ok_or("物品光效没有准确版本数据")?;
    let tag = field(value, "tag");
    let components = field(value, "components");
    if version == Some(3955) {
        if let Some(v) = field(components, "minecraft:enchantment_glint_override") {
            return match v {
                Value::Byte(0) => Ok(false),
                Value::Byte(1) => Ok(true),
                _ => Err("光效覆盖必须为 Boolean Byte".into()),
            };
        }
    }
    if version == Some(1343) && id == "minecraft:golden_apple" {
        return Ok(damage > 0);
    }
    if version == Some(1343)
        && matches!(
            id,
            "minecraft:potion" | "minecraft:splash_potion" | "minecraft:lingering_potion"
        )
    {
        if value.is_none() {
            return Ok(catalog.default_item(id).ok_or("药水缺少默认物品")?.foil);
        }
        let potion = text(field(tag, "Potion"))?.map(qualified);
        let base = match potion {
            None => false,
            Some(id) => !catalog
                .potions
                .get(&id)
                .ok_or_else(|| format!("未知药水 {id}"))?
                .is_empty(),
        };
        return Ok(base || !list(field(tag, "CustomPotionEffects"))?.is_empty());
    }
    if catalog.default_item(id).is_some_and(|p| p.foil) {
        return Ok(true);
    }
    if id == "minecraft:compass" && version != Some(1343) {
        let lodestone = if version == Some(3955) {
            field(components, "minecraft:lodestone_tracker").is_some()
        } else {
            field(tag, "LodestonePos").is_some() && field(tag, "LodestoneDimension").is_some()
        };
        if lodestone {
            return Ok(true);
        }
    }
    if version == Some(3955) {
        let enchantments = field(components, "minecraft:enchantments");
        let levels = field(enchantments, "levels").or(enchantments);
        return match levels {
            None => Ok(false),
            Some(Value::Compound(v)) => Ok(!v.is_empty()),
            _ => Err("附魔组件必须为 Compound".into()),
        };
    }
    Ok(!list(field(
        tag,
        if version == Some(1343) {
            "ench"
        } else {
            "Enchantments"
        },
    ))?
    .is_empty())
}
fn contents(value: Option<&Value>, modern: bool) -> Result<&[Value], String> {
    list(if modern {
        field(field(value, "components"), "minecraft:bundle_contents")
    } else {
        field(field(value, "tag"), "Items")
    })
}
fn weight(items: &[Value], version: Option<i32>) -> Result<f64, String> {
    let modern = version == Some(3955);
    let catalog = item_presets::get(version).ok_or("收纳袋缺少版本清单")?;
    let mut sum = 0.;
    for item in items {
        let id = qualified(text(field(Some(item), "id"))?.ok_or("收纳袋物品缺少 id")?);
        let context = field(Some(item), if modern { "components" } else { "tag" });
        let nested = if modern {
            field(context, "minecraft:bundle_contents").is_some()
        } else {
            id == "minecraft:bundle"
        };
        let unit = if nested {
            0.0625 + weight(contents(Some(item), modern)?, version)?
        } else {
            let bees = if modern {
                field(context, "minecraft:bees")
            } else {
                field(field(context, "BlockEntityTag"), "Bees")
            };
            if !list(bees)?.is_empty() {
                1.
            } else {
                let default = catalog
                    .default_item(&id)
                    .ok_or_else(|| format!("收纳袋物品 {id} 缺少堆叠上限"))?
                    .max_count as i32;
                let max = if modern {
                    int(field(context, "minecraft:max_stack_size"), default)?
                } else {
                    default
                };
                if max <= 0 {
                    return Err("收纳袋物品堆叠上限必须为正数".into());
                }
                if modern {
                    1. / f64::from(max)
                } else {
                    f64::from(64 / max) / 64.
                }
            }
        };
        let count = int(field(Some(item), if modern { "count" } else { "Count" }), 1)?;
        sum += unit * f64::from(count);
    }
    Ok(sum)
}
pub(crate) fn properties(
    id: &str,
    version: Option<i32>,
    value: Option<&Value>,
    damage: i32,
    model_data: i32,
) -> Result<BTreeMap<String, u32>, String> {
    let catalog = item_presets::get(version).ok_or("物品属性没有准确版本数据")?;
    let default = catalog.default_item(id);
    let generic = catalog
        .default_item("minecraft:stone")
        .expect("内置物品清单含石头");
    let mut properties = default.unwrap_or(generic).properties.clone();
    let tag = field(value, "tag");
    let components = field(value, "components");
    let modern = version == Some(3955);
    let max = if modern {
        if field(components, "!minecraft:max_damage").is_some() {
            0
        } else {
            int(
                field(components, "minecraft:max_damage"),
                default.map_or(0, |p| p.max_damage as i32),
            )?
        }
    } else {
        default.map_or(0, |p| p.max_damage as i32)
    };
    let unbreakable = if modern {
        field(components, "minecraft:unbreakable").is_some()
    } else {
        int(field(tag, "Unbreakable"), 0)? != 0
    };
    if max > 0 {
        let ratio = (damage as f32 / max as f32).clamp(0., 1.);
        properties.insert("damage".into(), ratio);
        properties.insert(
            "damaged".into(),
            if damage > 0 && !unbreakable { 1. } else { 0. },
        );
    } else {
        properties.remove("damage");
        properties.remove("damaged");
    }
    if version != Some(1343) {
        properties.insert("custom_model_data".into(), model_data as f32);
    }
    if properties.contains_key("broken") {
        properties.insert("broken".into(), if damage >= max - 1 { 1. } else { 0. });
    }
    if properties.contains_key("charged") {
        let projectiles = list(if modern {
            field(components, "minecraft:charged_projectiles")
        } else {
            field(tag, "ChargedProjectiles")
        })?;
        let charged = if modern {
            !projectiles.is_empty()
        } else {
            int(field(tag, "Charged"), 0)? != 0
        };
        let fireworks=projectiles.iter().any(|v|matches!(field(Some(v),"id"),Some(Value::String(id)) if id=="minecraft:firework_rocket"));
        properties.insert("charged".into(), if charged { 1. } else { 0. });
        properties.insert(
            "firework".into(),
            if fireworks && (modern || charged) {
                1.
            } else {
                0.
            },
        );
    }
    if properties.contains_key("filled") {
        properties.insert(
            "filled".into(),
            (weight(contents(value, modern)?, version)? as f32).clamp(0., 1.),
        );
    }
    if properties.contains_key("level") {
        let state = if modern {
            field(components, "minecraft:block_state")
        } else {
            field(tag, "BlockStateTag")
        };
        let level = match text(field(state, "level"))? {
            Some(s) => s.parse::<i32>().map_err(|e| e.to_string())? as f32 / 16.,
            None => 1.,
        };
        properties.insert("level".into(), level.clamp(0., 1.));
    }
    if modern && properties.contains_key("trim_type") {
        if let Some(trim) = field(components, "minecraft:trim") {
            // 原版 Holder 支持已注册名称，也支持完整内联材料。
            let material = field(Some(trim), "material").ok_or("盔甲纹饰缺少材料")?;
            let index = match material {
                Value::String(id) => {
                    let name = id.strip_prefix("minecraft:").unwrap_or(id);
                    match name {
                        "quartz" => 0.1,
                        "iron" => 0.2,
                        "netherite" => 0.3,
                        "redstone" => 0.4,
                        "copper" => 0.5,
                        "gold" => 0.6,
                        "emerald" => 0.7,
                        "diamond" => 0.8,
                        "lapis" => 0.9,
                        "amethyst" => 1.,
                        _ => return Err(format!("未知盔甲纹饰材料 {id}")),
                    }
                }
                Value::Compound(_) => match field(Some(material), "item_model_index") {
                    Some(Value::Float(value)) => *value,
                    Some(Value::Double(value)) => *value as f32,
                    Some(Value::Long(value)) => *value as f32,
                    value => number(value).ok_or("内联纹饰材料缺少 item_model_index 数值")? as f32,
                },
                _ => return Err("盔甲纹饰材料类型无效".into()),
            };
            properties.insert("trim_type".into(), index.clamp(0., 1.));
        }
    }
    Ok(properties
        .into_iter()
        .map(|(key, value)| (key, value.to_bits()))
        .collect())
}
