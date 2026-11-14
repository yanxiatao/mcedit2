//! 图案 Holder 的实际定义；旧版资源别名来自客户端导出，世界覆盖只作用于动态注册表。
use super::{registry_id, PatternReference};
use crate::world_data::RegistryLayer;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Definition {
    pub asset_id: String,
    pub translation_key: Option<String>,
    pub legacy_hash: Option<String>,
}
impl Definition {
    fn validate(mut self) -> Result<Self, String> {
        self.asset_id = registry_id(&self.asset_id)?;
        crate::world_data::registry_id(&self.asset_id).map_err(|e| e.to_string())?;
        Ok(self)
    }
    fn world(bytes: &[u8]) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Modern {
            asset_id: String,
            translation_key: String,
        }
        let value: Modern = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        Self {
            asset_id: value.asset_id,
            translation_key: Some(value.translation_key),
            legacy_hash: None,
        }
        .validate()
    }
}
#[derive(Debug, Clone)]
pub struct Catalog(pub BTreeMap<String, Definition>);
impl Catalog {
    pub fn builtin(version: i32) -> Result<Arc<Self>, String> {
        static OLD: OnceLock<Arc<Catalog>> = OnceLock::new();
        static MID: OnceLock<Arc<Catalog>> = OnceLock::new();
        static NEW: OnceLock<Arc<Catalog>> = OnceLock::new();
        let (slot, bytes): (&OnceLock<Arc<Catalog>>, &[u8]) = match version {
            1343 => (
                &OLD,
                include_bytes!("../assets/banner-patterns-1.12.2.json"),
            ),
            3465 => (
                &MID,
                include_bytes!("../assets/banner-patterns-1.20.1.json"),
            ),
            3955 => (
                &NEW,
                include_bytes!("../assets/banner-patterns-1.21.1.json"),
            ),
            _ => return Err(format!("尚未核对 DataVersion {version} 的旗帜图案注册表")),
        };
        Ok(Arc::clone(slot.get_or_init(|| {
            let raw: BTreeMap<String, Definition> =
                serde_json::from_slice(bytes).expect("内建图案目录必须有效");
            let mut values = BTreeMap::new();
            for (id, value) in raw {
                crate::world_data::registry_id(&id).expect("内建图案 ID 必须有效");
                if let Some(hash) = &value.legacy_hash {
                    assert_eq!(
                        super::legacy_pattern(hash).expect("原生旧图案缩写有格式映射"),
                        id
                    );
                }
                assert!(
                    values
                        .insert(id, value.validate().expect("内建图案资源必须有效"))
                        .is_none(),
                    "内建图案 ID 重复"
                );
            }
            Arc::new(Self(values))
        })))
    }
    pub fn with_layers(layers: &[RegistryLayer]) -> Result<Arc<Self>, String> {
        if layers.is_empty() {
            return Self::builtin(3955);
        }
        let values = crate::world_data::apply_registry_layers(
            "banner_pattern",
            Self::builtin(3955)?.0.clone(),
            layers,
            |id, bytes| Definition::world(bytes).map_err(|e| format!("旗帜图案定义 {id}：{e}")),
        )?;
        Ok(Arc::new(Self(values)))
    }
    pub fn resolve(&self, pattern: &PatternReference) -> Result<Definition, String> {
        match pattern {
            PatternReference::Registry(id) => {
                self.0.get(id).cloned().ok_or_else(|| {
                    format!("缺少旗帜图案注册数据 {id}，不能从同名 PNG 猜测 asset_id")
                })
            }
            PatternReference::Inline {
                asset_id,
                translation_key,
            } => Definition {
                asset_id: asset_id.clone(),
                translation_key: Some(translation_key.clone()),
                legacy_hash: None,
            }
            .validate(),
        }
    }
}
