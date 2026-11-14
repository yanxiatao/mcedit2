//! 用户 forcedModel 复用正常 Java 父链、纹理、UV、依赖裁取和网格选取。
use super::*;
use crate::block_definitions::Binding;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub(crate) struct TextureSlot {
    pub alias: String,
    pub authored: Option<String>,
    pub effective: Result<Option<String>, String>,
    pub overridden: bool,
}
fn resolve_slot(
    textures: &serde_json::Map<String, Value>,
    alias: &str,
) -> Result<Option<String>, String> {
    if alias.is_empty() {
        return Err("纹理变量名为空".into());
    }
    let mut raw = format!("#{alias}");
    let mut visited = BTreeSet::new();
    while let Some(name) = raw.strip_prefix('#') {
        if !visited.insert(name.to_owned()) {
            return Err(format!("纹理变量循环：#{alias}"));
        }
        let Some(value) = textures.get(name) else {
            return Ok(None);
        };
        raw = value
            .as_str()
            .ok_or_else(|| format!("纹理变量 #{name} 不是字符串"))?
            .to_owned();
    }
    normalize_texture(&raw)
        .map(Some)
        .ok_or_else(|| format!("纹理资源 ID 无效：{raw}"))
}

#[derive(Debug, Clone, Default)]
pub(super) struct Configured {
    exact: std::collections::HashMap<State, Value>,
    names: BTreeMap<String, Value>,
    bytes: usize,
}
impl Configured {
    fn get(&self, state: &State) -> Option<&Value> {
        self.exact
            .get(state)
            .or_else(|| self.names.get(&state.name))
    }
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(super) fn choices(&self) -> impl Iterator<Item = &Value> {
        self.exact.values().chain(self.names.values())
    }
    pub(super) fn states(&self) -> impl Iterator<Item = &State> {
        self.exact.keys()
    }
    pub(super) fn names(&self) -> impl Iterator<Item = &String> {
        self.names.keys()
    }
}
impl ModelBank {
    /// 配置页复用渲染父链和覆盖顺序，列出根模型变量；独立组合子模型不混入父纹理空间。
    pub(crate) fn definition_texture_slots(
        &self,
        model: &str,
        overrides: &BTreeMap<String, String>,
    ) -> Result<Vec<TextureSlot>, String> {
        let key = normalize_model(model).ok_or("模型资源 ID 无效")?;
        let document = self
            .files
            .get(&key)
            .ok_or_else(|| format!("模型资源不存在：{model}"))?;
        let parents = graph::parent_chain(&self.files, document, &key, &BTreeSet::new())?;
        let inherited = graph::merged_textures(&parents.chain)?;
        let mut textures = inherited.clone();
        let mut explicit = BTreeSet::new();
        for (alias, value) in overrides {
            let alias = alias.strip_prefix('#').unwrap_or(alias);
            if alias.is_empty() || !explicit.insert(alias.to_owned()) {
                return Err(format!("纹理覆盖变量为空或重复：{alias}"));
            }
            textures.insert(alias.into(), Value::String(value.clone()));
        }
        let mut aliases = textures.keys().cloned().collect::<BTreeSet<_>>();
        for value in textures.values() {
            if let Some(alias) = value.as_str().and_then(|value| value.strip_prefix('#')) {
                aliases.insert(alias.to_owned());
            }
        }
        // 父链声明及已核实内置父模型都采用与渲染器相同的资源对象。
        let elements = parents
            .chain
            .iter()
            .filter_map(|document| document.get("elements"))
            .map(|value| {
                value
                    .as_array()
                    .map(Vec::as_slice)
                    .ok_or("elements 必须为数组")
            })
            .chain(
                parents
                    .fallback
                    .into_iter()
                    .map(|elements| Ok(elements.as_slice())),
            );
        for elements in elements {
            for element in elements? {
                let Some(faces) = element.get("faces") else {
                    continue;
                };
                for face in faces.as_object().ok_or("模型 faces 必须为对象")?.values() {
                    if let Some(value) = face.get("texture") {
                        let value = value.as_str().ok_or("面纹理必须为字符串")?;
                        let reference = face_texture_reference(value);
                        if let Some(alias) = reference.strip_prefix('#') {
                            aliases.insert(alias.to_owned());
                        }
                    }
                }
            }
        }
        Ok(aliases
            .into_iter()
            .map(|alias| TextureSlot {
                authored: inherited.get(&alias).map(|value| match value {
                    Value::String(value) => value.clone(),
                    value => value.to_string(),
                }),
                effective: resolve_slot(&textures, &alias),
                overridden: explicit.contains(&alias),
                alias,
            })
            .collect())
    }
    pub(crate) fn configured_choice(&self, state: &State) -> Option<&Value> {
        self.configured
            .get(state)
            .or_else(|| self.configured.get(self.render_state(state)))
    }
    pub(crate) fn available_models(&self) -> &[String] {
        &self.available_models
    }
    pub(crate) fn install_configured(&mut self, bindings: &[Binding]) -> Result<(), String> {
        self.available_models = self
            .files
            .keys()
            .filter_map(|key| key.strip_prefix("model:").map(str::to_owned))
            .collect();
        for binding in bindings {
            binding.validate()?;
            if !binding.enabled {
                continue;
            }
            let parent = normalize_model(&binding.model).ok_or("自定义模型 ID 无效")?;
            if !self.files.contains_key(&parent) {
                return Err(format!(
                    "方块 {} 指定的实际模型不存在：{}",
                    binding.selector(),
                    binding.model
                ));
            }
            let encoded = serde_json::to_vec(binding).map_err(|e| e.to_string())?;
            let hash = format!("{:x}", Sha256::digest(&encoded));
            let id = format!("mcedit2:configured_block/{hash}");
            let key = normalize_model(&id).expect("内部模型标识符合资源格式");
            if self.files.contains_key(&key) {
                return Err("自定义模型内部标识与现有资源冲突".into());
            }
            let textures = binding
                .textures
                .iter()
                .map(|(key, value)| {
                    (
                        key.strip_prefix('#').unwrap_or(key).to_owned(),
                        Value::String(value.clone()),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            let model = serde_json::json!({"parent":binding.model,"textures":textures});
            let bytes = serde_json::to_vec(&model).map_err(|e| e.to_string())?;
            self.insert(
                &format!("assets/mcedit2/models/configured_block/{hash}.json"),
                &bytes,
            )?;
            let choice = serde_json::json!({"model":id,"x":binding.rotation[0],"y":binding.rotation[1],"uvlock":binding.uvlock});
            self.configured.bytes = self
                .configured
                .bytes
                .checked_add(encoded.len() + bytes.len() + std::mem::size_of::<State>() + 128)
                .ok_or("自定义模型工作集溢出")?;
            if binding.all_states {
                self.configured
                    .names
                    .insert(binding.state.name.clone(), choice);
            } else {
                self.configured.exact.insert(binding.state.clone(), choice);
            }
        }
        if let Some(budget) = self.budget {
            budget.check(self.source_bytes(), 0, 0)?;
        }
        // 直接传播真实父链 / loader 错误，不能让失败配置退回旧模型。
        for choice in self.configured.choices() {
            let key = normalize_model(choice["model"].as_str().expect("已建立模型选择"))
                .expect("已建立资源标识");
            graph::compile(&self.files, &key, &|spec| self.bake_obj(spec), &|key| {
                self.layer_catalog(key)
            })?;
        }
        Ok(())
    }
}
