//! 计划更新的空间传输与原始列表历史；时间、优先级和扩展字段不经重编码。
use crate::{
    sections::{parse_root, Tag},
    WorldError,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

fn invalid(text: impl ToString) -> WorldError {
    WorldError::Invalid(text.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Block,
    Fluid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tick {
    /// 相对于所属剪贴板的方块坐标；世界列表读取时为绝对坐标。
    pub position: [i32; 3],
    pub kind: Kind,
    /// 不含 Compound 标签头、包含 End 的原始载荷。
    pub compound: Vec<u8>,
}

fn one<'a, 'b>(tag: &'a Tag<'b>, name: &str) -> Result<Option<&'a Tag<'b>>, WorldError> {
    let mut found = tag.fields.iter().filter(|(key, _)| key == name);
    let value = found.next().map(|(_, value)| value);
    if found.next().is_some() {
        return Err(invalid(format!("计划更新字段 {name} 重复")));
    }
    Ok(value)
}
pub fn position(tag: &Tag<'_>) -> Result<[i32; 3], WorldError> {
    if tag.kind != 10 {
        return Err(invalid("计划更新不是 Compound"));
    }
    let mut position = [0; 3];
    for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
        let field = one(tag, name)?.ok_or_else(|| invalid("计划更新缺少坐标"))?;
        if field.kind != 3 {
            return Err(invalid("计划更新坐标必须为 Int"));
        }
        position[axis] = field.i32().map_err(invalid)?;
    }
    Ok(position)
}
pub fn compound_position(compound: &[u8]) -> Result<[i32; 3], WorldError> {
    let mut raw = vec![10, 0, 0];
    raw.extend_from_slice(compound);
    position(&parse_root(&raw).map_err(invalid)?)
}
pub fn relocate(compound: &[u8], destination: [i32; 3]) -> Result<Vec<u8>, WorldError> {
    compound_position(compound)?;
    crate::be_editing::relocate_compound(compound, destination.into())
}
pub(crate) fn read_list(tag: &Tag<'_>, kind: Kind) -> Result<Vec<Tick>, WorldError> {
    if tag.kind != 9 || (tag.raw[0] != 10 && !(tag.raw[0] == 0 && tag.items.is_empty())) {
        return Err(invalid("计划更新需要 Compound 列表"));
    }
    tag.items
        .iter()
        .map(|item| {
            Ok(Tick {
                position: position(item)?,
                kind,
                compound: item.raw.to_vec(),
            })
        })
        .collect()
}
fn container<'a, 'b>(root: &'a Tag<'b>) -> Result<(&'a Tag<'b>, bool), WorldError> {
    match one(root, "Level")? {
        Some(level) if level.kind == 10 => Ok((level, true)),
        Some(_) => Err(invalid("计划更新的 Level 不是 Compound")),
        None => Ok((root, false)),
    }
}
fn names(legacy: bool) -> [(Kind, &'static str); 2] {
    if legacy {
        [(Kind::Block, "TileTicks"), (Kind::Fluid, "LiquidTicks")]
    } else {
        [(Kind::Block, "block_ticks"), (Kind::Fluid, "fluid_ticks")]
    }
}
pub(crate) fn read_chunk(raw: &[u8]) -> Result<Vec<Tick>, WorldError> {
    let root = parse_root(raw).map_err(invalid)?;
    let (container, legacy) = container(&root)?;
    let mut ticks = Vec::new();
    for (kind, name) in names(legacy) {
        if let Some(list) = one(container, name)? {
            ticks.extend(read_list(list, kind)?);
        }
    }
    Ok(ticks)
}
pub(crate) fn fingerprint(raw: &[u8]) -> Result<String, WorldError> {
    use sha2::{Digest, Sha256};
    let root = parse_root(raw).map_err(invalid)?;
    let (container, legacy) = container(&root)?;
    let mut digest = Sha256::new();
    digest.update([u8::from(legacy)]);
    for (_, name) in names(legacy) {
        match one(container, name)? {
            Some(tag) => {
                digest.update([tag.kind]);
                digest.update((tag.raw.len() as u64).to_be_bytes());
                digest.update(tag.raw);
            }
            None => digest.update([0]),
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub(crate) fn list_payload<'a>(
    compounds: impl IntoIterator<Item = &'a [u8]>,
) -> Result<Vec<u8>, WorldError> {
    let compounds: Vec<_> = compounds.into_iter().collect();
    let mut result = vec![10];
    result.extend(
        i32::try_from(compounds.len())
            .map_err(invalid)?
            .to_be_bytes(),
    );
    for compound in compounds {
        compound_position(compound)?;
        result.extend_from_slice(compound);
    }
    Ok(result)
}

/// 只记录实际改变的更新列表；重放不覆盖同一区块中的几何、实体或其他属性。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    chunk: (i32, i32),
    legacy: bool,
    data_version: Option<i32>,
    fields: Vec<Field>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Field {
    name: String,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
}
impl Proof {
    pub fn chunk(&self) -> (i32, i32) {
        self.chunk
    }
    /// 共用光照事务先恢复原列表，再按正式历史顺序重放，不拿最终列表冒充来源。
    pub(crate) fn restore_fields(
        &self,
        target: &[u8],
        source: &[u8],
    ) -> Result<Vec<u8>, WorldError> {
        let mut output = target.to_vec();
        let parent = if self.legacy { "/Level" } else { "" };
        for field in &self.fields {
            output =
                crate::nbt_document::copy_raw_field(&output, parent, &field.name, source, parent)?;
        }
        Ok(output)
    }
    pub(crate) fn plan(
        raw: &[u8],
        chunk: (i32, i32),
        touched: &BTreeSet<[i32; 3]>,
        incoming: &[Tick],
    ) -> Result<Option<Self>, WorldError> {
        let root = parse_root(raw).map_err(invalid)?;
        let (container, legacy) = container(&root)?;
        let decoded = crate::sections::decode_sections(raw).map_err(invalid)?;
        if (decoded.x, decoded.z) != (Some(chunk.0), Some(chunk.1)) {
            return Err(invalid("计划更新的区块坐标不一致"));
        }
        let mut fields = Vec::new();
        for (kind, name) in names(legacy) {
            let old = one(container, name)?;
            let existing = old
                .map(|tag| read_list(tag, kind))
                .transpose()?
                .unwrap_or_default();
            let additions = incoming
                .iter()
                .filter(|tick| tick.kind == kind && touched.contains(&tick.position))
                .collect::<Vec<_>>();
            let removed = existing.iter().any(|tick| touched.contains(&tick.position));
            if !removed && additions.is_empty() {
                continue;
            }
            let after = list_payload(
                existing
                    .iter()
                    .filter(|tick| !touched.contains(&tick.position))
                    .map(|tick| tick.compound.as_slice())
                    .chain(additions.iter().map(|tick| tick.compound.as_slice())),
            )?;
            if old.is_some_and(|old| old.raw == after) {
                continue;
            }
            fields.push(Field {
                name: name.into(),
                before: old.map(|tag| tag.raw.to_vec()),
                after,
            });
        }
        Ok((!fields.is_empty()).then_some(Self {
            chunk,
            legacy,
            data_version: decoded.data_version,
            fields,
        }))
    }
    pub fn replay(&self, raw: &[u8]) -> Result<Vec<u8>, WorldError> {
        let root = parse_root(raw).map_err(invalid)?;
        let (container, legacy) = container(&root)?;
        let decoded = crate::sections::decode_sections(raw).map_err(invalid)?;
        if legacy != self.legacy
            || decoded.data_version != self.data_version
            || (decoded.x, decoded.z) != (Some(self.chunk.0), Some(self.chunk.1))
        {
            return Err(invalid("计划更新历史的区块来源或版本已变化"));
        }
        let mut donor = vec![10, 0, 0];
        let mut seen = BTreeSet::new();
        for field in &self.fields {
            if !names(legacy).iter().any(|(_, name)| *name == field.name)
                || !seen.insert(&field.name)
            {
                return Err(invalid("计划更新历史字段无效或重复"));
            }
            if one(container, &field.name)?.map(|tag| tag.raw) != field.before.as_deref() {
                return Err(invalid("计划更新列表在预览后已变化"));
            }
            donor.push(9);
            donor.extend(
                u16::try_from(field.name.len())
                    .map_err(invalid)?
                    .to_be_bytes(),
            );
            donor.extend(field.name.as_bytes());
            donor.extend_from_slice(&field.after);
        }
        donor.push(0);
        let tag = parse_root(&donor).map_err(invalid)?;
        for (name, list) in &tag.fields {
            let kind = names(legacy)
                .into_iter()
                .find(|(_, field)| *field == name.as_str())
                .expect("字段已核对")
                .0;
            for tick in read_list(list, kind)? {
                if (
                    tick.position[0].div_euclid(16),
                    tick.position[2].div_euclid(16),
                ) != self.chunk
                {
                    return Err(invalid("计划更新位置超出所属区块"));
                }
            }
        }
        let mut output = raw.to_vec();
        for field in &self.fields {
            output = crate::nbt_document::copy_raw_field(
                &output,
                if legacy { "/Level" } else { "" },
                &field.name,
                &donor,
                "",
            )?;
        }
        Ok(output)
    }
}
