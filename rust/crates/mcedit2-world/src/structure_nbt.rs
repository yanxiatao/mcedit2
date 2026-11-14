//! 独立结构对象的类型化 NBT 查询与完整批量候选，不构造世界或区块。
use super::*;
use crate::{
    editing::{NbtMutation, NbtSearchOptions},
    nbt_query::{self, Cursor, Hit, Source},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct NbtHit {
    pub object: Object,
    pub position: [f64; 3],
    pub id: String,
    pub hit: Hit,
}
#[derive(Debug, Clone)]
pub struct NbtResults {
    source: Arc<BlockClipboard>,
    revision: Arc<()>,
    definitions: Option<Arc<crate::world_data::Snapshot>>,
    path: PathBuf,
    retained: usize,
    pub hits: Vec<NbtHit>,
    pub objects: usize,
    pub visited: u64,
}
impl NbtResults {
    pub fn matches(&self, document: &Document) -> bool {
        self.path == document.path
            && Arc::ptr_eq(&self.source, &document.current)
            && Arc::ptr_eq(&self.revision, &document.revision)
            && match (&self.definitions, &document.definitions) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}
#[derive(Debug, Clone)]
pub struct PreparedNbt {
    source: Arc<NbtResults>,
    candidate: Arc<Document>,
    pub operations: usize,
    pub objects: usize,
    pub changed: bool,
}
impl PreparedNbt {
    pub fn matches(&self, document: &Document) -> bool {
        self.source.matches(document)
    }
    pub fn candidate(&self) -> &Document {
        &self.candidate
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(invalid("结构 NBT 查找 / 替换已取消"))
    } else {
        Ok(())
    }
}
impl Document {
    /// 选区使用结构局部坐标；实体的完整乘客树只查询一次，命中保留真实根路径。
    pub fn search_nbt_cancellable(
        &self,
        options: NbtSearchOptions,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(usize, usize, u64, usize),
    ) -> Result<NbtResults, WorldError> {
        if options.chunks || !options.entities && !options.block_entities {
            return Err(invalid(
                "独立结构查询需选择实体或方块实体；文件没有区块 NBT",
            ));
        }
        if let Some(crate::nbt_query::ValueMatch::Exact(value)) = &options.filter.value {
            value.payload()?;
        }
        if let Some(region) = options.region {
            if (0..3).any(|axis| {
                region.bounds.minimum[axis] < 0
                    || region.bounds.maximum[axis] >= i32::from(self.current.size[axis])
            }) {
                return Err(invalid("NBT 查询选区超出结构文档尺寸"));
            }
        }
        self.current.check_work(options.budget)?;
        let mut result = NbtResults {
            source: Arc::clone(&self.current),
            revision: self.revision(),
            definitions: self.definitions.clone(),
            path: self.path.clone(),
            retained: 0,
            hits: Vec::new(),
            objects: 0,
            visited: 0,
        };
        let mut filter = options.filter.clone();
        let id_only = filter.name_contains.is_none()
            && filter.name_exact.is_none()
            && filter.kind.is_none()
            && filter.value.is_none();
        if id_only {
            filter.name_exact = Some("id".into());
        }
        let total = if options.block_entities {
            self.current.block_entities.len()
        } else {
            0
        } + if options.entities {
            self.current.entities.len()
        } else {
            0
        };
        let objects = self
            .current
            .block_entities
            .iter()
            .enumerate()
            .filter(|_| options.block_entities)
            .map(|(index, entity)| (Object::BlockEntity(index), entity.position.map(f64::from)))
            .chain(
                self.current
                    .entities
                    .iter()
                    .enumerate()
                    .filter(|_| options.entities)
                    .map(|(index, entity)| (Object::Entity(index), entity.position)),
            );
        for (done, (object, position)) in objects.enumerate() {
            stop(&mut cancelled)?;
            if options
                .region
                .is_some_and(|region| !region.contains_point(position))
            {
                continue;
            }
            let document = self.object(object)?;
            let id = match document.inspect_field("", "id")? {
                Some(nbt_document::Value::String(id)) => id,
                _ => return Err(invalid("结构 NBT 查询对象缺少 String id")),
            };
            let ids = match object {
                Object::BlockEntity(_) => &options.block_entity_ids,
                Object::Entity(_) => &options.entity_ids,
            };
            if !ids.is_empty() && !ids.contains(&id) {
                continue;
            }
            let source = Source::StructureObject {
                path: self.path.clone(),
                object,
            };
            let working = document
                .bytes()
                .len()
                .checked_mul(4)
                .ok_or_else(|| invalid("NBT 查询工作集溢出"))?;
            options
                .budget
                .check(result.retained.saturating_add(working))?;
            let mut cursor = Cursor::new(source, document.bytes().to_vec(), filter.clone())?;
            result.objects += 1;
            loop {
                let batch = cursor.advance(1024, &mut cancelled)?;
                if batch.cancelled {
                    return Err(invalid("结构 NBT 查找已取消"));
                }
                for hit in batch.hits {
                    if id_only && hit.path != "/id" {
                        continue;
                    }
                    let value_bytes = match &hit.value {
                        Some(nbt_document::Value::String(value)) => value.len(),
                        _ => 16,
                    };
                    result.retained = result.retained.saturating_add(
                        std::mem::size_of::<NbtHit>()
                            + hit.path.len()
                            + id.len()
                            + value_bytes
                            + self.path.as_os_str().len() * 4
                            + 512,
                    );
                    options
                        .budget
                        .check(result.retained.saturating_add(working))?;
                    result.hits.push(NbtHit {
                        object,
                        position,
                        id: id.clone(),
                        hit,
                    });
                }
                progress(
                    done,
                    total,
                    result.visited + batch.visited,
                    result.hits.len(),
                );
                if batch.complete {
                    result.visited += batch.visited;
                    break;
                }
            }
        }
        stop(&mut cancelled)?;
        progress(total, total, result.visited, result.hits.len());
        Ok(result)
    }
    /// 先分组核对原始来源，完整私有候选只增加一次文件历史。
    pub fn prepare_nbt_changes_cancellable(
        &self,
        results: Arc<NbtResults>,
        indices: &[usize],
        mutation: NbtMutation,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedNbt, WorldError> {
        self.require_editable()?;
        if !results.matches(self) {
            return Err(invalid("结构 NBT 查询来源已变化，请重新查找"));
        }
        if indices.is_empty() {
            return Err(invalid("请选择查询结果"));
        }
        let mut seen = BTreeSet::new();
        let mut groups: BTreeMap<Object, Vec<&Hit>> = BTreeMap::new();
        for &index in indices {
            stop(&mut cancelled)?;
            if !seen.insert(index) {
                return Err(invalid("查询结果重复选择"));
            }
            let hit = results
                .hits
                .get(index)
                .ok_or_else(|| invalid("查询结果索引无效"))?;
            groups.entry(hit.object).or_default().push(&hit.hit);
        }
        let objects = groups.len();
        let mut edits_entities = false;
        let mut next = (*self.current).clone();
        let mut charge = results
            .retained
            .saturating_add(next.work_bytes()?.saturating_mul(4));
        budget.check(charge)?;
        for (object, hits) in groups {
            stop(&mut cancelled)?;
            let before = self.object(object)?;
            let sha: [u8; 32] = Sha256::digest(before.bytes()).into();
            let source = Source::StructureObject {
                path: self.path.clone(),
                object,
            };
            let mut operations = Vec::new();
            let mut renames = Vec::new();
            for hit in hits {
                if hit.source != source || hit.source_sha != sha {
                    return Err(invalid("结构 NBT 结果不属于当前源对象"));
                }
                let (values, names) = nbt_query::mutation_operations(hit, &mutation)?;
                charge = charge.saturating_add(
                    serde_json::to_vec(&(&values, &names))
                        .map_err(invalid)?
                        .len(),
                );
                budget.check(charge)?;
                operations.extend(values);
                renames.extend(names);
            }
            let operations = nbt_query::changed_operations(&before, &operations)?;
            let raw = if operations.is_empty() {
                before.bytes().to_vec()
            } else {
                nbt_document::patch_batch(before.bytes(), &operations)?
            };
            let after = nbt_document::Document::new(raw)?;
            let renames = nbt_query::changed_operations(&after, &renames)?;
            let raw = if renames.is_empty() {
                after.bytes().to_vec()
            } else {
                nbt_document::patch_batch(after.bytes(), &renames)?
            };
            edits_entities |=
                matches!(object, Object::Entity(_)) && raw.as_slice() != before.bytes();
            Self::replace_object(&mut next, object, &before, &raw)?;
        }
        if edits_entities {
            crate::entity_editing::validate_document_identities(
                &next.entities,
                budget,
                &mut cancelled,
            )?;
        }
        stop(&mut cancelled)?;
        let mut candidate = self.clone();
        let changed = candidate.commit(next, budget, "NBT 批量替换 / 删除")?;
        stop(&mut cancelled)?;
        Ok(PreparedNbt {
            source: results,
            candidate: Arc::new(candidate),
            operations: indices.len(),
            objects,
            changed,
        })
    }
    pub fn commit_nbt_changes_cancellable(
        &mut self,
        prepared: &PreparedNbt,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        self.require_editable()?;
        if !prepared.matches(self) {
            return Err(invalid("结构 NBT 候选来源、修订或定义已变化"));
        }
        stop(&mut cancelled)?;
        *self = (*prepared.candidate).clone();
        Ok(prepared.changed)
    }
}
