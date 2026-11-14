//! 狼的实际外观注册表；资源身份来自客户端与世界启用的数据包。
use crate::{
    nbt_document::Value as Nbt,
    world_data::{apply_registry_layers, RegistryLayer},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug, Deserialize)]
struct Variant {
    wild_texture: String,
    tame_texture: String,
    angry_texture: String,
}
impl Variant {
    fn checked(mut self) -> Result<Self, String> {
        for id in [
            &mut self.wild_texture,
            &mut self.tame_texture,
            &mut self.angry_texture,
        ] {
            if !id.contains(':') {
                *id = format!("minecraft:{id}");
            }
            crate::world_data::registry_id(id).map_err(|error| error.to_string())?;
        }
        Ok(self)
    }
}
#[derive(Clone, Debug)]
pub struct Catalog {
    variants: BTreeMap<String, Variant>,
}
impl Catalog {
    pub fn textures(&self) -> impl Iterator<Item = &str> {
        self.variants
            .values()
            .flat_map(|value| {
                [
                    &value.wild_texture,
                    &value.tame_texture,
                    &value.angry_texture,
                ]
            })
            .map(String::as_str)
    }
    pub fn builtin() -> Result<Arc<Self>, String> {
        static CATALOG: OnceLock<Result<Arc<Catalog>, String>> = OnceLock::new();
        CATALOG
            .get_or_init(|| {
                #[derive(Deserialize)]
                struct Data {
                    data_version: i32,
                    variants: BTreeMap<String, Variant>,
                }
                let source: Data =
                    serde_json::from_str(include_str!("../assets/wolf-variants-1.21.1.json"))
                        .map_err(|error| error.to_string())?;
                if source.data_version != 3955 {
                    return Err("狼外观定义与源版本不同".into());
                }
                Ok(Arc::new(Self {
                    variants: source
                        .variants
                        .into_iter()
                        .map(|(id, value)| {
                            crate::world_data::registry_id(&id)
                                .map_err(|error| error.to_string())?;
                            Ok((id, value.checked()?))
                        })
                        .collect::<Result<_, String>>()?,
                }))
            })
            .clone()
    }
    pub fn with_layers(layers: &[RegistryLayer]) -> Result<Arc<Self>, String> {
        let base = Self::builtin()?;
        let variants = apply_registry_layers(
            "wolf_variant",
            base.variants.clone(),
            layers,
            |id, bytes| {
                let value: Variant = serde_json::from_slice(bytes)
                    .map_err(|error| format!("狼外观 {id}：{error}"))?;
                value.checked()
            },
        )?;
        Ok(Arc::new(Self { variants }))
    }
    pub fn texture(&self, source: Option<&Nbt>, tame: bool, angry: bool) -> Result<&str, String> {
        // Wolf 构造时绑定 PALE；存在但未知的定义不套用该默认值。
        let id = match source {
            None => "minecraft:pale",
            Some(Nbt::String(value)) => value,
            _ => return Err("狼 variant 必须为注册字符串".into()),
        };
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        let value = self
            .variants
            .get(&id)
            .ok_or_else(|| format!("狼外观 {id} 没有实际注册定义"))?;
        Ok(if tame {
            &value.tame_texture
        } else if angry {
            &value.angry_texture
        } else {
            &value.wild_texture
        })
    }
}
