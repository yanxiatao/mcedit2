//! 标量 BE 检查点以当前磁盘为摘要基线，按保存点与当前历史的共同前缀生成可逆步骤。
use super::*;
use crate::{
    be_journal::{BeJournal, BeJournalEdit},
    nbt_document::Value,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScalarStep {
    chunk: ChunkPos,
    position: Position,
    field: String,
    kind: u8,
    before: String,
    after: String,
}

fn scalar(raw: &[u8], position: Position, field: &str) -> Result<(u8, String), WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| refuse(error.to_string()))?;
    let (container, name) = match root.field("Level") {
        Some(level) => (level, "TileEntities"),
        None => (&root, "block_entities"),
    };
    let list = container
        .require(name, 9)
        .map_err(|error| refuse(error.to_string()))?;
    let mut matches = list.items.iter().filter(|item| {
        item.field("x").and_then(|value| value.i32().ok()) == Some(position.0)
            && item.field("y").and_then(|value| value.i32().ok()) == Some(position.1)
            && item.field("z").and_then(|value| value.i32().ok()) == Some(position.2)
    });
    let mut tag = matches
        .next()
        .ok_or_else(|| refuse("BE 检查点的对象不存在"))?;
    if matches.next().is_some() {
        return Err(refuse("BE 检查点对象坐标重复"));
    }
    for part in field.split('/') {
        if tag.kind != 10 {
            return Err(refuse("BE 标量路径经过非 Compound 字段"));
        }
        let mut fields = tag.fields.iter().filter(|(name, _)| *name == part);
        tag = &fields.next().ok_or_else(|| refuse("BE 标量字段不存在"))?.1;
        if fields.next().is_some() {
            return Err(refuse("BE 标量字段名称重复"));
        }
    }
    let value = match crate::nbt_document::value(tag)? {
        Value::Byte(value) => value.to_string(),
        Value::Short(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Long(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Double(value) => value.to_string(),
        Value::String(value) => value,
        _ => return Err(refuse("BE 检查点只能表示原有数值或字符串标量")),
    };
    Ok((tag.kind, value))
}
fn be_name(raw: &[u8]) -> Result<&'static str, WorldError> {
    Ok(
        if crate::sections::parse_root(raw)
            .map_err(|error| refuse(error.to_string()))?
            .field("Level")
            .is_some()
        {
            "TileEntities"
        } else {
            "block_entities"
        },
    )
}

