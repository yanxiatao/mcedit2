//! 工具私有引用与原生写回结果的逐根回执；令牌不会进入存档 NBT。
use crate::*;
use mcedit2_world::editing::snapshot::{Content, ObjectResults};
use mcedit2_world::nbt_document::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bindings {
    pub entities: Vec<u64>,
    pub block_entities: Vec<u64>,
}
impl Bindings {
    pub fn validate(&self, content: &Content) -> Result<(), String> {
        if self.entities.len() != content.entities.len()
            || self.block_entities.len() != content.block_entities.len()
        {
            return Err("工具引用令牌数量与实际输出根不一致".into());
        }
        let mut tokens = std::collections::BTreeSet::new();
        for token in self.entities.iter().chain(&self.block_entities) {
            if *token == 0 || !tokens.insert(*token) {
                return Err("工具引用令牌为零或重复".into());
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Root {
    pub token: u64,
    pub submitted: Value,
    pub canonical: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub entities: Vec<Root>,
    pub block_entities: Vec<Root>,
}
pub(crate) fn prepare(
    output: &RunOutput,
    results: Option<&ObjectResults>,
    budget: u64,
) -> Result<Option<Receipt>, String> {
    let Some(bindings) = &output.object_bindings else {
        return Ok(None);
    };
    let content = output
        .content
        .as_ref()
        .ok_or("工具引用回执缺少实际输出内容")?;
    prepare_local(bindings, content, results, budget).map(Some)
}
pub(crate) fn prepare_local(
    bindings: &Bindings,
    content: &Content,
    results: Option<&ObjectResults>,
    budget: u64,
) -> Result<Receipt, String> {
    bindings.validate(content)?;
    // 已完成领域预检但没有源差异时，根保持提交身份；不能借用上一候选的根顺序。
    let unchanged;
    let results = match results {
        Some(results) => results,
        None => {
            unchanged = ObjectResults {
                entities: content
                    .entities
                    .iter()
                    .map(|root| root.nbt.clone())
                    .collect(),
                block_entities: content
                    .block_entities
                    .iter()
                    .map(|root| root.nbt.clone())
                    .collect(),
            };
            &unchanged
        }
    };
    if results.entities.len() != bindings.entities.len()
        || results.block_entities.len() != bindings.block_entities.len()
    {
        return Err("原生对象回执与本次工具输出根数量不一致".into());
    }
    let roots = |tokens: &[u64],
                 submitted: &[mcedit2_world::editing::snapshot::Object],
                 canonical: &[Value]| {
        tokens
            .iter()
            .zip(submitted)
            .zip(canonical)
            .map(|((&token, submitted), canonical)| Root {
                token,
                submitted: submitted.nbt.clone(),
                canonical: canonical.clone(),
            })
            .collect()
    };
    let receipt = Receipt {
        entities: roots(&bindings.entities, &content.entities, &results.entities),
        block_entities: roots(
            &bindings.block_entities,
            &content.block_entities,
            &results.block_entities,
        ),
    };
    validate_budget(&receipt, budget)?;
    Ok(receipt)
}
pub(crate) fn validate_budget(receipt: &Receipt, budget: u64) -> Result<(), String> {
    let bytes = transport::serialized_size(receipt, budget)?;
    work_budget(budget)?
        .check(
            usize::try_from(bytes)
                .map_err(|_| "工具回执超出地址范围")?
                .checked_mul(16)
                .ok_or("工具回执工作集溢出")?,
        )
        .map_err(|error| error.to_string())
}
