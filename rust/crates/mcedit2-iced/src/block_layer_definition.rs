//! 方块实体外观入口。定义显式声明追加 / 替换、源 BE 身份和数据版本，
//! 模型部分继续由同一个模型图与模型层后端解释，不执行模组专用渲染器。
use super::{normalize_model, ModelBank, ObjSelection};
use mcedit2_world::{entity_catalog::EntityVisual, nbt_document::Value as Nbt, sections::State};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Append,
    Replace,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    mode: Mode,
    ids: BTreeSet<String>,
    data_versions: BTreeSet<i32>,
    #[serde(default)]
    source_fields: crate::visual_schema::Fields,
}
pub(super) type Policies = BTreeMap<String, OnceLock<Result<Arc<Policy>, String>>>;
fn key(state: &State) -> Option<String> {
    let (ns, path) = state.name.split_once(':')?;
    normalize_model(&format!("{ns}:block_entity/{path}"))
}
impl ModelBank {
    pub(crate) fn validate_block_visual_definitions(&self) -> Vec<String> {
        self.declarative_block_states()
            .into_iter()
            .filter_map(|name| {
                self.block_layer_policy(&State {
                    name,
                    properties: Vec::new(),
                })
                .and_then(Result::err)
            })
            .collect()
    }
    pub(crate) fn declarative_block_states(&self) -> BTreeSet<String> {
        let mut names: BTreeSet<String> = self
            .files
            .keys()
            .filter_map(|key| {
                let (ns, path) = key.strip_prefix("model:")?.split_once(':')?;
                let block = path.strip_prefix("block_entity/")?;
                Some(format!("{ns}:{block}"))
            })
            .collect();
        names.extend(self.configured.states().map(|state| state.name.clone()));
        names.extend(self.configured.names().cloned());
        names
    }
    fn block_layer_policy(&self, state: &State) -> Option<Result<(String, Arc<Policy>), String>> {
        let key = key(self.render_state(state))?;
        let document = self.files.get(&key)?;
        let policies = self.block_layer_policies.get_or_init(|| {
            self.files
                .keys()
                .filter(|key| {
                    key.strip_prefix("model:")
                        .and_then(|id| id.split_once(':'))
                        .is_some_and(|(_, path)| path.starts_with("block_entity/"))
                })
                .map(|key| (key.clone(), OnceLock::new()))
                .collect()
        });
        let result = policies.get(&key)?.get_or_init(|| {
            let result = (|| {
                let policy: Policy = serde_json::from_value(
                    document
                        .get("block_entity")
                        .cloned()
                        .ok_or("方块实体定义缺少 block_entity 合同")?,
                )
                .map_err(|e| format!("方块实体定义 {key} 无效：{e}"))?;
                if policy.ids.is_empty() || policy.data_versions.is_empty() {
                    return Err(format!("方块实体定义 {key} 必须声明 BE 身份和 DataVersion"));
                }
                if policy.ids.iter().any(|id| {
                    crate::resource_ids::qualified(id, "minecraft").as_deref() != Some(id.as_str())
                }) || policy.data_versions.iter().any(|v| *v < 0)
                {
                    return Err(format!("方块实体定义 {key} 含无效身份或版本"));
                }
                crate::visual_schema::check(&policy.source_fields)?;
                Ok(Arc::new(policy))
            })();
            if let Err(error) = &result {
                log::error!("[models] {key} 外观合同无效：{error}");
            }
            result
        });
        Some(
            result
                .as_ref()
                .map(|policy| (key, Arc::clone(policy)))
                .map_err(Clone::clone),
        )
    }
    pub(crate) fn declarative_block_replaces(&self, state: &State) -> Option<bool> {
        if self.configured_choice(state).is_some() {
            return Some(true);
        }
        self.block_layer_policy(state).map(|result| {
            result
                .map(|(_, policy)| matches!(policy.mode, Mode::Replace))
                .unwrap_or(true)
        })
    }
    pub(crate) fn declarative_block_layers(
        &self,
        state: &State,
        visual: Option<&EntityVisual>,
    ) -> Option<(bool, Result<Vec<ObjSelection>, String>)> {
        if self.configured_choice(state).is_some() {
            return Some((true, Ok(Vec::new())));
        }
        let policy = self.block_layer_policy(state)?;
        let (key, policy) = match policy {
            Ok(policy) => policy,
            Err(error) => return Some((true, Err(error))),
        };
        let replaces = matches!(policy.mode, Mode::Replace);
        let result = (|| {
            let visual = visual.ok_or("声明式外观缺少同代源方块实体 NBT")?;
            let version = visual
                .source_version
                .ok_or("声明式方块实体源 DataVersion 未知")?;
            if !policy.data_versions.contains(&version) {
                return Err(format!("方块实体定义 {key} 未声明源 DataVersion {version}"));
            }
            let Nbt::String(id) = visual.source_field("id")? else {
                return Err("源方块实体 id 必须为 String".into());
            };
            if !policy.ids.contains(&id) {
                return Err(format!("方块实体定义 {key} 不接受源身份 {id}"));
            }
            crate::visual_schema::validate(&policy.source_fields, visual)?;
            let model = self
                .compiled_model(&key)
                .ok_or_else(|| format!("方块实体定义 {key} 无法编译"))?;
            if model.layers.is_empty() || !model.elements.is_empty() || !model.triangles.is_empty()
            {
                return Err(format!("方块实体定义 {key} 必须由模型层组成"));
            }
            let context = crate::layer_renderer::Context {
                state: Some(self.render_state(state)),
                visual: Some(visual),
                params: None,
                catalog: None,
            };
            let mut faces = Vec::new();
            for layer in &model.layers {
                faces.extend(layer.instantiate(&context)?);
            }
            Ok(faces)
        })();
        Some((replaces, result))
    }
}
