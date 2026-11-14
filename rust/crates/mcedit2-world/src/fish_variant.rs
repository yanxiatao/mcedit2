//! 两版实际 TropicalFish.Variant 的四个字节；不将未知编码替换为其他鱼。
use crate::{entity_catalog::EntityVisual, nbt_document::Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Small,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tropical {
    pub shape: Shape,
    pub pattern: u8,
    pub base_color: u8,
    pub pattern_color: u8,
}
impl Tropical {
    pub fn read(visual: &EntityVisual) -> Result<Self, String> {
        if !matches!(visual.source_version, Some(3465 | 3955)) {
            return Err(format!("尚未核实 {:?} 的热带鱼编码", visual.source_version));
        }
        let packed = match visual.source_field_optional("Variant")? {
            None => 0,
            Some(Value::Int(value)) => value as u32,
            _ => return Err("热带鱼 Variant 必须为 Int".into()),
        };
        let shape = match packed & 255 {
            0 => Shape::Small,
            1 => Shape::Large,
            value => return Err(format!("热带鱼体型编码 {value} 缺少实际定义")),
        };
        let pattern = ((packed >> 8) & 255) as u8;
        let base_color = ((packed >> 16) & 255) as u8;
        let pattern_color = (packed >> 24) as u8;
        if pattern > 5 {
            return Err(format!("热带鱼花纹编码 {pattern} 缺少实际定义"));
        }
        if base_color > 15 || pattern_color > 15 {
            return Err(format!(
                "热带鱼底色 {base_color} / 花纹色 {pattern_color} 缺少实际染料定义"
            ));
        }
        Ok(Self {
            shape,
            pattern,
            base_color,
            pattern_color,
        })
    }
}
