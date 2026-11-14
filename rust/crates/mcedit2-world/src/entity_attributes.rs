//! 保存属性的版本化解码与实际修饰运算；缺省值与属性范围由使用该属性的原生类型决定。
use crate::{entity_catalog::EntityVisual, nbt_document::Value};
use std::collections::BTreeSet;

/// 原生 generic.scale 的默认值和注册范围；身体、装备与眼位共用同一属性运算。
pub fn scale(visual: &EntityVisual) -> Result<f32, String> {
    if visual.source_version != Some(3955) {
        return Err("当前源版本没有已绑定的 generic.scale 属性".into());
    }
    Ok(value(visual, "generic.scale", "generic.scale")?
        .unwrap_or(1.)
        .clamp(0.0625, 16.) as f32)
}
/// 原生最大生命值属性的运算、范围及 float 结果；默认基值由实际实体类型提供。
pub fn max_health(visual: &EntityVisual, default: f64) -> Result<f32, String> {
    Ok(value(visual, "generic.max_health", "generic.maxHealth")?
        .unwrap_or(default)
        .clamp(1., 1024.) as f32)
}
/// LivingEntity 保存 Health 接受 NumericTag，再按已解释的实际最大生命值约束。
pub fn health(visual: &EntityVisual, maximum: f32) -> Result<f32, String> {
    let health = match visual.source_field_optional("Health")? {
        None => maximum,
        Some(Value::Byte(value)) => f32::from(value),
        Some(Value::Short(value)) => f32::from(value),
        Some(Value::Int(value)) => value as f32,
        Some(Value::Long(value)) => value as f32,
        Some(Value::Float(value)) => value,
        Some(Value::Double(value)) => value as f32,
        _ => return Err("生物 Health 必须为数值".into()),
    };
    Ok(health.clamp(0., maximum))
}

fn field<'a>(value: &'a Value, key: &str) -> Result<Option<&'a Value>, String> {
    let Value::Compound(fields) = value else {
        return Err("实体属性必须为 Compound".into());
    };
    let mut matching = fields.iter().filter(|field| field.name == key);
    let result = matching.next().map(|field| &field.value);
    if matching.next().is_some() {
        return Err(format!("实体属性字段 {key} 重复"));
    }
    Ok(result)
}
fn double(value: Option<&Value>, key: &str) -> Result<f64, String> {
    match value {
        Some(Value::Double(value)) if value.is_finite() => Ok(*value),
        _ => Err(format!("实体属性 {key} 必须为有限 Double")),
    }
}
pub fn value(
    visual: &EntityVisual,
    modern_id: &str,
    legacy_id: &str,
) -> Result<Option<f64>, String> {
    let version = visual.source_version.ok_or("实体属性缺少实际版本")?;
    let modern = match version {
        3955 => true,
        1343 | 3465 => false,
        _ => return Err(format!("未绑定版本 {version} 的实体属性格式")),
    };
    let key = if modern { "attributes" } else { "Attributes" };
    let Some(attributes) = visual.appearance.get(key) else {
        return Ok(None);
    };
    let Value::List(list) = attributes else {
        return Err(format!("{key} 必须为列表"));
    };
    let id_key = if modern { "id" } else { "Name" };
    let wanted = if version == 1343 {
        legacy_id
    } else {
        modern_id
    };
    let mut found = None;
    for attribute in &list.items {
        let Some(Value::String(id)) = field(attribute, id_key)? else {
            return Err(format!("属性 {id_key} 缺失或不是 String"));
        };
        if id.strip_prefix("minecraft:").unwrap_or(id)
            != wanted.strip_prefix("minecraft:").unwrap_or(wanted)
        {
            continue;
        }
        if found.is_some() {
            return Err(format!("实体属性 {wanted} 重复"));
        }
        found = Some(attribute);
    }
    let Some(attribute) = found else {
        return Ok(None);
    };
    let base_key = if modern { "base" } else { "Base" };
    let mut base = double(field(attribute, base_key)?, base_key)?;
    let mods_key = if modern { "modifiers" } else { "Modifiers" };
    let mut added = 0.;
    let mut multiplied = Vec::new();
    let mut total = Vec::new();
    if let Some(modifiers) = field(attribute, mods_key)? {
        let Value::List(list) = modifiers else {
            return Err(format!("属性 {mods_key} 必须为列表"));
        };
        let mut ids = BTreeSet::new();
        for modifier in &list.items {
            let id = if modern {
                let Some(Value::String(id)) = field(modifier, "id")? else {
                    return Err("属性修饰缺少 id".into());
                };
                id.clone()
            } else if version == 3465 {
                let Some(Value::IntArray(words)) = field(modifier, "UUID")? else {
                    return Err("属性修饰缺少 UUID".into());
                };
                if words.len() != 4 {
                    return Err("属性修饰 UUID 必须为四项 IntArray".into());
                }
                format!("{words:?}")
            } else {
                let (Some(Value::Long(most)), Some(Value::Long(least))) =
                    (field(modifier, "UUIDMost")?, field(modifier, "UUIDLeast")?)
                else {
                    return Err("旧属性修饰缺少 UUIDMost / UUIDLeast".into());
                };
                format!("{most}:{least}")
            };
            if !ids.insert(id) {
                return Err("实体属性修饰身份重复".into());
            }
            let amount = double(
                field(modifier, if modern { "amount" } else { "Amount" })?,
                "amount",
            )?;
            let operation = match field(modifier, if modern { "operation" } else { "Operation" })? {
                Some(Value::String(operation)) if modern => match operation.as_str() {
                    "add_value" => 0,
                    "add_multiplied_base" => 1,
                    "add_multiplied_total" => 2,
                    _ => return Err(format!("属性修饰运算 {operation} 未识别")),
                },
                Some(Value::Int(operation)) if !modern && (0..=2).contains(operation) => *operation,
                _ => return Err("属性修饰运算与版本格式不符".into()),
            };
            match operation {
                0 => added += amount,
                1 => multiplied.push(amount),
                2 => total.push(amount),
                _ => unreachable!(),
            }
        }
    }
    base += added;
    let mut result = base;
    for amount in multiplied {
        result += base * amount;
    }
    for amount in total {
        result *= 1. + amount;
    }
    if !result.is_finite() {
        return Err(format!("实体属性 {wanted} 计算溢出"));
    }
    Ok(Some(result))
}
