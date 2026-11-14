//! 原版静态附加模型只是通用模型层后端的一组数据绑定，不包含逐类几何实现。
use super::{ModelBank, ObjSelection};
use crate::{
    layer_renderer::{Context, Template},
    root_transform::Affine,
};
use mcedit2_world::sections::State;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
#[derive(Clone, Debug)]
struct Rule {
    when: BTreeMap<String, String>,
    template: Template,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Models {
    rules: BTreeMap<String, Vec<Rule>>,
    dependencies: BTreeSet<String>,
    bytes: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    bindings: Vec<Binding>,
    #[serde(default)]
    block_entities: Vec<BlockBinding>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockBinding {
    block: String,
    stub: String,
    model: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    block: String,
    #[serde(default)]
    when: BTreeMap<String, String>,
    model: Value,
}
impl Models {
    fn load(version: Option<u32>) -> Result<(Self, Vec<BlockBinding>), String> {
        let mut out = Self::default();
        let raw = include_str!("../../mcedit2-world/assets/native-block-layers-1.21.1.json");
        let document: Document =
            serde_json::from_str(raw).map_err(|e| format!("原版模型绑定定义无效：{e}"))?;
        if version != Some(document.version) {
            return Ok((out, Vec::new()));
        }
        out.bytes = raw.len().checked_mul(8).ok_or("原版模型定义内存大小溢出")?;
        for binding in document.bindings {
            let texture = binding
                .model
                .get("texture")
                .and_then(Value::as_str)
                .ok_or("原版模型绑定缺少材质")?;
            out.dependencies.insert(texture.into());
            let template = Template::compile(
                &binding.model,
                Arc::new(Map::new()),
                Affine::IDENTITY,
                &BTreeMap::new(),
            )?;
            out.rules.entry(binding.block).or_default().push(Rule {
                when: binding.when,
                template,
            });
        }
        Ok((out, document.block_entities))
    }
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(super) fn dependencies(&self) -> BTreeSet<String> {
        self.dependencies.clone()
    }
    pub(super) fn at(&self, state: &State) -> Result<Option<Vec<ObjSelection>>, String> {
        let Some(rules) = self.rules.get(&state.name) else {
            return Ok(None);
        };
        let mut result = None;
        for rule in rules {
            if !rule.when.iter().all(|(key, wanted)| {
                state
                    .properties
                    .iter()
                    .any(|(name, value)| name == key && value == wanted)
            }) {
                continue;
            }
            result
                .get_or_insert_with(Vec::new)
                .extend(rule.template.instantiate(&Context::block(state))?);
        }
        Ok(result)
    }
}
impl ModelBank {
    pub(crate) fn load_native_block_models(&mut self, version: Option<u32>) -> Result<(), String> {
        let (models, block_entities) = Models::load(version)?;
        if let Some(budget) = self.budget {
            budget.check(self.total_json_bytes, models.bytes(), 0)?;
        }
        self.native_blocks = Arc::new(models);
        for binding in block_entities {
            let (namespace, path) = binding
                .block
                .split_once(':')
                .ok_or("原版方块实体定义缺少命名空间")?;
            if namespace != "minecraft" {
                return Err("内置方块实体定义不能接管其他命名空间".into());
            }
            let key = format!("model:{namespace}:block_entity/{path}");
            if self.files.contains_key(&key) || !self.is_vanilla_entity_stub(path, &binding.stub) {
                continue;
            }
            let asset = format!("assets/{namespace}/models/block_entity/{path}.json");
            if !self.insert(
                &asset,
                &serde_json::to_vec(&binding.model).map_err(|e| e.to_string())?,
            )? {
                return Err(format!("内置方块实体定义路径无效：{asset}"));
            }
        }
        Ok(())
    }
    pub(crate) fn native_block_texture_dependencies(&self) -> BTreeSet<String> {
        self.native_blocks.dependencies()
    }
}
