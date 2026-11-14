//! 模型目录按本次不可变资源索引共享；组合部件不会重复解析同一套骨架和四角数据。
use super::ModelBank;
use mcedit2_world::entity_model::Catalog;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};
pub(super) type Catalogs = BTreeMap<String, OnceLock<Result<Arc<Catalog>, String>>>;
impl ModelBank {
    pub(super) fn layer_catalog(&self, key: &str) -> Result<Arc<Catalog>, String> {
        let sources = self.layer_catalogs.get_or_init(|| {
            self.files
                .iter()
                .filter(|(key, value)| key.starts_with("model:") && value.get("layers").is_some())
                .map(|(key, _)| (key.clone(), OnceLock::new()))
                .collect()
        });
        let entry = sources
            .get(key)
            .ok_or_else(|| format!("资源 {key} 不存在或没有声明模型层目录"))?;
        entry
            .get_or_init(|| {
                let result = (|| {
                    let value = self.files.get(key).ok_or("模型目录来源在编译期间消失")?;
                    let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
                    Catalog::from_json(&json).map(Arc::new)
                })();
                if let Err(error) = &result {
                    log::error!("[models] 模型目录 {key} 无效：{error}");
                }
                result
            })
            .clone()
    }
}
