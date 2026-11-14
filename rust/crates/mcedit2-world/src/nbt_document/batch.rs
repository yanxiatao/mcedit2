//! 一次解析、按原快照定位的 NBT 多项修改；未选中的载荷及字段头保留原字节。
use super::{field_bytes, index, parse_path, refuse, Operation, Result, Value, MAX_BYTES};
use crate::sections::{parse_root, Tag};
use std::{borrow::Cow, collections::BTreeMap};

#[derive(Default)]
struct Node<'a> {
    direct: Option<&'a Operation>,
    fields: Vec<(&'a str, &'a Value)>,
    items: BTreeMap<usize, Vec<&'a Value>>,
    children: BTreeMap<String, Node<'a>>,
}
impl<'a> Node<'a> {
    fn occupied(&self) -> bool {
        self.direct.is_some()
            || !self.fields.is_empty()
            || !self.items.is_empty()
            || !self.children.is_empty()
    }
    fn add(&mut self, operation: &'a Operation) -> Result<()> {
        let path = match operation {
            Operation::Replace { path, .. }
            | Operation::Remove { path }
            | Operation::Rename { path, .. }
            | Operation::InsertField { path, .. }
            | Operation::InsertItem { path, .. } => parse_path(path)?,
        };
        let mut node = self;
        for part in path {
            if node.direct.is_some() {
                return Err(refuse("NBT 批量操作同时修改了节点及其子节点"));
            }
            node = node.children.entry(part).or_default();
        }
        if node.direct.is_some() {
            return Err(refuse("NBT 批量操作的目标重复或相互覆盖"));
        }
        match operation {
            Operation::InsertField { name, value, .. } => node.fields.push((name, value)),
            Operation::InsertItem { index, value, .. } => {
                node.items.entry(*index).or_default().push(value);
            }
            _ => {
                if node.occupied() {
                    return Err(refuse("NBT 批量操作同时修改了节点及其子节点"));
                }
                node.direct = Some(operation);
            }
        }
        Ok(())
    }
    fn removed(&self) -> bool {
        matches!(self.direct, Some(Operation::Remove { .. }))
    }
}

fn extend(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    if out
        .len()
        .checked_add(bytes.len())
        .is_none_or(|n| n > MAX_BYTES)
    {
        return Err(refuse("编辑后的 NBT 超过 64 MiB"));
    }
    out.extend_from_slice(bytes);
    Ok(())
}
fn length(out: &mut Vec<u8>, len: usize) -> Result<()> {
    extend(
        out,
        &i32::try_from(len)
            .map_err(|_| refuse("NBT 集合长度超限"))?
            .to_be_bytes(),
    )
}
fn field(out: &mut Vec<u8>, name: &str, kind: u8, payload: &[u8]) -> Result<()> {
    let mut header = Vec::new();
    field_bytes(name, kind, &[], &mut header)?;
    extend(out, &header)?;
    extend(out, payload)
}
fn indexed<'a, 'p>(node: &'p Node<'a>, len: usize) -> Result<BTreeMap<usize, &'p Node<'a>>> {
    let mut children = BTreeMap::new();
    for (key, child) in &node.children {
        let at = index(key)?;
        if at >= len {
            return Err(refuse("NBT 批量操作的原始索引越界"));
        }
        if matches!(child.direct, Some(Operation::Rename { .. })) {
            return Err(refuse("NBT 列表和数组元素不能重命名"));
        }
        children.insert(at, child);
    }
    if !node.fields.is_empty() || node.items.keys().any(|&at| at > len) {
        return Err(refuse("NBT 集合插入类型错误或原始索引越界"));
    }
    Ok(children)
}

