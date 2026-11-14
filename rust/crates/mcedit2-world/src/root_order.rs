//! 实体与方块实体根列表的原始载荷置换；仅改变指定成员的相对顺序。
use crate::{
    editing::EditWorkBudget,
    sections::{parse_root, Tag},
    WorldError,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};

fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Kind {
    EmbeddedEntities,
    SeparateEntities,
    BlockEntities,
}

/// 绑定真实列表摘要与完整排列，重放不能增删对象或修改任意对象字节。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    chunk: (i32, i32),
    kind: Kind,
    list_sha256: [u8; 32],
    order: Vec<usize>,
}

fn field<'a, 'b>(node: &'a Tag<'b>, name: &str) -> Result<Option<&'a Tag<'b>>, WorldError> {
    let mut fields = node.fields.iter().filter(|(key, _)| key == name);
    let first = fields.next().map(|(_, value)| value);
    if fields.next().is_some() {
        return Err(invalid(format!("根列表所属字段重复：{name}")));
    }
    Ok(first)
}

fn list<'a, 'b>(root: &'a Tag<'b>, kind: Kind) -> Result<&'a Tag<'b>, WorldError> {
    let level = field(root, "Level")?;
    let parent = if kind == Kind::SeparateEntities {
        if level.is_some() {
            return Err(invalid("独立实体载荷不应具有 Level 容器"));
        }
        root
    } else {
        match level {
            Some(level) if level.kind == 10 => level,
            Some(_) => return Err(invalid("根列表 Level 不是 Compound")),
            None => root,
        }
    };
    let name = match kind {
        Kind::BlockEntities if level.is_some() => "TileEntities",
        Kind::BlockEntities => "block_entities",
        _ => "Entities",
    };
    let list = field(parent, name)?.ok_or_else(|| invalid(format!("缺少真实根列表：{name}")))?;
    if list.kind != 9 || (list.raw[0] != 10 && !(list.raw[0] == 0 && list.items.is_empty())) {
        return Err(invalid("重排目标不是 Compound 列表"));
    }
    Ok(list)
}

/// 指定索引的成员只占用它们原先的槽；其他成员保持原索引与原字节。
pub(crate) fn order_indices(length: usize, requested: &[usize]) -> Result<Vec<usize>, WorldError> {
    let mut slots = requested.to_vec();
    slots.sort_unstable();
    if slots.last().is_some_and(|index| *index >= length)
        || slots.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(invalid("根列表排列包含重复或越界成员"));
    }
    let mut order = (0..length).collect::<Vec<_>>();
    for (slot, source) in slots.into_iter().zip(requested.iter().copied()) {
        order[slot] = source;
    }
    Ok(order)
}

/// 独立文件直接移动完整对象，保留独立 blockPos、未知字段和未选择成员。
pub(crate) fn reorder_slots<T>(
    items: &mut Vec<T>,
    requested: &[usize],
) -> Result<Vec<usize>, WorldError> {
    let order = order_indices(items.len(), requested)?;
    let moved = order
        .iter()
        .enumerate()
        .filter(|(slot, source)| *slot != **source)
        .map(|(_, &source)| source)
        .collect::<Vec<_>>();
    if moved.is_empty() {
        return Ok(moved);
    }
    let mut source = std::mem::take(items)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    *items = order
        .into_iter()
        .map(|index| source[index].take().expect("完整排列中每个来源只使用一次"))
        .collect();
    Ok(moved)
}

impl Proof {
    pub fn chunk(&self) -> (i32, i32) {
        self.chunk
    }
    pub fn separate(&self) -> bool {
        self.kind == Kind::SeparateEntities
    }
    pub(crate) fn is_entity(&self) -> bool {
        self.kind != Kind::BlockEntities
    }

    pub(crate) fn prepare(
        source: &[u8],
        chunk: (i32, i32),
        kind: Kind,
        requested: &[Vec<u8>],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Self>, WorldError> {
        if requested.len() < 2 {
            return Ok(None);
        }
        budget.check(
            source
                .len()
                .checked_mul(6)
                .ok_or_else(|| invalid("根列表重排工作集溢出"))?,
        )?;
        if cancelled() {
            return Err(invalid("根列表重排已取消"));
        }
        let root = parse_root(source).map_err(|error| invalid(error.to_string()))?;
        let list = list(&root, kind)?;
        budget.check(
            source
                .len()
                .checked_mul(6)
                .and_then(|bytes| {
                    list.items
                        .len()
                        .checked_mul(128)
                        .and_then(|indices| bytes.checked_add(indices))
                })
                .ok_or_else(|| invalid("根列表索引工作集溢出"))?,
        )?;
        let mut members = BTreeMap::<&[u8], VecDeque<usize>>::new();
        for (index, item) in list.items.iter().enumerate() {
            if index.is_multiple_of(4096) && cancelled() {
                return Err(invalid("根列表重排已取消"));
            }
            members.entry(item.raw).or_default().push_back(index);
        }
        let mut selected = Vec::with_capacity(requested.len());
        for (index, payload) in requested.iter().enumerate() {
            if index.is_multiple_of(4096) && cancelled() {
                return Err(invalid("根列表重排已取消"));
            }
            selected.push(
                members
                    .get_mut(payload.as_slice())
                    .and_then(VecDeque::pop_front)
                    .ok_or_else(|| invalid("待重排的完整对象载荷不在当前真实列表中"))?,
            );
        }
        let order = order_indices(list.items.len(), &selected)?;
        if order.iter().copied().eq(0..list.items.len()) {
            return Ok(None);
        }
        Ok(Some(Self {
            chunk,
            kind,
            list_sha256: Sha256::digest(list.raw).into(),
            order,
        }))
    }

    fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        let root = parse_root(source).map_err(|error| invalid(error.to_string()))?;
        let list = list(&root, self.kind)?;
        if <[u8; 32]>::from(Sha256::digest(list.raw)) != self.list_sha256
            || self.order.len() != list.items.len()
        {
            return Err(invalid("根列表重排来源已变化"));
        }
        order_indices(list.items.len(), &self.order)?;
        let offset = list.raw.as_ptr() as usize - source.as_ptr() as usize;
        let mut result = Vec::with_capacity(source.len());
        result.extend_from_slice(&source[..offset + 5]);
        for &index in &self.order {
            result.extend_from_slice(list.items[index].raw);
        }
        result.extend_from_slice(&source[offset + list.raw.len()..]);
        Ok(result)
    }
    pub(crate) fn replay_entities(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        if !self.is_entity() {
            return Err(invalid("实体顺序证明不能修改方块实体列表"));
        }
        self.replay(source)
    }
    pub(crate) fn replay_blocks(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        if self.is_entity() {
            return Err(invalid("方块实体顺序证明不能修改普通实体列表"));
        }
        self.replay(source)
    }
}

/// 导入证明按新增根顺序生成，每步真实结果的最后一项就是该步追加的完整根。
pub(crate) fn last_imported_entity(source: &[u8], separate: bool) -> Result<Vec<u8>, WorldError> {
    let root = parse_root(source).map_err(|error| invalid(error.to_string()))?;
    let kind = if separate {
        Kind::SeparateEntities
    } else {
        Kind::EmbeddedEntities
    };
    Ok(list(&root, kind)?
        .items
        .last()
        .ok_or_else(|| invalid("实体导入结果缺少新增根"))?
        .raw
        .to_vec())
}
