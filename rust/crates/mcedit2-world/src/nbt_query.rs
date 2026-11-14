//! 可分片、可取消的类型化 NBT 查询。调用者负责枚举真实源并将替换接入对应域证明。
use crate::{
    nbt_document::{parse_path, Document, Operation, Value},
    WorldError,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// 路径和区块坐标由真实来源提供，不把内存中的伪对象当作世界实体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File {
        path: PathBuf,
    },
    StructureObject {
        path: PathBuf,
        object: crate::structure_document::Object,
    },
    Chunk {
        dimension: String,
        chunk: (i32, i32),
    },
    BlockEntity {
        dimension: String,
        chunk: (i32, i32),
        position: (i32, i32, i32),
    },
    Entity {
        dimension: String,
        chunk: (i32, i32),
        entity_path: String,
    },
}
#[derive(Debug, Clone)]
pub enum ValueMatch {
    Exact(Value),
    StringContains(String),
}
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub name_contains: Option<String>,
    pub name_exact: Option<String>,
    pub value: Option<ValueMatch>,
    pub kind: Option<u8>,
}
#[derive(Debug, Clone)]
pub struct Hit {
    pub source: Source,
    pub source_sha: [u8; 32],
    pub path: String,
    pub kind: u8,
    pub value: Option<Value>,
}
enum Task {
    Page { path: String, offset: usize },
    Node(crate::nbt_document::Entry),
}
/// 搜索结果由调用者逐批消费，取消和未完成状态不会伪装为全量结果。
pub struct Cursor {
    source: Source,
    sha: [u8; 32],
    document: Document,
    filter: Filter,
    tasks: Vec<Task>,
    visited: u64,
    excluded: Vec<String>,
}
pub struct Batch {
    pub hits: Vec<Hit>,
    pub visited: u64,
    pub complete: bool,
    pub cancelled: bool,
}
impl Cursor {
    pub fn new(source: Source, bytes: Vec<u8>, filter: Filter) -> Result<Self, WorldError> {
        if filter.name_contains.is_none()
            && filter.name_exact.is_none()
            && filter.value.is_none()
            && filter.kind.is_none()
        {
            return Err(invalid("NBT 查询条件为空"));
        }
        let sha = digest(&bytes);
        let document = Document::new(bytes)?;
        Ok(Self {
            source,
            sha,
            document,
            filter,
            tasks: vec![Task::Page {
                path: String::new(),
                offset: 0,
            }],
            visited: 0,
            excluded: Vec::new(),
        })
    }
    pub fn exclude_subtrees(&mut self, paths: Vec<String>) {
        self.excluded = paths;
    }
    pub fn advance(
        &mut self,
        budget: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Batch, WorldError> {
        if budget == 0 {
            return Err(invalid("查询分片预算必须大于零"));
        }
        let mut hits = Vec::new();
        let mut consumed = 0;
        while consumed < budget && !self.tasks.is_empty() {
            if cancelled() {
                return Ok(Batch {
                    hits,
                    visited: self.visited,
                    complete: false,
                    cancelled: true,
                });
            }
            let task = self
                .tasks
                .pop()
                .ok_or_else(|| invalid("查询游标状态无效"))?;
            consumed += 1;
            match task {
                Task::Page { path, offset } => {
                    let (total, entries) = self.document.children(&path, offset, 128)?;
                    let next = offset
                        .checked_add(entries.len())
                        .ok_or_else(|| invalid("NBT 查询页数溢出"))?;
                    if next < total {
                        self.tasks.push(Task::Page { path, offset: next });
                    }
                    for entry in entries.into_iter().rev() {
                        self.tasks.push(Task::Node(entry));
                    }
                }
                Task::Node(entry) => {
                    if self.excluded.iter().any(|path| path == &entry.path) {
                        continue;
                    }
                    self.visited = self
                        .visited
                        .checked_add(1)
                        .ok_or_else(|| invalid("NBT 查询计数溢出"))?;
                    if matches!(entry.kind, 7 | 9 | 10 | 11 | 12) {
                        self.tasks.push(Task::Page {
                            path: entry.path.clone(),
                            offset: 0,
                        });
                    }
                    let name = parse_path(&entry.path)?
                        .pop()
                        .ok_or_else(|| invalid("查询结果路径无效"))?;
                    if self
                        .filter
                        .name_contains
                        .as_ref()
                        .is_some_and(|v| !name.contains(v))
                        || self.filter.name_exact.as_ref().is_some_and(|v| v != &name)
                        || self.filter.kind.is_some_and(|k| k != entry.kind)
                    {
                        continue;
                    }
                    let value = if matches!(entry.kind, 1..=6 | 8) {
                        Some(self.document.inspect(&entry.path)?)
                    } else {
                        None
                    };
                    let matches = match &self.filter.value {
                        None => true,
                        Some(ValueMatch::Exact(expected)) => {
                            if expected.kind() != entry.kind {
                                false
                            } else if let Some(value) = &value {
                                value == expected
                            } else {
                                &self.document.inspect(&entry.path)? == expected
                            }
                        }
                        Some(ValueMatch::StringContains(needle)) => {
                            matches!(&value,Some(Value::String(s)) if s.contains(needle))
                        }
                    };
                    if matches {
                        hits.push(Hit {
                            source: self.source.clone(),
                            source_sha: self.sha,
                            path: entry.path,
                            kind: entry.kind,
                            value,
                        });
                    }
                }
            }
        }
        Ok(Batch {
            hits,
            visited: self.visited,
            complete: self.tasks.is_empty(),
            cancelled: false,
        })
    }
}

/// 对同一来源的选中结果生成真实类型操作，变更前重新核对源摘要。
/// 重命名会改变后续路径，故一次只允许互不重叠的结果，避免隐式部分替换。
pub fn replace(
    bytes: &[u8],
    hits: &[Hit],
    value: Option<Value>,
    name: Option<&str>,
) -> Result<Vec<u8>, WorldError> {
    if hits.is_empty() || value.is_none() && name.is_none() {
        return Err(invalid("替换缺少结果或操作"));
    }
    let sha = digest(bytes);
    let source = &hits[0].source;
    let mut paths = std::collections::BTreeSet::new();
    for hit in hits {
        if hit.source_sha != sha || &hit.source != source || !paths.insert(hit.path.clone()) {
            return Err(invalid("查询来源已变化或结果重复"));
        }
    }
    let mut output = bytes.to_vec();
    if let Some(value) = value {
        let operations = hits
            .iter()
            .map(|hit| Operation::Replace {
                path: hit.path.clone(),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        output = crate::nbt_document::patch_batch(&output, &operations)?;
    }
    if let Some(name) = name {
        let operations = hits
            .iter()
            .map(|hit| Operation::Rename {
                path: hit.path.clone(),
                name: name.to_owned(),
            })
            .collect::<Vec<_>>();
        output = crate::nbt_document::patch_batch(&output, &operations)?;
    }
    Ok(output)
}

/// 批量结果可包含已经等于目标的节点；只将实际变化交给严格补丁器。
pub fn changed_operations(
    document: &Document,
    operations: &[Operation],
) -> Result<Vec<Operation>, WorldError> {
    let mut changed = Vec::new();
    for operation in operations {
        let equal = match operation {
            Operation::Replace { path, value } => document.inspect(path)? == *value,
            Operation::Rename { path, name } => {
                document.child_index(path)?;
                let mut parts = parse_path(path)?;
                let current = parts.pop();
                let parent = parts
                    .iter()
                    .map(|part| format!("/{}", part.replace('~', "~0").replace('/', "~1")))
                    .collect::<String>();
                current.as_ref() == Some(name)
                    && matches!(document.inspect(&parent)?, Value::Compound(_))
            }
            _ => false,
        };
        if !equal {
            changed.push(operation.clone());
        }
    }
    Ok(changed)
}

/// 同一类型化结果共用替换语义；重命名单独在值变更之后执行。
pub fn mutation_operations(
    hit: &Hit,
    mutation: &crate::editing::NbtMutation,
) -> Result<(Vec<Operation>, Vec<Operation>), WorldError> {
    use crate::editing::{NbtMutation, NbtReplacement};
    let mut operations = Vec::new();
    let mut renames = Vec::new();
    match mutation {
        NbtMutation::Remove => operations.push(Operation::Remove {
            path: hit.path.clone(),
        }),
        NbtMutation::Replace { name, value } => {
            if name.is_none() && value.is_none() {
                return Err(invalid("请指定替换名称或值"));
            }
            if let Some(value) = value {
                let value = match value {
                    NbtReplacement::Typed(value) => value.clone(),
                    NbtReplacement::Scalar(text) => parse_scalar_like(
                        hit.value
                            .clone()
                            .ok_or_else(|| invalid("容器结果需要类型化 NBT 替换值"))?,
                        text,
                    )
                    .map_err(invalid)?,
                };
                operations.push(Operation::Replace {
                    path: hit.path.clone(),
                    value,
                });
            }
            if let Some(name) = name {
                renames.push(Operation::Rename {
                    path: hit.path.clone(),
                    name: name.clone(),
                });
            }
        }
    }
    Ok((operations, renames))
}

/// 标量输入保持实际节点宽度；整数不经过浮点转换。
pub fn parse_scalar_like(old: Value, text: &str) -> Result<Value, String> {
    Ok(match old {
        Value::String(_) => Value::String(text.into()),
        Value::Byte(_) => Value::Byte(text.parse().map_err(|_| "请输入 Byte 范围内的整数")?),
        Value::Short(_) => Value::Short(text.parse().map_err(|_| "请输入 Short 范围内的整数")?),
        Value::Int(_) => Value::Int(text.parse().map_err(|_| "请输入 Int 范围内的整数")?),
        Value::Long(_) => Value::Long(text.parse().map_err(|_| "请输入 Long 范围内的整数")?),
        Value::Float(_) => {
            let v: f32 = text.parse().map_err(|_| "请输入浮点数")?;
            if !v.is_finite() {
                return Err("数值必须有限".into());
            }
            Value::Float(v)
        }
        Value::Double(_) => {
            let v: f64 = text.parse().map_err(|_| "请输入浮点数")?;
            if !v.is_finite() {
                return Err("数值必须有限".into());
            }
            Value::Double(v)
        }
        _ => return Err("容器与数组请使用类型化 NBT 值替换".into()),
    })
}