fn rewrite<'b>(tag: &Tag<'b>, node: &Node<'_>) -> Result<(u8, Cow<'b, [u8]>)> {
    if let Some(Operation::Replace { value, .. }) = node.direct {
        return Ok((value.kind(), Cow::Owned(value.payload()?)));
    }
    if !node.occupied() || node.direct.is_some() {
        return Ok((tag.kind, Cow::Borrowed(tag.raw)));
    }
    let mut out = Vec::new();
    match tag.kind {
        10 => {
            if !node.items.is_empty() {
                return Err(refuse("InsertItem 需要 List 或数组"));
            }
            let mut original_names = BTreeMap::new();
            for (name, _) in &tag.fields {
                *original_names.entry(name.as_str()).or_insert(0usize) += 1;
            }
            for key in node.children.keys() {
                if original_names.get(key.as_str()) != Some(&1) {
                    return Err(refuse(format!("NBT 目标字段不存在或重复：{key}")));
                }
            }
            let mut names = std::collections::BTreeSet::new();
            let mut cursor = 0;
            for (name, child) in &tag.fields {
                let payload_at = child.raw.as_ptr() as usize - tag.raw.as_ptr() as usize;
                let header = &tag.raw[cursor..payload_at];
                cursor = payload_at + child.raw.len();
                let plan = node.children.get(name);
                if plan.is_some_and(Node::removed) {
                    continue;
                }
                let final_name = match plan.and_then(|p| p.direct) {
                    Some(Operation::Rename { name, .. }) => name,
                    _ => name,
                };
                if !names.insert(final_name.as_str()) {
                    return Err(refuse("编辑后的 NBT Compound 字段名称重复"));
                }
                let (kind, payload) = if let Some(plan) = plan {
                    rewrite(child, plan)?
                } else {
                    (child.kind, Cow::Borrowed(child.raw))
                };
                if kind == child.kind && final_name == name {
                    extend(&mut out, header)?;
                    extend(&mut out, &payload)?;
                } else {
                    field(&mut out, final_name, kind, &payload)?;
                }
            }
            for &(name, value) in &node.fields {
                if !names.insert(name) {
                    return Err(refuse("新增 NBT 字段名称重复或已存在"));
                }
                field(&mut out, name, value.kind(), &value.payload()?)?;
            }
            extend(&mut out, &[0])?;
        }
        9 => {
            let children = indexed(node, tag.items.len())?;
            let mut kind = tag.raw[0];
            if kind == 0 && tag.items.is_empty() {
                if let Some(value) = node.items.values().flatten().next() {
                    kind = value.kind();
                }
            }
            if node.items.values().flatten().any(|v| v.kind() != kind) {
                return Err(refuse("NBT 列表新元素类型不一致"));
            }
            out.push(kind);
            length(
                &mut out,
                tag.items.len() + node.items.values().map(Vec::len).sum::<usize>()
                    - children.values().filter(|p| p.removed()).count(),
            )?;
            for (at, child) in tag.items.iter().enumerate() {
                if let Some(values) = node.items.get(&at) {
                    for value in values {
                        extend(&mut out, &value.payload()?)?;
                    }
                }
                if let Some(plan) = children.get(&at) {
                    if plan.removed() {
                        continue;
                    }
                    let (new_kind, payload) = rewrite(child, plan)?;
                    if new_kind != kind {
                        return Err(refuse("NBT 列表替换元素类型不一致"));
                    }
                    extend(&mut out, &payload)?;
                } else {
                    extend(&mut out, child.raw)?;
                }
            }
            if let Some(values) = node.items.get(&tag.items.len()) {
                for value in values {
                    extend(&mut out, &value.payload()?)?;
                }
            }
        }
        7 | 11 | 12 => {
            let (width, kind) = match tag.kind {
                7 => (1, 1),
                11 => (4, 3),
                _ => (8, 4),
            };
            let len = (tag.raw.len() - 4) / width;
            let children = indexed(node, len)?;
            if node.items.values().flatten().any(|v| v.kind() != kind) {
                return Err(refuse("NBT 数组插入元素类型不匹配"));
            }
            length(
                &mut out,
                len + node.items.values().map(Vec::len).sum::<usize>()
                    - children.values().filter(|p| p.removed()).count(),
            )?;
            let positions = children
                .keys()
                .chain(node.items.keys())
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            let mut cursor = 4;
            for at in positions {
                let split = 4 + at * width;
                extend(&mut out, &tag.raw[cursor..split])?;
                if let Some(values) = node.items.get(&at) {
                    for value in values {
                        extend(&mut out, &value.payload()?)?;
                    }
                }
                cursor = split;
                if let Some(plan) = children.get(&at) {
                    match plan.direct {
                        Some(Operation::Remove { .. }) => (),
                        Some(Operation::Replace { value, .. }) if value.kind() == kind => {
                            extend(&mut out, &value.payload()?)?;
                        }
                        _ => return Err(refuse("NBT 数组仅支持同类型元素替换、插入或删除")),
                    }
                    cursor += width;
                }
            }
            extend(&mut out, &tag.raw[cursor..])?;
        }
        _ => return Err(refuse("NBT 操作经过非容器节点")),
    }
    Ok((tag.kind, Cow::Owned(out)))
}

pub(super) fn patch(bytes: &[u8], operations: &[Operation]) -> Result<Vec<u8>> {
    if bytes.len() > MAX_BYTES || operations.is_empty() {
        return Err(refuse(
            "NBT 批量修改需要 1..=65536 项操作及不超过 64 MiB 的来源",
        ));
    }
    let root = parse_root(bytes).map_err(|e| refuse(e.to_string()))?;
    let mut plan = Node::default();
    for operation in operations {
        plan.add(operation)?;
    }
    if matches!(
        plan.direct,
        Some(Operation::Remove { .. } | Operation::Rename { .. })
    ) {
        return Err(refuse("根只能替换为 Compound，不能移除或重命名"));
    }
    let (kind, payload) = rewrite(&root, &plan)?;
    if kind != 10 {
        return Err(refuse("NBT 根只能替换为 Compound"));
    }
    let offset = root.raw.as_ptr() as usize - bytes.as_ptr() as usize;
    let mut out = Vec::new();
    extend(&mut out, &bytes[..offset])?;
    extend(&mut out, &payload)?;
    parse_root(&out).map_err(|e| refuse(format!("编辑后的 NBT 无效：{e}")))?;
    if out == bytes {
        return Err(refuse("NBT 操作没有改变任何内容"));
    }
    Ok(out)
}
