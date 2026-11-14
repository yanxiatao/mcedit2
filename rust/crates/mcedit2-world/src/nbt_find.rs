//! 原版维度 NBT 搜索、结果绑定和多对象组合编辑，复用各领域的原生证明。
use super::*;
use crate::{
    nbt_document::{Operation, Value},
    nbt_query::{self, Cursor, Filter, Hit, Source},
    RegionIndex,
};
use std::{fs, sync::Arc};

#[derive(Debug, Clone)]
pub struct NbtSearchOptions {
    pub entities: bool,
    pub block_entities: bool,
    pub chunks: bool,
    pub entity_ids: BTreeSet<String>,
    pub block_entity_ids: BTreeSet<String>,
    pub region: Option<SelectionRegion>,
    pub filter: Filter,
    pub budget: EditWorkBudget,
}
#[derive(Debug, Clone)]
pub struct NbtSearchHit {
    pub target: ObjectTarget,
    pub position: Option<[f64; 3]>,
    pub id: String,
    pub hit: Hit,
}
#[derive(Debug, Clone)]
pub struct NbtSearchResults {
    root: PathBuf,
    dimension: String,
    revision: Arc<()>,
    pub hits: Vec<NbtSearchHit>,
    pub chunks: usize,
    pub visited: u64,
    retained: usize,
}
impl NbtSearchResults {
    pub fn revision(&self) -> Arc<()> {
        Arc::clone(&self.revision)
    }
}
#[derive(Debug, Clone)]
pub enum NbtReplacement {
    Scalar(String),
    Typed(Value),
}
#[derive(Debug, Clone)]
pub enum NbtMutation {
    Replace {
        name: Option<String>,
        value: Option<NbtReplacement>,
    },
    Remove,
}
#[derive(Debug, Clone)]
struct Change {
    target: ObjectTarget,
    source_sha: String,
    operations: Vec<Operation>,
    renames: Vec<Operation>,
}
#[derive(Debug, Clone)]
pub struct NbtBatch {
    root: PathBuf,
    dimension: String,
    revision: Arc<()>,
    changes: Vec<Change>,
    budget: EditWorkBudget,
    candidate: Arc<Session>,
    pub operations: usize,
    pub affected: BTreeSet<(i32, i32)>,
}
impl NbtBatch {
    pub fn candidate(&self) -> Arc<Session> {
        Arc::clone(&self.candidate)
    }
    pub fn objects(&self) -> usize {
        self.changes.len()
    }
}
fn cancelled(stop: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if stop() {
        Err(refuse("NBT 搜索 / 替换已取消"))
    } else {
        Ok(())
    }
}
fn add_regions(
    dir: &Path,
    extension: &str,
    set: &mut BTreeSet<ChunkPos>,
    budget: EditWorkBudget,
    stop: &mut impl FnMut() -> bool,
) -> Result<(), WorldError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if fs::symlink_metadata(dir)?.file_type().is_symlink() {
        return Err(refuse("NBT 搜索不跟随维度目录符号链接"));
    }
    for entry in entries {
        cancelled(stop)?;
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(coords) = name
            .strip_prefix("r.")
            .and_then(|s| s.strip_suffix(&format!(".{extension}")))
        else {
            continue;
        };
        let Some((rx, rz)) = coords.split_once('.') else {
            continue;
        };
        let (Ok(rx), Ok(rz)) = (rx.parse::<i32>(), rz.parse::<i32>()) else {
            continue;
        };
        if entry.file_type()?.is_symlink() {
            return Err(refuse("NBT 搜索不跟随 Region 符号链接"));
        }
        // 原生世界索引将零字节 Region 作为尚未生成槽的空来源。
        if entry.metadata()?.len() == 0 {
            continue;
        }
        let index = RegionIndex::open(entry.path())?;
        for z in 0..32u8 {
            for x in 0..32u8 {
                if index.slot(x, z).is_some() {
                    let cx = rx
                        .checked_mul(32)
                        .and_then(|v| v.checked_add(i32::from(x)))
                        .ok_or_else(|| refuse("Region 坐标溢出"))?;
                    let cz = rz
                        .checked_mul(32)
                        .and_then(|v| v.checked_add(i32::from(z)))
                        .ok_or_else(|| refuse("Region 坐标溢出"))?;
                    set.insert((cx, cz));
                    budget.check(set.len().saturating_mul(64))?;
                }
            }
        }
    }
    Ok(())
}
impl Session {
    /// 枚举实际地形槽和暂存创建；批量工具不跨世界构造连续的虚假区块范围。
    pub(super) fn terrain_chunks_cancellable(
        &self,
        budget: EditWorkBudget,
        mut stop: impl FnMut() -> bool,
    ) -> Result<BTreeSet<ChunkPos>, WorldError> {
        let dim = self
            .world
            .dimensions
            .iter()
            .find(|d| d.name == self.dimension)
            .ok_or_else(|| refuse("未知维度"))?;
        let mut terrain = BTreeSet::new();
        if dim.extension.contains("c.x.z.dat") {
            for (x, z, _) in crate::alpha::find_alpha_chunks(&dim.region_dir)? {
                cancelled(&mut stop)?;
                terrain.insert((x, z));
                budget.check(terrain.len().saturating_mul(64))?;
            }
        } else {
            for extension in ["mca", "mcr"] {
                add_regions(&dim.region_dir, extension, &mut terrain, budget, &mut stop)?;
            }
        }
        terrain.extend(self.creation.keys().copied());
        terrain.extend(self.be_overlay.keys().copied());
        budget.check(terrain.len().saturating_mul(64))?;
        cancelled(&mut stop)?;
        Ok(terrain)
    }
    /// 联合实际地形槽、独立实体槽和暂存实体；保留没有地形槽的真实实体来源。
    pub(super) fn entity_chunks_cancellable(
        &self,
        mut chunks: BTreeSet<ChunkPos>,
        budget: EditWorkBudget,
        mut stop: impl FnMut() -> bool,
    ) -> Result<BTreeSet<ChunkPos>, WorldError> {
        let dim = self
            .world
            .dimensions
            .iter()
            .find(|d| d.name == self.dimension)
            .ok_or_else(|| refuse("未知维度"))?;
        let parent = dim
            .region_dir
            .parent()
            .ok_or_else(|| refuse("维度 Region 目录缺少父目录"))?;
        add_regions(
            &parent.join("entities"),
            "mca",
            &mut chunks,
            budget,
            &mut stop,
        )?;
        chunks.extend(self.entity_overlay.keys().copied());
        budget.check(chunks.len().saturating_mul(64))?;
        cancelled(&mut stop)?;
        Ok(chunks)
    }
    /// 搜索实际磁盘来源与当前暂存来源。仅在完整完成时返回结果，取消或损坏源正常报错。
    pub fn search_nbt(
        &self,
        mut options: NbtSearchOptions,
        mut stop: impl FnMut() -> bool,
        mut progress: impl FnMut(usize, usize, u64, usize),
    ) -> Result<NbtSearchResults, WorldError> {
        if !options.entities && !options.block_entities && !options.chunks {
            return Err(refuse("请选择 NBT 搜索来源"));
        }
        if let Some(crate::nbt_query::ValueMatch::Exact(value)) = &options.filter.value {
            options
                .budget
                .check(value.payload()?.len().saturating_mul(3))?;
        }
        if let Some(region) = options.region {
            options.region = Some(region.resolved(&self.world, &self.dimension)?);
        }
        let terrain = self.terrain_chunks_cancellable(options.budget, &mut stop)?;
        let mut chunks = if options.entities {
            self.entity_chunks_cancellable(terrain.clone(), options.budget, &mut stop)?
        } else {
            terrain.clone()
        };
        if let Some(region) = options.region {
            let b = region.bounds;
            chunks.retain(|&(x, z)| {
                x >= b.minimum[0].div_euclid(16)
                    && x <= b.maximum[0].div_euclid(16)
                    && z >= b.minimum[2].div_euclid(16)
                    && z <= b.maximum[2].div_euclid(16)
            });
        }
        let mut result = NbtSearchResults {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            hits: Vec::new(),
            chunks: chunks.len(),
            visited: 0,
            retained: chunks.len().saturating_mul(192),
        };
        let total = chunks.len();
        for (i, chunk) in chunks.into_iter().enumerate() {
            cancelled(&mut stop)?;
            if terrain.contains(&chunk) && (options.chunks || options.block_entities) {
                let raw = self.inspect_chunk_bytes(chunk, options.budget)?;
                options
                    .budget
                    .check(result.retained.saturating_add(raw.len().saturating_mul(4)))?;
                if options.block_entities {
                    let root =
                        crate::sections::parse_root(&raw).map_err(|e| refuse(e.to_string()))?;
                    let container = root.field("Level").unwrap_or(&root);
                    if let Some(list) = container
                        .field("TileEntities")
                        .or_else(|| container.field("block_entities"))
                    {
                        if list.kind != 9 || list.items.iter().any(|tag| tag.kind != 10) {
                            return Err(refuse("方块实体列表类型无效"));
                        }
                        for tag in &list.items {
                            cancelled(&mut stop)?;
                            let xyz = |name| {
                                tag.require(name, 3)
                                    .and_then(|t| t.i32())
                                    .map_err(|e| refuse(e.to_string()))
                            };
                            let position = (xyz("x")?, xyz("y")?, xyz("z")?);
                            if options.region.is_some_and(|r| !r.contains(position)) {
                                continue;
                            }
                            let id = tag
                                .require("id", 8)
                                .and_then(|t| t.string())
                                .map_err(|e| refuse(e.to_string()))?;
                            if !options.block_entity_ids.is_empty()
                                && !options.block_entity_ids.contains(&id)
                            {
                                continue;
                            }
                            let target = ObjectTarget::BlockEntity { position };
                            self.query_object(
                                &options,
                                target,
                                Some([position.0 as f64, position.1 as f64, position.2 as f64]),
                                id,
                                &mut result,
                                &mut stop,
                            )?;
                        }
                    }
                }
                if options.chunks {
                    self.query_object(
                        &options,
                        ObjectTarget::Chunk { chunk },
                        None,
                        "chunk".into(),
                        &mut result,
                        &mut stop,
                    )?;
                }
            }
            if options.entities {
                let input = self.entity_input(chunk)?;
                options.budget.check(
                    result
                        .retained
                        .saturating_add(input.before.len().saturating_mul(4)),
                )?;
                let catalog =
                    crate::entity_catalog::parse(&input.before, &input.source, chunk.0, chunk.1)?;
                for entity in catalog.entities {
                    // 每棵实体树只遍历一次；乘客字段仍包含在根实体的真实 NBT 中。
                    if entity.path.trim_start_matches('/').split('/').count() != 2 {
                        continue;
                    }
                    if options
                        .region
                        .is_some_and(|r| entity.pos.is_none_or(|p| !r.contains_point(p)))
                    {
                        continue;
                    }
                    let id = entity.id.ok_or_else(|| refuse("NBT 搜索实体缺少 id"))?;
                    if !options.entity_ids.is_empty() && !options.entity_ids.contains(&id) {
                        continue;
                    }
                    self.query_object(
                        &options,
                        ObjectTarget::Entity {
                            chunk,
                            path: entity.path,
                        },
                        entity.pos,
                        id,
                        &mut result,
                        &mut stop,
                    )?;
                }
            }
            progress(i + 1, total, result.visited, result.hits.len());
        }
        cancelled(&mut stop)?;
        Ok(result)
    }
    fn query_object(
        &self,
        options: &NbtSearchOptions,
        target: ObjectTarget,
        position: Option<[f64; 3]>,
        id: String,
        result: &mut NbtSearchResults,
        stop: &mut impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        let draft = self.inspect_object(target.clone(), options.budget)?;
        let bytes = draft.document().bytes();
        options.budget.check(
            result
                .retained
                .saturating_add(bytes.len().saturating_mul(4)),
        )?;
        let source = match &target {
            ObjectTarget::Entity { chunk, path } => Source::Entity {
                dimension: self.dimension.clone(),
                chunk: *chunk,
                entity_path: path.clone(),
            },
            ObjectTarget::BlockEntity { position: p } => Source::BlockEntity {
                dimension: self.dimension.clone(),
                chunk: (p.0.div_euclid(16), p.2.div_euclid(16)),
                position: *p,
            },
            ObjectTarget::Chunk { chunk } => Source::Chunk {
                dimension: self.dimension.clone(),
                chunk: *chunk,
            },
        };
        let mut filter = options.filter.clone();
        let id_only = filter.name_contains.is_none()
            && filter.name_exact.is_none()
            && filter.kind.is_none()
            && filter.value.is_none()
            && !matches!(target, ObjectTarget::Chunk { .. });
        if id_only {
            filter.name_exact = Some("id".into());
        } else if filter.name_contains.is_none()
            && filter.name_exact.is_none()
            && filter.kind.is_none()
            && filter.value.is_none()
        {
            filter.name_contains = Some(String::new());
        }
        let mut cursor = Cursor::new(source, bytes.to_vec(), filter)?;
        if matches!(target, ObjectTarget::Chunk { .. }) {
            cursor.exclude_subtrees(
                [
                    "/Level/Entities",
                    "/Level/TileEntities",
                    "/Entities",
                    "/entities",
                    "/block_entities",
                ]
                .map(str::to_owned)
                .to_vec(),
            );
        }
        loop {
            let batch = cursor.advance(512, &mut *stop)?;
            if batch.cancelled {
                return Err(refuse("NBT 搜索已取消"));
            }
            for hit in batch.hits {
                if id_only && hit.path != "/id" {
                    continue;
                }
                let value_bytes = match &hit.value {
                    Some(Value::String(s)) => s.len(),
                    _ => 16,
                };
                result.retained = result.retained.saturating_add(
                    std::mem::size_of::<NbtSearchHit>()
                        + hit.path.len()
                        + id.len()
                        + value_bytes
                        + 512,
                );
                options.budget.check(
                    result
                        .retained
                        .saturating_add(bytes.len().saturating_mul(4)),
                )?;
                result.hits.push(NbtSearchHit {
                    target: target.clone(),
                    position,
                    id: id.clone(),
                    hit,
                });
            }
            if batch.complete {
                result.visited += batch.visited;
                break;
            }
        }
        Ok(())
    }
    fn replay_nbt_changes(
        &self,
        changes: &[Change],
        budget: EditWorkBudget,
        stop: &mut impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        // 先核对整组初始来源，再让不同对象在私有快照中顺序重放。
        for change in changes {
            cancelled(stop)?;
            let draft = self.inspect_object(change.target.clone(), budget)?;
            if nbt_sha(draft.document().bytes()) != change.source_sha {
                return Err(refuse("NBT 搜索来源已变化，请重新查找"));
            }
        }
        let mut work = self.clone();
        work.begin_recording(budget)?;
        for change in changes {
            cancelled(stop)?;
            let mut draft = work.inspect_object(change.target.clone(), budget)?;
            let operations = nbt_query::changed_operations(draft.document(), &change.operations)?;
            if !operations.is_empty() {
                draft.apply_batch(&operations)?;
            }
            let renames = nbt_query::changed_operations(draft.document(), &change.renames)?;
            if !renames.is_empty() {
                draft.apply_batch(&renames)?;
            }
            if !draft.document().changed() {
                continue;
            }
            let preview = work.prepare_object_edit(&draft, &mut *stop)?;
            work.commit_object_edit(preview)?;
        }
        work.finish_recording()?;
        cancelled(stop)?;
        Ok(work)
    }
    pub fn prepare_nbt_changes(
        &self,
        results: &NbtSearchResults,
        indices: &[usize],
        mutation: NbtMutation,
        budget: EditWorkBudget,
        mut stop: impl FnMut() -> bool,
    ) -> Result<NbtBatch, WorldError> {
        if results.root != self.world.root
            || results.dimension != self.dimension
            || !Arc::ptr_eq(&results.revision, &self.revision())
        {
            return Err(refuse("NBT 查询结果属于旧会话，请重新查找"));
        }
        if indices.is_empty() {
            return Err(refuse("请选择查询结果"));
        }
        let mut seen = BTreeSet::new();
        let mut changes: Vec<Change> = Vec::new();
        let mut target_indices: BTreeMap<ObjectTarget, usize> = BTreeMap::new();
        let mut affected = BTreeSet::new();
        let mut charge = results.retained;
        for &index in indices {
            cancelled(&mut stop)?;
            if !seen.insert(index) {
                return Err(refuse("查询结果重复选择"));
            }
            let hit = results
                .hits
                .get(index)
                .ok_or_else(|| refuse("查询结果索引无效"))?;
            let (operations, renames) = nbt_query::mutation_operations(&hit.hit, &mutation)?;
            charge = charge.saturating_add(compound::proof_bytes(&(&operations, &renames))?);
            budget.check(charge.saturating_mul(2))?;
            let sha = hit
                .hit
                .source_sha
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            if let Some(&target_index) = target_indices.get(&hit.target) {
                let change = &mut changes[target_index];
                change.operations.extend(operations);
                change.renames.extend(renames);
            } else {
                target_indices.insert(hit.target.clone(), changes.len());
                changes.push(Change {
                    target: hit.target.clone(),
                    source_sha: sha,
                    operations,
                    renames,
                });
            }
            affected.insert(match hit.target {
                ObjectTarget::Chunk { chunk } | ObjectTarget::Entity { chunk, .. } => chunk,
                ObjectTarget::BlockEntity { position: p } => {
                    (p.0.div_euclid(16), p.2.div_euclid(16))
                }
            });
        }
        let candidate = self.replay_nbt_changes(&changes, budget, &mut stop)?;
        Ok(NbtBatch {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            changes,
            budget,
            candidate: Arc::new(candidate),
            operations: indices.len(),
            affected,
        })
    }
    pub fn commit_nbt_changes(
        &mut self,
        batch: NbtBatch,
        mut stop: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        if batch.root != self.world.root
            || batch.dimension != self.dimension
            || !Arc::ptr_eq(&batch.revision, &self.revision())
        {
            return Err(refuse("NBT 替换候选已过期"));
        }
        let work = self.replay_nbt_changes(&batch.changes, batch.budget, &mut stop)?;
        *self = work;
        Ok(())
    }
}
