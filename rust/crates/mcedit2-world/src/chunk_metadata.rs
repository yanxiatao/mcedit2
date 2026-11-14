//! 区块元数据的快照批次证明。几何、坐标、实体与版本仍由其所属编辑域维护。
use crate::{
    nbt_document::{self, Operation, Value},
    sections::{parse_root, Tag},
    WorldError,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
fn err(text: impl Into<String>) -> WorldError {
    WorldError::Invalid(text.into())
}
fn escape(text: &str) -> String {
    text.replace('~', "~0").replace('/', "~1")
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Field {
    parent: String,
    name: String,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Stamp {
    field: Field,
    sha: Option<String>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    source: Vec<Stamp>,
    operations: Vec<Operation>,
}
/// 保存时逐份验证实际元数据字段，不把其他领域的新内容覆盖回旧快照。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    chunk: (i32, i32),
    legacy: bool,
    data_version: Option<i32>,
    steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    basis: Vec<crate::chunk_completion::Basis>,
    /// 完成字段由会话的显式赋值域物化，普通历史字节保留其原基底。
    #[serde(default)]
    completion_deferred: bool,
}
fn one<'a, 'b>(tag: &'a Tag<'b>, name: &str) -> Result<Option<&'a Tag<'b>>, WorldError> {
    let mut fields = tag.fields.iter().filter(|(key, _)| *key == name);
    let first = fields.next().map(|(_, tag)| tag);
    if fields.next().is_some() {
        return Err(err(format!("区块元数据字段 {name} 重复")));
    }
    Ok(first)
}
fn legacy(raw: &[u8]) -> Result<bool, WorldError> {
    let root = parse_root(raw).map_err(|e| err(e.to_string()))?;
    match one(&root, "Level")? {
        Some(level) if level.kind == 10 => Ok(true),
        None => Ok(false),
        _ => Err(err("Level 不是 Compound")),
    }
}
fn managed(name: &str) -> bool {
    matches!(
        name,
        "DataVersion"
            | "Level"
            | "xPos"
            | "yPos"
            | "zPos"
            | "Blocks"
            | "Data"
            | "Add"
            | "Sections"
            | "sections"
            | "Biomes"
            | "biomes"
            | "HeightMap"
            | "Heightmaps"
            | "BlockLight"
            | "SkyLight"
            | "Entities"
            | "entities"
            | "TileEntities"
            | "block_entities"
    )
}
/// 标量 BE 日志不承载区块元数据；派生字段和其他编辑域分别由所属流程核对。
pub(crate) fn scalar_checkpoint_metadata(raw: &[u8]) -> Result<[u8; 32], WorldError> {
    let root = parse_root(raw).map_err(|error| err(error.to_string()))?;
    let mut digest = Sha256::new();
    for (scope, container) in
        std::iter::once((0_u8, &root)).chain(root.field("Level").map(|level| (1, level)))
    {
        let mut fields = container
            .fields
            .iter()
            .filter(|(name, _)| {
                !managed(name) && !matches!(name.as_str(), "isLightOn" | "LightPopulated")
            })
            .collect::<Vec<_>>();
        fields.sort_by(|(left, _), (right, _)| left.cmp(right));
        digest.update([scope]);
        for (name, tag) in fields {
            digest.update((name.len() as u64).to_be_bytes());
            digest.update(name.as_bytes());
            digest.update([tag.kind]);
            digest.update((tag.raw.len() as u64).to_be_bytes());
            digest.update(tag.raw);
        }
    }
    Ok(digest.finalize().into())
}
fn slot(parts: &[String], legacy: bool) -> Result<Field, WorldError> {
    let (parent, name) = if legacy && parts.first().is_some_and(|p| p == "Level") {
        ("/Level", parts.get(1))
    } else {
        ("", parts.first())
    };
    let name = name.ok_or_else(|| err("请修改区块中的具体元数据字段"))?;
    if managed(name) {
        return Err(err(format!(
            "{name} 由方块、实体、biome 或区块来源工具维护，请使用对应工具"
        )));
    }
    Ok(Field {
        parent: parent.into(),
        name: name.clone(),
    })
}
fn fields(operations: &[Operation], legacy: bool) -> Result<BTreeSet<Field>, WorldError> {
    if operations.is_empty() {
        return Err(err("区块元数据批次为空"));
    }
    let mut result = BTreeSet::new();
    for op in operations {
        let path = match op {
            Operation::Replace { path, .. }
            | Operation::InsertField { path, .. }
            | Operation::InsertItem { path, .. }
            | Operation::Remove { path }
            | Operation::Rename { path, .. } => path,
        };
        let parts = nbt_document::parse_path(path)?;
        let inserted = match op {
            Operation::InsertField { name, .. } => Some(name),
            _ => None,
        };
        if parts.is_empty() || (legacy && parts == ["Level"]) {
            let name = inserted.ok_or_else(|| err("区块根容器不能整体替换、删除或重命名"))?;
            let mut target = parts;
            target.push(name.clone());
            result.insert(slot(&target, legacy)?);
        } else {
            result.insert(slot(&parts, legacy)?);
            if let Operation::Rename { name, .. } = op {
                let mut renamed = parts;
                *renamed.last_mut().expect("具体字段路径非空") = name.clone();
                result.insert(slot(&renamed, legacy)?);
            }
        }
    }
    Ok(result)
}
fn stamp(raw: &[u8], field: &Field) -> Result<Option<String>, WorldError> {
    let root = parse_root(raw).map_err(|e| err(e.to_string()))?;
    let container = if field.parent.is_empty() {
        &root
    } else {
        one(&root, "Level")?.ok_or_else(|| err("区块缺少 Level"))?
    };
    Ok(one(container, &field.name)?.map(|tag| {
        let mut digest = Sha256::new();
        digest.update([tag.kind]);
        digest.update(tag.raw);
        format!("{:x}", digest.finalize())
    }))
}
fn properties(raw: &[u8]) -> Result<(), WorldError> {
    let root = parse_root(raw).map_err(|e| err(e.to_string()))?;
    let container = match one(&root, "Level")? {
        Some(level) => level,
        None => &root,
    };
    for name in ["TerrainPopulated", "LightPopulated", "isLightOn"] {
        if let Some(value) = one(container, name)? {
            if value.kind != 1 || !matches!(value.byte().map_err(|e| err(e.to_string()))?, 0 | 1) {
                return Err(err(format!("{name} 应为值 0 或 1 的 Byte")));
            }
        }
    }
    for name in ["InhabitedTime", "LastUpdate"] {
        if one(container, name)?.is_some_and(|tag| tag.kind != 4) {
            return Err(err(format!("{name} 应为 Long")));
        }
    }
    if let Some(status) = one(container, "Status")? {
        if status.kind != 8 || status.string().map_err(|e| err(e.to_string()))?.is_empty() {
            return Err(err("Status 应为非空 String"));
        }
    }
    Ok(())
}
/// 编辑器和日志重放共用的领域检查；验证失败不改变文档。
pub fn validate_batch(raw: &[u8], operations: &[Operation]) -> Result<(), WorldError> {
    fields(operations, legacy(raw)?)?;
    Ok(())
}
pub fn validate_result(raw: &[u8]) -> Result<(), WorldError> {
    properties(raw)
}
impl Proof {
    pub fn chunk(&self) -> (i32, i32) {
        self.chunk
    }
    pub(crate) fn new(
        raw: &[u8],
        chunk: (i32, i32),
        batches: &[Vec<Operation>],
    ) -> Result<Self, WorldError> {
        if batches.is_empty() {
            return Err(err("区块元数据没有修改批次"));
        }
        let decoded = crate::sections::decode_sections(raw).map_err(|e| err(e.to_string()))?;
        let mut proof = Self {
            chunk,
            legacy: legacy(raw)?,
            data_version: decoded.data_version,
            steps: Vec::new(),
            basis: Vec::new(),
            completion_deferred: false,
        };
        let mut output = raw.to_vec();
        for batch in batches {
            let source = fields(batch, proof.legacy)?
                .into_iter()
                .map(|field| {
                    Ok(Stamp {
                        sha: stamp(&output, &field)?,
                        field,
                    })
                })
                .collect::<Result<Vec<_>, WorldError>>()?;
            output = nbt_document::patch_batch(&output, batch)?;
            properties(&output)?;
            proof.steps.push(Step {
                source,
                operations: batch.clone(),
            });
        }
        proof.replay(raw)?;
        Ok(proof)
    }
    pub(crate) fn completion_fields(
        &self,
    ) -> Result<BTreeSet<crate::chunk_completion::Field>, WorldError> {
        let parent = if self.legacy { "/Level" } else { "" };
        self.steps
            .iter()
            .try_fold(BTreeSet::new(), |mut result, step| {
                for field in fields(&step.operations, self.legacy)? {
                    if field.parent == parent {
                        if let Some(field) = crate::chunk_completion::Field::from_name(&field.name)
                        {
                            result.insert(field);
                        }
                    }
                }
                Ok(result)
            })
    }
    /// 用户批次在实际显示值上执行，历史证明仍从原逻辑字节开始重放。
    pub(crate) fn new_rebased(
        raw: &[u8],
        shown: &[u8],
        chunk: (i32, i32),
        batches: &[Vec<Operation>],
    ) -> Result<Self, WorldError> {
        let shown_proof = Self::new(shown, chunk, batches)?;
        let source = crate::chunk_completion::read(raw)?;
        let display = crate::chunk_completion::read(shown)?;
        let mut basis = Vec::new();
        let mut values = crate::chunk_completion::Values::new();
        for field in shown_proof.completion_fields()? {
            if source[&field] != display[&field] {
                basis.push(crate::chunk_completion::Basis {
                    field,
                    source: source[&field].clone(),
                    display: display[&field].clone(),
                });
                values.insert(field, display[&field].clone());
            }
        }
        let initial = crate::chunk_completion::write(raw, &values)?;
        let mut proof = Self::new(&initial, chunk, batches)?;
        proof.basis = basis;
        proof.completion_deferred = !proof.completion_fields()?.is_empty();
        proof.replay(raw)?;
        Ok(proof)
    }
    pub(crate) fn replay(&self, raw: &[u8]) -> Result<Vec<u8>, WorldError> {
        let decoded = crate::sections::decode_sections(raw).map_err(|e| err(e.to_string()))?;
        if decoded.x != Some(self.chunk.0)
            || decoded.z != Some(self.chunk.1)
            || decoded.data_version != self.data_version
            || legacy(raw)? != self.legacy
            || self.steps.is_empty()
        {
            return Err(err("区块元数据证明的来源身份不匹配"));
        }
        let source = crate::chunk_completion::read(raw)?;
        let touched = self.completion_fields()?;
        let mut values = crate::chunk_completion::Values::new();
        for basis in &self.basis {
            if !touched.contains(&basis.field)
                || source[&basis.field] != basis.source
                || basis.source == basis.display
                || values.insert(basis.field, basis.display.clone()).is_some()
            {
                return Err(err("完成字段显示基底与已核对的历史来源不一致"));
            }
        }
        let mut out = crate::chunk_completion::write(raw, &values)?;
        for step in &self.steps {
            let expected = fields(&step.operations, self.legacy)?;
            if expected
                != step
                    .source
                    .iter()
                    .map(|check| check.field.clone())
                    .collect()
                || expected.len() != step.source.len()
            {
                return Err(err("区块元数据证明字段缺失、无关或重复"));
            }
            for check in &step.source {
                if stamp(&out, &check.field)? != check.sha {
                    return Err(err(format!(
                        "区块字段 {}{} 的原始载荷变化",
                        check.field.parent, check.field.name
                    )));
                }
            }
            out = nbt_document::patch_batch(&out, &step.operations)?;
            properties(&out)?;
        }
        if self.completion_deferred {
            if touched.is_empty() {
                return Err(err("完成字段赋值证明缺少实际字段"));
            }
            let values = touched
                .into_iter()
                .map(|field| (field, source[&field].clone()))
                .collect();
            out = crate::chunk_completion::write(&out, &values)?;
        }
        Ok(out)
    }
    /// 光照独立验证先撤下已证明的元数据，再将其按顺序重放到最终光照载荷。
    pub(crate) fn restore_fields(
        &self,
        target: &[u8],
        source: &[u8],
    ) -> Result<Vec<u8>, WorldError> {
        let mut out = target.to_vec();
        for field in self
            .steps
            .iter()
            .flat_map(|step| step.source.iter().map(|check| check.field.clone()))
            .collect::<BTreeSet<_>>()
        {
            out = nbt_document::copy_raw_field(
                &out,
                &field.parent,
                &field.name,
                source,
                &field.parent,
            )?;
        }
        Ok(out)
    }
}
/// 属性页直接操作原有真实字段；不存在的字段不凭版本猜测创建。
pub fn property_rows(
    document: &nbt_document::Document,
) -> Result<Vec<(String, String, Value)>, WorldError> {
    let prefix = if legacy(document.bytes())? {
        "/Level"
    } else {
        ""
    };
    let root = parse_root(document.bytes()).map_err(|e| err(e.to_string()))?;
    let container = if prefix.is_empty() {
        &root
    } else {
        one(&root, "Level")?.expect("已确认 Level")
    };
    let mut rows = Vec::new();
    for (name, label) in [
        ("TerrainPopulated", "地形生成完成"),
        ("LightPopulated", "光照完成"),
        ("isLightOn", "光照有效"),
        ("Status", "生成状态"),
        ("InhabitedTime", "居住时间"),
        ("LastUpdate", "最后更新时间"),
    ] {
        if one(container, name)?.is_some() {
            let path = format!("{prefix}/{}", escape(name));
            rows.push((label.into(), path.clone(), document.inspect(&path)?));
        }
    }
    Ok(rows)
}
