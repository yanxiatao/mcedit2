//! 用户明确指定的方块模型绑定；来源状态和模型别名分开，不改存档或注册 ID。
use mcedit2_world::{
    editing::{parse_state, render_state},
    sections::State,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    /// None 为用户显式选择的全局配置，Some 为实际世界或独立结构文件。
    pub scope: Option<PathBuf>,
    pub state: State,
    pub all_states: bool,
    pub enabled: bool,
    pub model: String,
    pub rotation: [u16; 2],
    pub uvlock: bool,
    pub textures: BTreeMap<String, String>,
    /// 旧编辑器保存但未参与 forcedModel 的字段只保留原文，不冒充生效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<serde_json::Value>,
}
impl Binding {
    pub fn validate(&self) -> Result<(), String> {
        if self.scope.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err("方块定义的文档路径必须为绝对路径".into());
        }
        if parse_state(&render_state(&self.state)).map_err(|error| error.to_string())? != self.state
        {
            return Err("方块定义的实际状态不是规范形式".into());
        }
        if self.all_states && self.state.name.starts_with("numeric:") {
            return Err("数字方块定义须明确指定 Data，不能用名称通配元数据".into());
        }
        if self.all_states && !self.state.properties.is_empty() {
            return Err("按名称匹配时不要同时指定状态属性".into());
        }
        if self
            .rotation
            .iter()
            .any(|value| !matches!(*value, 0 | 90 | 180 | 270))
        {
            return Err("模型 X / Y 旋转必须为 0、90、180 或 270".into());
        }
        if !self.enabled {
            return Ok(());
        }
        if crate::resource_models::normalize_model(&self.model).is_none() {
            return Err("方块定义的模型资源 ID 无效".into());
        }
        let mut aliases = BTreeSet::new();
        for (alias, texture) in &self.textures {
            let alias = alias.strip_prefix('#').unwrap_or(alias);
            if !aliases.insert(alias) {
                return Err(format!("重复纹理变量：{alias}"));
            }
            if alias.is_empty()
                || !alias
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                return Err("纹理变量名无效".into());
            }
            if crate::resource_models::normalize_texture(texture).is_none() {
                return Err(format!("纹理资源 ID 无效：{texture}"));
            }
        }
        Ok(())
    }
    pub fn selector(&self) -> String {
        if self.all_states {
            format!("{}[*]", self.state.name)
        } else {
            render_state(&self.state)
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub version: u32,
    pub bindings: Vec<Binding>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            bindings: Vec::new(),
        }
    }
}
impl Settings {
    pub fn import(
        bytes: &[u8],
        scope: Option<PathBuf>,
        registry: Option<&crate::legacy_render::Registry>,
    ) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| format!("方块定义 JSON 无效：{e}"))?;
        let settings = if let Some(entries) = value.as_array() {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Legacy {
                internal_name: String,
                #[serde(default)]
                meta: u16,
                model_path: Option<String>,
                #[serde(default)]
                model_rotations: Option<[u16; 2]>,
                #[serde(default)]
                model_textures: BTreeMap<String, String>,
            }
            fn location(value: &str, folder: &str, extension: &str) -> Result<String, String> {
                if let Some(relative) = value.strip_prefix("assets/") {
                    let (namespace, path) =
                        relative.split_once('/').ok_or("旧资源路径缺少命名空间")?;
                    let path = path
                        .strip_prefix(folder)
                        .and_then(|path| path.strip_suffix(extension))
                        .ok_or("旧资源路径类型无效")?;
                    Ok(format!("{namespace}:{path}"))
                } else {
                    Ok(value.to_owned())
                }
            }
            let mut bindings = Vec::new();
            for entry in entries {
                let old: Legacy = serde_json::from_value(entry.clone())
                    .map_err(|e| format!("旧方块定义无效：{e}"))?;
                let registry=registry.ok_or("旧 defined_blocks.json 需要实际旧版数字注册表，请载入对应客户端；不会猜测名称与数值 ID 的对应关系")?;
                let state = registry.named_numeric_state(&old.internal_name, old.meta)?;
                bindings.push(Binding {
                    scope: scope.clone(),
                    state,
                    all_states: false,
                    enabled: old.model_path.is_some(),
                    model: old
                        .model_path
                        .as_deref()
                        .map(|path| location(path, "models/", ".json"))
                        .transpose()?
                        .unwrap_or_default(),
                    rotation: old.model_rotations.unwrap_or([0, 0]),
                    uvlock: false,
                    textures: old
                        .model_textures
                        .into_iter()
                        .map(|(alias, path)| Ok((alias, location(&path, "textures/", ".png")?)))
                        .collect::<Result<_, String>>()?,
                    legacy: Some(entry.clone()),
                });
            }
            Self {
                version: 1,
                bindings,
            }
        } else {
            serde_json::from_value(value).map_err(|e| format!("方块定义格式无效：{e}"))?
        };
        settings.validate()?;
        Ok(settings)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err(format!("不支持方块定义版本 {}", self.version));
        }
        let mut unique = BTreeSet::new();
        for binding in &self.bindings {
            binding.validate()?;
            if !unique.insert((binding.scope.clone(), binding.selector())) {
                return Err(format!("方块定义重复：{}", binding.selector()));
            }
        }
        Ok(())
    }
    pub fn for_source(&self, source: Option<&Path>) -> std::sync::Arc<Vec<Binding>> {
        let mut entries = BTreeMap::new();
        for scoped in [false, true] {
            for binding in &self.bindings {
                if binding.enabled
                    && binding.scope.is_some() == scoped
                    && (!scoped || binding.scope.as_deref() == source)
                {
                    entries.insert(binding.selector(), binding.clone());
                }
            }
        }
        std::sync::Arc::new(entries.into_values().collect())
    }
    pub fn load() -> Result<Self, String> {
        let prefs = mcedit2_world::launch::read_rust_prefs().map_err(|e| e.to_string())?;
        let value = prefs.get("block_definitions").cloned();
        let settings = match value {
            Some(value) => {
                serde_json::from_value(value).map_err(|e| format!("方块定义设置无效：{e}"))?
            }
            None => Self::default(),
        };
        settings.validate()?;
        Ok(settings)
    }
}
