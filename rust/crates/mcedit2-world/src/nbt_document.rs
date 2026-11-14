//! 原始 NBT 文档的类型化编辑。仅重建选定容器，未选中的载荷按原字节复制。
use crate::{
    sections::{parse_root, Tag},
    WorldError,
};
use serde::{Deserialize, Serialize};
#[path = "nbt_children.rs"]
mod children;
pub use children::ChildOrder;
#[path = "nbt_hierarchy.rs"]
mod hierarchy;
pub use hierarchy::{TreeNode, TreePage, TreeRow, TreeState, TREE_BRANCH_PAGE};
#[path = "nbt_fields.rs"]
mod fields;
#[path = "nbt_new_child.rs"]
mod new_child;

const MAX_BYTES: usize = 64 * 1024 * 1024;
/// 整体值编辑的编码预算；节点头和分页浏览不展开整个值。
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;
const MAX_HISTORY_BYTES: usize = 32 * 1024 * 1024;
fn refuse(text: impl Into<String>) -> WorldError {
    WorldError::Invalid(text.into())
}
type Result<T> = std::result::Result<T, WorldError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Value {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(#[serde(deserialize_with = "json_f32")] f32),
    Double(#[serde(deserialize_with = "json_f64")] f64),
    ByteArray(Vec<i8>),
    String(String),
    List(List),
    Compound(Vec<Field>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}
// arbitrary_precision 的数字经标签枚举缓冲后由 Number 表示；使用库的数字入口，
// 再直接解析目标宽度，避免 f32 经 f64 二次舍入，JSON 格式和整数精度保持不变。
fn json_f32<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<f32, D::Error> {
    let number = serde_json::Number::deserialize(deserializer)?;
    let value = number
        .to_string()
        .parse::<f32>()
        .map_err(serde::de::Error::custom)?;
    if !value.is_finite() {
        return Err(serde::de::Error::custom("NBT Float 超出有限数范围"));
    }
    Ok(value)
}
fn json_f64<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<f64, D::Error> {
    let number = serde_json::Number::deserialize(deserializer)?;
    let value = number
        .to_string()
        .parse::<f64>()
        .map_err(serde::de::Error::custom)?;
    if !value.is_finite() {
        return Err(serde::de::Error::custom("NBT Double 超出有限数范围"));
    }
    Ok(value)
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub element_type: u8,
    pub items: Vec<Value>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub value: Value,
}
impl Value {
    pub fn kind(&self) -> u8 {
        match self {
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }
    fn encode(&self, depth: usize, out: &mut Vec<u8>, limit: usize) -> Result<()> {
        if depth > 64 || out.len() > limit {
            return Err(refuse("NBT 新值超过深度或编码预算"));
        }
        match self {
            Self::Byte(v) => out.push(*v as u8),
            Self::Short(v) => out.extend(v.to_be_bytes()),
            Self::Int(v) => out.extend(v.to_be_bytes()),
            Self::Long(v) => out.extend(v.to_be_bytes()),
            Self::Float(v) => {
                if !v.is_finite() {
                    return Err(refuse("新浮点值必须有限"));
                }
                out.extend(v.to_be_bytes());
            }
            Self::Double(v) => {
                if !v.is_finite() {
                    return Err(refuse("新浮点值必须有限"));
                }
                out.extend(v.to_be_bytes());
            }
            Self::String(v) => string(v, out)?,
            Self::ByteArray(v) => {
                count(v.len(), out, limit)?;
                out.extend(v.iter().map(|v| *v as u8));
            }
            Self::IntArray(v) => {
                count(v.len(), out, limit)?;
                for value in v {
                    out.extend(value.to_be_bytes());
                }
            }
            Self::LongArray(v) => {
                count(v.len(), out, limit)?;
                for value in v {
                    out.extend(value.to_be_bytes());
                }
            }
            Self::List(v) => {
                if v.element_type > 12
                    || (v.element_type == 0 && !v.items.is_empty())
                    || v.items.iter().any(|i| i.kind() != v.element_type)
                {
                    return Err(refuse("NBT 列表元素类型不一致"));
                }
                out.push(v.element_type);
                count(v.items.len(), out, limit)?;
                for item in &v.items {
                    item.encode(depth + 1, out, limit)?;
                }
            }
            Self::Compound(v) => {
                let mut names = std::collections::BTreeSet::new();
                for field in v {
                    if !names.insert(&field.name) {
                        return Err(refuse("NBT Compound 字段名称重复"));
                    }
                    out.push(field.value.kind());
                    string(&field.name, out)?;
                    field.value.encode(depth + 1, out, limit)?;
                }
                out.push(0);
            }
        }
        if out.len() > limit {
            return Err(refuse("NBT 新值超过编码预算"));
        }
        Ok(())
    }
    pub fn payload(&self) -> Result<Vec<u8>> {
        self.payload_limited(MAX_VALUE_BYTES)
    }
    pub(crate) fn payload_limited(&self, limit: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.encode(0, &mut bytes, limit)?;
        Ok(bytes)
    }
}
fn count(n: usize, out: &mut Vec<u8>, limit: usize) -> Result<()> {
    if n > limit {
        return Err(refuse("NBT 新集合超过编码预算"));
    }
    let n = i32::try_from(n).map_err(|_| refuse("NBT 集合过大"))?;
    out.extend(n.to_be_bytes());
    Ok(())
}
fn string(value: &str, out: &mut Vec<u8>) -> Result<()> {
    let encoded = crate::nbt_utf::encode(value)?;
    let len = u16::try_from(encoded.len()).map_err(|_| refuse("NBT 字符串超过 65535 字节"))?;
    out.extend(len.to_be_bytes());
    out.extend(encoded);
    Ok(())
}

/// 路径使用 JSON Pointer 的转义规则，支持包含斜线、波浪号及空名称的真实 NBT 字段。
pub fn parse_path(path: &str) -> Result<Vec<String>> {
    if path.is_empty() {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') {
        return Err(refuse("NBT 路径必须为空（根）或以 / 开头"));
    }
    let mut parts = Vec::new();
    for part in path[1..].split('/') {
        let mut output = String::new();
        let mut chars = part.chars();
        while let Some(c) = chars.next() {
            output.push(if c == '~' {
                match chars.next() {
                    Some('0') => '~',
                    Some('1') => '/',
                    _ => return Err(refuse("NBT 路径转义无效")),
                }
            } else {
                c
            });
        }
        parts.push(output);
    }
    if parts.len() > 64 {
        return Err(refuse("NBT 路径超过 64 层"));
    }
    Ok(parts)
}
fn index(text: &str) -> Result<usize> {
    let value = text
        .parse::<usize>()
        .map_err(|_| refuse("NBT 列表路径必须使用非负索引"))?;
    if value.to_string() != text {
        return Err(refuse("NBT 列表索引必须使用标准十进制"));
    }
    Ok(value)
}
fn resolve<'t, 'b>(mut node: &'t Tag<'b>, parts: &[String]) -> Result<&'t Tag<'b>> {
    for part in parts {
        node = match node.kind {
            10 => {
                let mut fields = node.fields.iter().filter(|(name, _)| name == part);
                let first = fields.next().map(|(_, tag)| tag);
                if fields.next().is_some() {
                    return Err(refuse(format!("NBT 路径字段重复：{part}")));
                }
                first
            }
            9 => node.items.get(index(part)?),
            _ => return Err(refuse("NBT 路径经过非 Compound/List 节点")),
        }
        .ok_or_else(|| refuse(format!("NBT 路径不存在：{part}")))?;
    }
    Ok(node)
}
fn field_bytes(name: &str, kind: u8, payload: &[u8], output: &mut Vec<u8>) -> Result<()> {
    output.push(kind);
    string(name, output)?;
    output.extend(payload);
    Ok(())
}
/// 域层验证过的实体/BE 列表可提取原始 Compound 载荷，保留未知字段字节。
pub(crate) fn raw_compound_list_item(bytes: &[u8], path: &str, at: usize) -> Result<Vec<u8>> {
    let root = parse_root(bytes).map_err(|e| refuse(e.to_string()))?;
    let list = resolve(&root, &parse_path(path)?)?;
    if list.kind != 9 || (list.raw[0] != 10 && !(list.raw[0] == 0 && list.items.is_empty())) {
        return Err(refuse("目标不是 Compound 列表"));
    }
    let item = list
        .items
        .get(at)
        .ok_or_else(|| refuse("Compound 列表索引越界"))?;
    if item.kind != 10 {
        return Err(refuse("列表元素不是 Compound"));
    }
    Ok(item.raw.to_vec())
}
/// 域层核实列表与索引后，一次删除多个完整 Compound，其余载荷逐字节保留。
pub(crate) fn remove_raw_compound_list_items(
    bytes: &[u8],
    path: &str,
    remove: &std::collections::BTreeSet<usize>,
) -> Result<Vec<u8>> {
    let root = parse_root(bytes).map_err(|e| refuse(e.to_string()))?;
    let list = resolve(&root, &parse_path(path)?)?;
    if list.kind != 9 || list.raw[0] != 10 || remove.iter().any(|&i| i >= list.items.len()) {
        return Err(refuse("批量删除目标不是已核实的 Compound 列表及索引"));
    }
    let offset = list.raw.as_ptr() as usize - bytes.as_ptr() as usize;
    let removed = remove
        .iter()
        .map(|&i| list.items[i].raw.len())
        .sum::<usize>();
    let mut out = Vec::with_capacity(bytes.len() - removed);
    out.extend_from_slice(&bytes[..offset]);
    out.push(10);
    out.extend_from_slice(
        &i32::try_from(list.items.len() - remove.len())
            .map_err(|_| refuse("Compound 列表数量溢出"))?
            .to_be_bytes(),
    );
    for (index, item) in list.items.iter().enumerate() {
        if !remove.contains(&index) {
            out.extend_from_slice(item.raw);
        }
    }
    out.extend_from_slice(&bytes[offset + list.raw.len()..]);
    parse_root(&out).map_err(|e| refuse(e.to_string()))?;
    Ok(out)
}
/// 从已经解析的源中提取选中 Compound，不经 Value 往返或重新编码。
pub(crate) fn raw_compound_at(bytes: &[u8], path: &str) -> Result<Vec<u8>> {
    let root = parse_root(bytes).map_err(|e| refuse(e.to_string()))?;
    let tag = resolve(&root, &parse_path(path)?)?;
    if tag.kind != 10 {
        return Err(refuse("选中的 NBT 节点不是 Compound"));
    }
    let mut result = vec![10, 0, 0];
    result.extend_from_slice(tag.raw);
    Ok(result)
}
/// 在两个真实 NBT 中复制同名字段的原始载荷；缺失字段按实际存在性插入或移除。
/// 其余字段头与载荷保持原字节，复制过程不受 Value 的单节点编辑预算限制。
pub(crate) fn copy_raw_field(
    target: &[u8],
    parent_path: &str,
    name: &str,
    source: &[u8],
    source_parent: &str,
) -> Result<Vec<u8>> {
    let target_root = parse_root(target).map_err(|e| refuse(e.to_string()))?;
    let source_root = parse_root(source).map_err(|e| refuse(e.to_string()))?;
    let parent = resolve(&target_root, &parse_path(parent_path)?)?;
    let donor = resolve(&source_root, &parse_path(source_parent)?)?;
    if parent.kind != 10 || donor.kind != 10 {
        return Err(refuse("原始字段复制需要 Compound 容器"));
    }
    if parent.fields.iter().filter(|(key, _)| key == name).count() > 1
        || donor.fields.iter().filter(|(key, _)| key == name).count() > 1
    {
        return Err(refuse("原始字段名称重复"));
    }
    let old = parent.field(name);
    let next = donor.field(name);
    let (offset, len, bytes) = match (old, next) {
        (Some(old), Some(next)) if old.kind == next.kind => (
            (old.raw.as_ptr() as usize) - (target.as_ptr() as usize),
            old.raw.len(),
            next.raw.to_vec(),
        ),
        (Some(_), Some(_)) => return Err(refuse("原始字段类型不符")),
        (None, Some(next)) => {
            let mut bytes = Vec::new();
            field_bytes(name, next.kind, next.raw, &mut bytes)?;
            (
                (parent.raw.as_ptr() as usize) - (target.as_ptr() as usize) + parent.raw.len() - 1,
                0,
                bytes,
            )
        }
        (Some(_), None) => {
            return patch(
                target,
                &Operation::Remove {
                    path: format!(
                        "{parent_path}/{}",
                        name.replace('~', "~0").replace('/', "~1")
                    ),
                },
            )
        }
        (None, None) => return Ok(target.to_vec()),
    };
    let length = target
        .len()
        .checked_sub(len)
        .and_then(|value| value.checked_add(bytes.len()))
        .ok_or_else(|| refuse("NBT 字段复制长度溢出"))?;
    if length > MAX_BYTES {
        return Err(refuse("字段复制后的 NBT 超过 64 MiB"));
    }
    let mut output = Vec::with_capacity(length);
    output.extend_from_slice(&target[..offset]);
    output.extend_from_slice(&bytes);
    output.extend_from_slice(&target[offset + len..]);
    parse_root(&output).map_err(|e| refuse(e.to_string()))?;
    Ok(output)
}
/// 仅供经过来源与身份校验的域层插入原始载荷；函数仍独立验证 Compound 与目标类型。
pub(crate) fn insert_raw_compound_list_item(
    bytes: &[u8],
    path: &str,
    at: usize,
    payload: &[u8],
) -> Result<Vec<u8>> {
    if payload.len() > MAX_BYTES {
        return Err(refuse("Compound 原始载荷超限"));
    }
    let mut check = vec![10, 0, 0];
    check.extend_from_slice(payload);
    parse_root(&check).map_err(|e| refuse(format!("Compound 载荷无效：{e}")))?;
    let root = parse_root(bytes).map_err(|e| refuse(e.to_string()))?;
    let list = resolve(&root, &parse_path(path)?)?;
    if list.kind != 9 || (list.raw[0] != 10 && !(list.raw[0] == 0 && list.items.is_empty())) {
        return Err(refuse("目标不是 Compound 列表"));
    }
    if at > list.items.len() {
        return Err(refuse("Compound 插入索引越界"));
    }
    let new_len = list
        .items
        .len()
        .checked_add(1)
        .ok_or_else(|| refuse("列表长度溢出"))?;
    let mut replacement = vec![10];
    replacement.extend(
        i32::try_from(new_len)
            .map_err(|_| refuse("列表长度超限"))?
            .to_be_bytes(),
    );
    for (i, item) in list.items.iter().enumerate() {
        if i == at {
            replacement.extend(payload);
        }
        replacement.extend(item.raw);
    }
    if at == list.items.len() {
        replacement.extend(payload);
    }
    let offset = (list.raw.as_ptr() as usize)
        .checked_sub(bytes.as_ptr() as usize)
        .ok_or_else(|| refuse("列表偏移无效"))?;
    let size = bytes
        .len()
        .checked_sub(list.raw.len())
        .and_then(|n| n.checked_add(replacement.len()))
        .ok_or_else(|| refuse("NBT 长度溢出"))?;
    if size > MAX_BYTES {
        return Err(refuse("编辑后的 NBT 超过 64 MiB"));
    }
    let mut out = Vec::with_capacity(size);
    out.extend(&bytes[..offset]);
    out.extend(replacement);
    out.extend(&bytes[offset + list.raw.len()..]);
    parse_root(&out).map_err(|e| refuse(format!("编辑后的 NBT 无效：{e}")))?;
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Replace {
        path: String,
        value: Value,
    },
    InsertField {
        path: String,
        name: String,
        value: Value,
    },
    InsertItem {
        path: String,
        index: usize,
        value: Value,
    },
    Remove {
        path: String,
    },
    Rename {
        path: String,
        name: String,
    },
}

mod batch;

/// 对单项操作沿用批量引擎的同一类型、边界和原始字节规则。
pub fn patch(bytes: &[u8], operation: &Operation) -> Result<Vec<u8>> {
    patch_batch(bytes, std::slice::from_ref(operation))
}

/// 原子应用同一原始快照上的多项操作，仅解析来源与最终产物各一次。
/// 所有路径及列表/数组索引都指向操作前的快照；插入发生在原始索引之前。
/// 可同时修改兄弟节点、交换字段名、删除并插入同名字段；禁止重复目标、
/// 同时修改祖先与后代；同一索引的插入依输入顺序排列。任一错误都不返回部分产物。
pub fn patch_batch(bytes: &[u8], operations: &[Operation]) -> Result<Vec<u8>> {
    batch::patch(bytes, operations)
}

#[derive(Debug, Clone)]
struct Delta {
    offset: usize,
    before: Vec<u8>,
    after: Vec<u8>,
    description: crate::history::Entry,
}
#[derive(Debug, Clone)]
pub struct Document {
    revision: std::sync::Arc<()>,
    original: Vec<u8>,
    bytes: Vec<u8>,
    undo: Vec<Delta>,
    redo: Vec<Delta>,
}
impl Document {
    pub fn new(bytes: Vec<u8>) -> Result<Self> {
        parse_root(&bytes).map_err(|e| refuse(e.to_string()))?;
        Ok(Self {
            revision: std::sync::Arc::new(()),
            original: bytes.clone(),
            bytes,
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.original.len()
            + self.bytes.len()
            + self
                .undo
                .iter()
                .chain(&self.redo)
                .map(|delta| {
                    delta.before.len()
                        + delta.after.len()
                        + delta.description.label.len()
                        + delta.description.detail.len()
                })
                .sum::<usize>()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// 文档、克隆及编辑世代的身份；成功修改和历史操作后更新，失败时保持不变。
    pub fn revision(&self) -> std::sync::Arc<()> {
        std::sync::Arc::clone(&self.revision)
    }
    pub fn original(&self) -> &[u8] {
        &self.original
    }
    pub fn changed(&self) -> bool {
        self.bytes != self.original
    }
    /// 宿主已核对实际保存身份；只推进磁盘基线，当前内容及两侧历史保持不变。
    pub(crate) fn accept_saved(&mut self, saved: &Self) {
        self.original.clone_from(&saved.bytes);
        self.revision = std::sync::Arc::new(());
    }
    /// 明确释放历史载荷，不修改内容或保存基线。
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.revision = std::sync::Arc::new(());
    }
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
    pub fn history(&self) -> crate::history::Timeline {
        crate::history::Timeline {
            revision: self.revision(),
            cursor: self.undo.len(),
            entries: self
                .undo
                .iter()
                .chain(self.redo.iter().rev())
                .map(|delta| delta.description.clone())
                .collect(),
        }
    }
    /// 多步修订在副本中重放，失败或取消不改变当前字节与历史。
    pub fn goto_history_cancellable(
        &mut self,
        target: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool> {
        if target > self.undo.len() + self.redo.len() {
            return Err(refuse("目标 NBT 修订不在当前历史内"));
        }
        if target == self.undo.len() {
            return Ok(false);
        }
        let mut work = self.clone();
        while work.undo.len() != target {
            if cancelled() {
                return Err(refuse("NBT 历史跳转已取消"));
            }
            let changed = if work.undo.len() > target {
                work.undo()?
            } else {
                work.redo()?
            };
            if !changed {
                return Err(refuse("NBT 历史游标与记录不一致"));
            }
        }
        if cancelled() {
            return Err(refuse("NBT 历史发布已取消"));
        }
        *self = work;
        Ok(true)
    }
    pub fn apply(&mut self, operation: &Operation) -> Result<()> {
        self.apply_batch(std::slice::from_ref(operation))
    }
    /// 一组快照操作共享一个撤销记录；校验失败不改变文档、撤销或重做记录。
    pub fn apply_batch(&mut self, operations: &[Operation]) -> Result<()> {
        let result = patch_batch(&self.bytes, operations)?;
        let offset = self
            .bytes
            .iter()
            .zip(&result)
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = self.bytes[offset..]
            .iter()
            .rev()
            .zip(result[offset..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let delta = Delta {
            offset,
            before: self.bytes[offset..self.bytes.len() - suffix].to_vec(),
            after: result[offset..result.len() - suffix].to_vec(),
            description: {
                let describe = |operation: &Operation| {
                    (match operation {
                        Operation::Replace { path, .. } => format!("替换 {path}"),
                        Operation::InsertField { path, name, .. } => {
                            format!("新增字段 {path}/{name}")
                        }
                        Operation::InsertItem { path, index, .. } => {
                            format!("新增列表项 {path}/{index}")
                        }
                        Operation::Remove { path } => format!("删除 {path}"),
                        Operation::Rename { path, name } => format!("重命名 {path} → {name}"),
                    })
                    .chars()
                    .take(184)
                    .collect::<String>()
                };
                crate::history::Entry {
                    label: if operations.len() == 1 {
                        describe(&operations[0])
                    } else {
                        format!("批量 NBT 修改（{} 项）", operations.len())
                    },
                    detail: operations
                        .iter()
                        .take(6)
                        .map(describe)
                        .collect::<Vec<_>>()
                        .join("；"),
                }
            },
        };
        let history = self
            .undo
            .iter()
            .map(|d| {
                d.before.len()
                    + d.after.len()
                    + d.description.label.len()
                    + d.description.detail.len()
            })
            .sum::<usize>()
            + delta.before.len()
            + delta.after.len()
            + delta.description.label.len()
            + delta.description.detail.len();
        if history > MAX_HISTORY_BYTES {
            return Err(refuse("NBT 撤销历史超过 32 MiB，请在历史面板清空撤销记录"));
        }
        self.bytes = result;
        self.undo.push(delta);
        self.redo.clear();
        self.revision = std::sync::Arc::new(());
        Ok(())
    }
    pub fn undo(&mut self) -> Result<bool> {
        let Some(delta) = self.undo.last() else {
            return Ok(false);
        };
        self.bytes = apply_delta(&self.bytes, delta, false)?;
        self.redo.push(self.undo.pop().expect("已检查撤销记录"));
        self.revision = std::sync::Arc::new(());
        Ok(true)
    }
    pub fn redo(&mut self) -> Result<bool> {
        let Some(delta) = self.redo.last() else {
            return Ok(false);
        };
        self.bytes = apply_delta(&self.bytes, delta, true)?;
        self.undo.push(self.redo.pop().expect("已检查重做记录"));
        self.revision = std::sync::Arc::new(());
        Ok(true)
    }
    pub fn inspect(&self, path: &str) -> Result<Value> {
        let root = parse_root(&self.bytes).map_err(|e| refuse(e.to_string()))?;
        let parts = parse_path(path)?;
        if let Some((last, parents)) = parts.split_last() {
            let parent = resolve(&root, parents)?;
            if matches!(parent.kind, 7 | 11 | 12) {
                return array_item(parent, index(last)?);
            }
        }
        let node = resolve(&root, &parts)?;
        if node.raw.len() > MAX_VALUE_BYTES {
            return Err(refuse("节点超过 1 MiB，请选择较小子节点"));
        }
        value(node)
    }
    /// 查询真实节点类型和载荷字节数，供界面选择浏览或整体值编辑，不解码整个 Value。
    pub fn node_info(&self, path: &str) -> Result<NodeInfo> {
        let root = parse_root(&self.bytes).map_err(|e| refuse(e.to_string()))?;
        let parts = parse_path(path)?;
        if let Some((last, parents)) = parts.split_last() {
            let parent = resolve(&root, parents)?;
            if matches!(parent.kind, 7 | 11 | 12) {
                let value = array_item(parent, index(last)?)?;
                return Ok(NodeInfo {
                    kind: value.kind(),
                    bytes: match value.kind() {
                        1 => 1,
                        3 => 4,
                        _ => 8,
                    },
                });
            }
        }
        let node = resolve(&root, &parts)?;
        Ok(NodeInfo {
            kind: node.kind,
            bytes: node.raw.len(),
        })
    }
    /// 读取可选字段，缺失与类型 / 重复字段错误分开，不复制整个父 Compound。
    pub fn inspect_field(&self, parent: &str, name: &str) -> Result<Option<Value>> {
        let root = parse_root(&self.bytes).map_err(|e| refuse(e.to_string()))?;
        let node = resolve(&root, &parse_path(parent)?)?;
        if node.kind != 10 {
            return Err(refuse("字段所属节点必须为 Compound"));
        }
        let mut matching = node.fields.iter().filter(|(field, _)| field == name);
        let first = matching.next().map(|(_, tag)| tag);
        if matching.next().is_some() {
            return Err(refuse(format!("NBT 字段重复：{name}")));
        }
        first
            .map(|node| {
                if node.raw.len() > MAX_VALUE_BYTES {
                    return Err(refuse("节点超过 1 MiB，请选择较小子节点"));
                }
                value(node)
            })
            .transpose()
    }
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let root = parse_root(&self.bytes).map_err(|e| refuse(e.to_string()))?;
        let mut output = Vec::new();
        flatten(&root, "", &mut output);
        Ok(output)
    }
    /// 返回目标在直接父容器中的位置，供外部查找结果按页定位。
    pub fn child_index(&self, path: &str) -> Result<usize> {
        self.child_index_ordered(path, ChildOrder::Stored)
    }
    /// 只列举一个容器的指定页，避免大型树一次生成全部界面节点。
    pub fn children(&self, path: &str, offset: usize, limit: usize) -> Result<(usize, Vec<Entry>)> {
        self.children_ordered(path, offset, limit, ChildOrder::Stored)
    }
}
fn array_item(node: &Tag<'_>, at: usize) -> Result<Value> {
    let width = match node.kind {
        7 => 1,
        11 => 4,
        12 => 8,
        _ => return Err(refuse("节点不是数组")),
    };
    let start = 4usize
        .checked_add(
            at.checked_mul(width)
                .ok_or_else(|| refuse("数组索引溢出"))?,
        )
        .ok_or_else(|| refuse("数组索引溢出"))?;
    let bytes = node
        .raw
        .get(start..start + width)
        .ok_or_else(|| refuse("NBT 数组索引越界"))?;
    Ok(match node.kind {
        7 => Value::Byte(bytes[0] as i8),
        11 => Value::Int(i32::from_be_bytes(
            bytes.try_into().map_err(|_| refuse("数组宽度无效"))?,
        )),
        12 => Value::Long(i64::from_be_bytes(
            bytes.try_into().map_err(|_| refuse("数组宽度无效"))?,
        )),
        _ => unreachable!(),
    })
}
fn apply_delta(bytes: &[u8], delta: &Delta, forward: bool) -> Result<Vec<u8>> {
    let (before, after) = if forward {
        (&delta.before, &delta.after)
    } else {
        (&delta.after, &delta.before)
    };
    let end = delta
        .offset
        .checked_add(before.len())
        .ok_or_else(|| refuse("NBT 历史偏移溢出"))?;
    if bytes.get(delta.offset..end) != Some(before.as_slice()) {
        return Err(refuse("NBT 撤销源内容已经变化"));
    }
    let mut result = Vec::new();
    result.extend(&bytes[..delta.offset]);
    result.extend(after);
    result.extend(&bytes[end..]);
    parse_root(&result).map_err(|e| refuse(e.to_string()))?;
    Ok(result)
}
pub(crate) fn value(tag: &Tag<'_>) -> Result<Value> {
    let r = tag.raw;
    Ok(match tag.kind {
        1 => Value::Byte(r[0] as i8),
        2 => Value::Short(i16::from_be_bytes(r.try_into().unwrap())),
        3 => Value::Int(i32::from_be_bytes(r.try_into().unwrap())),
        4 => Value::Long(i64::from_be_bytes(r.try_into().unwrap())),
        5 => Value::Float(f32::from_be_bytes(r.try_into().unwrap())),
        6 => Value::Double(f64::from_be_bytes(r.try_into().unwrap())),
        7 => Value::ByteArray(r[4..].iter().map(|v| *v as i8).collect()),
        8 => Value::String(tag.string().map_err(|e| refuse(e.to_string()))?),
        9 => Value::List(List {
            element_type: r[0],
            items: tag.items.iter().map(value).collect::<Result<_>>()?,
        }),
        10 => Value::Compound(
            tag.fields
                .iter()
                .map(|(name, tag)| {
                    Ok(Field {
                        name: name.clone(),
                        value: value(tag)?,
                    })
                })
                .collect::<Result<_>>()?,
        ),
        11 => Value::IntArray(
            r[4..]
                .chunks_exact(4)
                .map(|v| i32::from_be_bytes(v.try_into().unwrap()))
                .collect(),
        ),
        12 => Value::LongArray(
            r[4..]
                .chunks_exact(8)
                .map(|v| i64::from_be_bytes(v.try_into().unwrap()))
                .collect(),
        ),
        _ => return Err(refuse("未知 NBT 类型")),
    })
}
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub kind: u8,
    pub bytes: usize,
}
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub kind: u8,
    pub bytes: usize,
    pub summary: String,
}
fn entry(tag: &Tag<'_>, path: String) -> Entry {
    let summary = if matches!(tag.kind, 1..=6 | 8) {
        value(tag)
            .map(|v| format!("{v:?}").chars().take(120).collect())
            .unwrap_or_else(|e| e.to_string())
    } else {
        format!("{} 字节", tag.raw.len())
    };
    Entry {
        path,
        kind: tag.kind,
        bytes: tag.raw.len(),
        summary,
    }
}
fn flatten(tag: &Tag<'_>, path: &str, out: &mut Vec<Entry>) {
    if out.len() >= 512 {
        return;
    }
    let summary = if matches!(tag.kind, 1..=6 | 8) {
        value(tag)
            .map(|v| format!("{v:?}").chars().take(120).collect())
            .unwrap_or_else(|e| e.to_string())
    } else {
        format!("{} 字节", tag.raw.len())
    };
    out.push(Entry {
        path: path.into(),
        kind: tag.kind,
        bytes: tag.raw.len(),
        summary,
    });
    for (name, child) in &tag.fields {
        flatten(
            child,
            &format!("{path}/{}", name.replace('~', "~0").replace('/', "~1")),
            out,
        );
    }
    for (i, child) in tag.items.iter().enumerate() {
        if out.len() >= 512 {
            break;
        }
        flatten(child, &format!("{path}/{i}"), out);
    }
}