impl Session {
    pub(crate) fn scalar_checkpoint_counts(&self) -> Result<BTreeMap<ChunkPos, usize>, WorldError> {
        let current = self.collect_scalar_steps()?;
        let prefix = self
            .saved_be_steps
            .iter()
            .zip(&current)
            .take_while(|(left, right)| left == right)
            .count();
        let mut counts = BTreeMap::new();
        for step in self.saved_be_steps[prefix..]
            .iter()
            .chain(&current[prefix..])
        {
            *counts.entry(step.chunk).or_default() += 1;
        }
        Ok(counts)
    }
    pub(super) fn check_scalar_history_budget(
        &self,
        preview: &BeScalarPreview,
        source_bytes: usize,
    ) -> Result<(), WorldError> {
        let mut bytes = self
            .be_overlay
            .values()
            .try_fold(0_usize, |bytes, raw| bytes.checked_add(raw.len()))
            .ok_or_else(|| refuse("BE 历史工作集大小溢出"))?;
        let mut sources = BTreeSet::new();
        for history in self.be_undo.iter().chain(&self.be_redo) {
            if sources.insert(history.source_before.as_ptr() as usize) {
                bytes = bytes
                    .checked_add(history.source_before.len())
                    .ok_or_else(|| refuse("BE 来源工作集大小溢出"))?;
            }
            bytes = bytes
                .checked_add(history.after.len())
                .and_then(|bytes| bytes.checked_add(history.before.as_ref().map_or(0, Vec::len)))
                .ok_or_else(|| refuse("BE 历史工作集大小溢出"))?;
        }
        for step in self.saved_be_steps.iter() {
            bytes = bytes
                .checked_add(
                    step.field.len()
                        + step.before.len()
                        + step.after.len()
                        + std::mem::size_of::<ScalarStep>(),
                )
                .ok_or_else(|| refuse("BE 保存点工作集大小溢出"))?;
        }
        bytes = bytes
            .checked_add(source_bytes)
            .ok_or_else(|| refuse("BE 来源工作集大小溢出"))?;
        bytes = bytes
            .checked_add(
                preview
                    .patched_nbt
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| refuse("BE 候选工作集大小溢出"))?,
            )
            .and_then(|bytes| {
                bytes.checked_add(self.be_overlay.get(&preview.chunk).map_or(0, Vec::len))
            })
            .ok_or_else(|| refuse("BE 候选工作集大小溢出"))?;
        EditWorkBudget::default().check(bytes)
    }
    pub(super) fn collect_scalar_steps(&self) -> Result<Vec<ScalarStep>, WorldError> {
        let mut result = Vec::with_capacity(self.be_undo.len());
        for change in &self.be_undo {
            let before = change.before.as_deref().unwrap_or(&change.source_before);
            let (kind, before) = scalar(before, change.position, &change.field)?;
            let (after_kind, after) = scalar(&change.after, change.position, &change.field)?;
            if kind != after_kind {
                return Err(refuse("BE 标量历史改变了字段类型"));
            }
            result.push(ScalarStep {
                chunk: change.chunk,
                position: change.position,
                field: change.field.clone(),
                kind,
                before,
                after,
            });
        }
        Ok(result)
    }

    pub(super) fn export_be_checkpoint(&self, path: &Path) -> Result<usize, WorldError> {
        if self.has_lifecycle_slot_changes() {
            return Err(refuse(
                "标量 BE 日志不承载区块删除或恢复，请使用完整混合保存",
            ));
        }
        if let Some(origin) = &self.origin {
            origin.verify_current()?;
        }
        let steps = self.collect_scalar_steps()?;
        let prefix = self
            .saved_be_steps
            .iter()
            .zip(&steps)
            .take_while(|(left, right)| left == right)
            .count();
        let mut original = self.source_reader()?;
        let mut disk = ReadOnlyRegionCache::new(&self.world, &self.dimension)?;
        let mut source = BTreeMap::new();
        let mut targets = BTreeMap::new();
        for &chunk in self
            .dirty_chunks
            .iter()
            .filter(|chunk| self.pending_be_chunk(**chunk))
        {
            let base = match self.creation.get(&chunk) {
                Some(created) => {
                    created.verify_cached(&mut original)?;
                    created.base.clone()
                }
                None => original
                    .snapshot_chunk_bytes(chunk.0, chunk.1)?
                    .ok_or_else(|| refuse("BE 标量日志不能移除整个区块"))?,
            };
            let logical = self.be_overlay.get(&chunk).unwrap_or(&base);
            let actual = disk
                .snapshot_chunk_bytes(chunk.0, chunk.1)?
                .ok_or_else(|| refuse("BE 标量日志不能创建整个区块"))?;
            let left = decode_sections(logical).map_err(|error| refuse(error.to_string()))?;
            let right = decode_sections(&actual).map_err(|error| refuse(error.to_string()))?;
            if left.layout != right.layout || left.data_version != right.data_version {
                return Err(refuse("BE 标量日志不能转换区块格式或版本"));
            }
            self.scalar_checkpoint_metadata_unchanged(chunk, &base, logical, &actual)?;
            let target = crate::shared_chunk_save::copy(&actual, be_name(logical)?, logical)?;
            if target != actual {
                source.insert(chunk, actual);
                targets.insert(chunk, target);
            }
        }
        let mut chain = source.clone();
        let operations = self.saved_be_steps[prefix..]
            .iter()
            .rev()
            .map(|step| (step, true))
            .chain(steps[prefix..].iter().map(|step| (step, false)));
        let mut entries = Vec::new();
        for (step, reverse) in operations {
            let Some(before) = chain.get(&step.chunk) else {
                continue;
            };
            let expected = if reverse { &step.after } else { &step.before };
            let value = if reverse { &step.before } else { &step.after };
            if scalar(before, step.position, &step.field)? != (step.kind, expected.clone()) {
                return Err(refuse("BE 检查点的标量顺序与实际保存内容不同"));
            }
            let after = crate::sections::patch_original_block_entity_scalar(
                before,
                step.position,
                &step.field,
                value,
            )
            .map_err(|error| refuse(error.to_string()))?;
            entries.push(BeJournalEdit {
                xyz: [step.position.0, step.position.1, step.position.2],
                field: step.field.clone(),
                value: value.clone(),
                original_chunk_sha256: nbt_sha(&source[&step.chunk]),
                resulting_chunk_sha256: nbt_sha(&after),
            });
            chain.insert(step.chunk, after);
        }
        if chain != targets {
            return Err(refuse("当前 BE 变化不能完整表示为标量检查点"));
        }
        let manifest = BeJournal {
            version: 1,
            world_root: self.world.root.to_string_lossy().into_owned(),
            dimension: self.dimension.clone(),
            data_version: self.world.data_version,
            edits: entries,
        };
        crate::be_journal::save(&self.world, &self.dimension, &manifest, path)?;
        Ok(manifest.edits.len())
    }

    pub(super) fn import_be_checkpoint(&mut self, path: &Path) -> Result<usize, WorldError> {
        if self.has_pending_changes() {
            return Err(refuse("请先保存或放弃当前修改，再导入 BE 检查点"));
        }
        if let Some(origin) = &self.origin {
            origin.verify_current()?;
        }
        let manifest = crate::be_journal::load(&self.world, &self.dimension, path)?;
        let mut reader = ReadOnlyRegionCache::new(&self.world, &self.dimension)?;
        let mut sources = BTreeMap::new();
        let mut chain = BTreeMap::new();
        for entry in &manifest.edits {
            let point = (entry.xyz[0], entry.xyz[1], entry.xyz[2]);
            let chunk = (point.0.div_euclid(16), point.2.div_euclid(16));
            if let std::collections::btree_map::Entry::Vacant(source) = sources.entry(chunk) {
                let raw = reader
                    .snapshot_chunk_bytes(chunk.0, chunk.1)?
                    .ok_or_else(|| refuse("BE 检查点的实际源区块缺失"))?;
                source.insert(nbt_sha(&raw));
                chain.insert(chunk, raw);
            }
            if sources[&chunk] != entry.original_chunk_sha256 {
                return Err(refuse("BE 检查点的实际源摘要已改变"));
            }
            let after = crate::sections::patch_original_block_entity_scalar(
                &chain[&chunk],
                point,
                &entry.field,
                &entry.value,
            )
            .map_err(|error| refuse(error.to_string()))?;
            if nbt_sha(&after) != entry.resulting_chunk_sha256 {
                return Err(refuse("BE 检查点重放结果摘要不同"));
            }
            chain.insert(chunk, after);
        }
        let mut work = self.clone();
        for entry in &manifest.edits {
            let point = (entry.xyz[0], entry.xyz[1], entry.xyz[2]);
            let candidate = work.prepare_be_edit(point, &entry.field, &entry.value)?;
            work.commit_be_edit(candidate)?;
        }
        *self = work;
        Ok(manifest.edits.len())
    }
}
