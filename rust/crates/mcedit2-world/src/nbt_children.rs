//! NBT 显示排序与真实节点位置换算；不改写容器或列表的存储顺序。
use super::{
    array_item, entry, index, parse_path, parse_root, refuse, resolve, Document, Entry, Result, Tag,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChildOrder {
    Stored,
    #[default]
    NameAscending,
    NameDescending,
    TypeAscending,
    TypeDescending,
}
impl ChildOrder {
    fn descending(self) -> bool {
        matches!(self, Self::NameDescending | Self::TypeDescending)
    }
    fn names(self) -> bool {
        matches!(self, Self::NameAscending | Self::NameDescending)
    }
}

// 与原 NBTFilterProxyModel 的类型顺序一致。
fn type_rank(kind: u8) -> u8 {
    match kind {
        10 => 0,
        9 => 1,
        11 => 2,
        12 => 3,
        7 => 4,
        8 => 5,
        6 => 6,
        5 => 7,
        4 => 8,
        3 => 9,
        2 => 10,
        1 => 11,
        _ => unreachable!("已解析的子节点必须有实际 NBT 类型"),
    }
}
pub(super) fn length(node: &Tag<'_>) -> Result<usize> {
    Ok(match node.kind {
        10 => node.fields.len(),
        9 => node.items.len(),
        7 => node.raw.len() - 4,
        11 => (node.raw.len() - 4) / 4,
        12 => (node.raw.len() - 4) / 8,
        _ => return Err(refuse("节点不是容器")),
    })
}
pub(super) fn compound_order(node: &Tag<'_>, order: ChildOrder) -> Vec<usize> {
    if order.names() {
        let mut keys: Vec<_> = node
            .fields
            .iter()
            .enumerate()
            .map(|(i, (name, child))| (i, !matches!(child.kind, 9 | 10), name.to_lowercase()))
            .collect();
        keys.sort_by(|a, b| {
            let ordering = a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2));
            if order.descending() {
                ordering.reverse()
            } else {
                ordering
            }
        });
        keys.into_iter().map(|key| key.0).collect()
    } else {
        let mut indices: Vec<_> = (0..node.fields.len()).collect();
        if order != ChildOrder::Stored {
            indices.sort_by(|&a, &b| {
                let ordering =
                    type_rank(node.fields[a].1.kind).cmp(&type_rank(node.fields[b].1.kind));
                if order.descending() {
                    ordering.reverse()
                } else {
                    ordering
                }
            });
        }
        indices
    }
}
pub(super) fn sequence_index(ordinal: usize, total: usize, order: ChildOrder) -> usize {
    if order == ChildOrder::NameDescending {
        total - 1 - ordinal
    } else {
        ordinal
    }
}
pub(super) fn child_position(parent: &Tag<'_>, child: &str, order: ChildOrder) -> Result<usize> {
    let total = length(parent)?;
    if parent.kind == 10 {
        let mut matches = parent
            .fields
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| name == child);
        let source = matches
            .next()
            .map(|(i, _)| i)
            .ok_or_else(|| refuse("目标字段不存在"))?;
        if matches.next().is_some() {
            return Err(refuse(format!("NBT 路径字段重复：{child}")));
        }
        if order == ChildOrder::Stored {
            return Ok(source);
        }
        return Ok(compound_order(parent, order)
            .iter()
            .position(|&i| i == source)
            .expect("显示排列包含每个实际字段一次"));
    }
    let source = index(child)?;
    if source >= total {
        return Err(refuse("目标元素索引越界"));
    }
    Ok(sequence_index(source, total, order))
}
impl Document {
    /// 目标在所选显示顺序中的序号；返回的路径和后续编辑仍使用原始字段名 / 索引。
    pub fn child_index_ordered(&self, path: &str, order: ChildOrder) -> Result<usize> {
        let root = parse_root(&self.bytes).map_err(|e| refuse(e.to_string()))?;
        let mut parts = parse_path(path)?;
        let child = parts.pop().ok_or_else(|| refuse("根节点没有父容器索引"))?;
        let parent = resolve(&root, &parts)?;
        child_position(parent, &child, order)
    }
    /// 先按整个容器排序，再只生成所需页；大数组不展开为完整 Value 或显示节点。
    pub fn children_ordered(
        &self,
        path: &str,
        offset: usize,
        limit: usize,
        order: ChildOrder,
    ) -> Result<(usize, Vec<Entry>)> {
        if limit == 0 || limit > 256 {
            return Err(refuse("NBT 每页大小必须为 1..=256"));
        }
        let root = parse_root(&self.bytes).map_err(|e| refuse(e.to_string()))?;
        let node = resolve(&root, &parse_path(path)?)?;
        let total = length(node)?;
        let mut rows = Vec::new();
        let indices =
            (node.kind == 10 && order != ChildOrder::Stored).then(|| compound_order(node, order));
        for ordinal in offset..total.min(offset.saturating_add(limit)) {
            let source = if node.kind == 10 {
                indices.as_ref().map_or(ordinal, |indices| indices[ordinal])
            } else {
                sequence_index(ordinal, total, order)
            };
            match node.kind {
                10 => {
                    let (name, child) = &node.fields[source];
                    rows.push(entry(
                        child,
                        format!("{path}/{}", name.replace('~', "~0").replace('/', "~1")),
                    ));
                }
                9 => rows.push(entry(&node.items[source], format!("{path}/{source}"))),
                7 | 11 | 12 => {
                    let value = array_item(node, source)?;
                    rows.push(Entry {
                        path: format!("{path}/{source}"),
                        kind: value.kind(),
                        bytes: match value.kind() {
                            1 => 1,
                            3 => 4,
                            _ => 8,
                        },
                        summary: format!("{value:?}"),
                    });
                }
                _ => unreachable!("已核对容器类型"),
            }
        }
        Ok((total, rows))
    }
}
