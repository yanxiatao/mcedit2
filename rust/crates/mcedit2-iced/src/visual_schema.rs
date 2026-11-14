//! 外观定义的类型化源字段合同；缺失与损坏分开处理，数值整数边界保持 i64 精度。
use mcedit2_world::{entity_catalog::EntityVisual, nbt_document::Value};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Byte,
    Short,
    Int,
    Long,
    Float,
    Double,
    Number,
    Boolean,
    String,
    List,
    Compound,
    ByteArray,
    IntArray,
    LongArray,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Field {
    #[serde(rename = "type")]
    kind: Kind,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    properties: BTreeMap<String, Field>,
    items: Option<Box<Field>>,
    min_length: Option<usize>,
    max_length: Option<usize>,
    minimum: Option<serde_json::Number>,
    maximum: Option<serde_json::Number>,
}
pub(crate) type Fields = BTreeMap<String, Field>;
fn numeric(value: &Value) -> Option<(Option<i64>, f64)> {
    Some(match value {
        Value::Byte(v) => (Some(i64::from(*v)), f64::from(*v)),
        Value::Short(v) => (Some(i64::from(*v)), f64::from(*v)),
        Value::Int(v) => (Some(i64::from(*v)), f64::from(*v)),
        Value::Long(v) => (Some(*v), *v as f64),
        Value::Float(v) => (None, f64::from(*v)),
        Value::Double(v) => (None, *v),
        _ => return None,
    })
}
impl Field {
    fn check(&self, path: &str, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err(format!("源字段合同 {path} 超过 64 层"));
        }
        let collection = matches!(
            self.kind,
            Kind::String
                | Kind::List
                | Kind::Compound
                | Kind::ByteArray
                | Kind::IntArray
                | Kind::LongArray
        );
        if !collection && (self.min_length.is_some() || self.max_length.is_some()) {
            return Err(format!("{path} 的类型不支持长度约束"));
        }
        if self
            .min_length
            .zip(self.max_length)
            .is_some_and(|(min, max)| min > max)
        {
            return Err(format!("{path} 的最小长度超过最大长度"));
        }
        if !matches!(self.kind, Kind::List) && self.items.is_some() {
            return Err(format!("{path} 只有 List 可以声明 items"));
        }
        if !matches!(self.kind, Kind::Compound) && !self.properties.is_empty() {
            return Err(format!("{path} 只有 Compound 可以声明 properties"));
        }
        let number = matches!(
            self.kind,
            Kind::Byte
                | Kind::Short
                | Kind::Int
                | Kind::Long
                | Kind::Float
                | Kind::Double
                | Kind::Number
        );
        if !number && (self.minimum.is_some() || self.maximum.is_some()) {
            return Err(format!("{path} 的类型不支持数值边界"));
        }
        if matches!(self.kind, Kind::Byte | Kind::Short | Kind::Int | Kind::Long)
            && [&self.minimum, &self.maximum]
                .into_iter()
                .flatten()
                .any(|value| value.as_i64().is_none())
        {
            return Err(format!("{path} 的整数字段边界必须为有符号 64 位整数"));
        }
        if let (Some(min), Some(max)) = (&self.minimum, &self.maximum) {
            let inverted = match (min.as_i64(), max.as_i64()) {
                (Some(min), Some(max)) => min > max,
                _ => min.as_f64().ok_or("数值下界无效")? > max.as_f64().ok_or("数值上界无效")?,
            };
            if inverted {
                return Err(format!("{path} 的下界超过上界"));
            }
        }
        if let Some(item) = &self.items {
            item.check(&format!("{path}[]"), depth + 1)?;
        }
        for (name, field) in &self.properties {
            field.check(&format!("{path}.{name}"), depth + 1)?;
        }
        Ok(())
    }
    fn validate(&self, value: &Value, path: &str, depth: usize) -> Result<(), String> {
        if depth > 64 {
            return Err(format!("源字段 {path} 超过 64 层"));
        }
        let matches = match (self.kind, value) {
            (Kind::Byte, Value::Byte(_))
            | (Kind::Short, Value::Short(_))
            | (Kind::Int, Value::Int(_))
            | (Kind::Long, Value::Long(_))
            | (Kind::Float, Value::Float(_))
            | (Kind::Double, Value::Double(_))
            | (Kind::String, Value::String(_))
            | (Kind::List, Value::List(_))
            | (Kind::Compound, Value::Compound(_))
            | (Kind::ByteArray, Value::ByteArray(_))
            | (Kind::IntArray, Value::IntArray(_))
            | (Kind::LongArray, Value::LongArray(_)) => true,
            (Kind::Number, _) => numeric(value).is_some(),
            (Kind::Boolean, Value::Byte(v)) => matches!(*v, 0 | 1),
            _ => false,
        };
        if !matches {
            return Err(format!(
                "{path} 应为 {:?}，实际 NBT 类型为 {}",
                self.kind,
                value.kind()
            ));
        }
        if let Some((integer, number)) = numeric(value) {
            if !number.is_finite() {
                return Err(format!("{path} 包含非有限数"));
            }
            for (bound, lower) in [(&self.minimum, true), (&self.maximum, false)] {
                if let Some(bound) = bound {
                    let outside = if let Some(integer) = integer {
                        let bound = bound
                            .as_i64()
                            .ok_or_else(|| format!("{path} 的整数边界必须为 i64"))?;
                        if lower {
                            integer < bound
                        } else {
                            integer > bound
                        }
                    } else {
                        let bound = bound
                            .as_f64()
                            .filter(|n| n.is_finite())
                            .ok_or("浮点边界无效")?;
                        if lower {
                            number < bound
                        } else {
                            number > bound
                        }
                    };
                    if outside {
                        return Err(format!("{path} 超出声明的数值边界"));
                    }
                }
            }
        }
        let length = match value {
            Value::String(value) => Some(value.chars().count()),
            Value::List(value) => Some(value.items.len()),
            Value::Compound(value) => Some(value.len()),
            Value::ByteArray(value) => Some(value.len()),
            Value::IntArray(value) => Some(value.len()),
            Value::LongArray(value) => Some(value.len()),
            _ => None,
        };
        if let Some(length) = length {
            if self.min_length.is_some_and(|min| length < min)
                || self.max_length.is_some_and(|max| length > max)
            {
                return Err(format!("{path} 的长度 {length} 超出声明范围"));
            }
        }
        if let (Value::List(list), Some(schema)) = (value, &self.items) {
            for (index, item) in list.items.iter().enumerate() {
                schema.validate(item, &format!("{path}[{index}]"), depth + 1)?;
            }
        }
        if let Value::Compound(fields) = value {
            let mut values = BTreeMap::new();
            for field in fields {
                if values.insert(field.name.as_str(), &field.value).is_some() {
                    return Err(format!("{path}.{} 重复", field.name));
                }
            }
            for (name, schema) in &self.properties {
                match values.get(name.as_str()) {
                    Some(value) => schema.validate(value, &format!("{path}.{name}"), depth + 1)?,
                    None if schema.optional => {}
                    None => return Err(format!("{path}.{name} 缺失")),
                }
            }
        }
        Ok(())
    }
}
pub(crate) fn check(fields: &Fields) -> Result<(), String> {
    for (name, field) in fields {
        field.check(&format!("NBT.{name}"), 0)?;
    }
    Ok(())
}
pub(crate) fn validate(fields: &Fields, visual: &EntityVisual) -> Result<(), String> {
    for (name, schema) in fields {
        match visual.source_field_optional(name)? {
            Some(value) => schema.validate(&value, &format!("NBT.{name}"), 0)?,
            None if schema.optional => {}
            None => return Err(format!("源 NBT 字段 {name} 缺失")),
        }
    }
    Ok(())
}
