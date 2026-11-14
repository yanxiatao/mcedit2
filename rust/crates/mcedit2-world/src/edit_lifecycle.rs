//! 整区块删除的会话步骤。只悬置所选暂存，不保存会话、历史链或 Region 镜像。
use super::*;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct Generation {
    pub(super) receipt: Arc<crate::lifecycle_history::Deletion>,
    sources: BTreeMap<ChunkPos, Arc<Vec<crate::history_source::FileVersion>>>,
}
impl Generation {
    fn new(
        receipt: Arc<crate::lifecycle_history::Deletion>,
        origin: &crate::history_source::Archive,
    ) -> Result<Self, WorldError> {
        let mut sources = BTreeMap::new();
        for chunk in receipt.chunks() {
            let region = (chunk.0.div_euclid(32), chunk.1.div_euclid(32));
            if let std::collections::btree_map::Entry::Vacant(entry) = sources.entry(region) {
                entry.insert(Arc::new(receipt.freeze_creation_versions(origin, *chunk)?));
            }
        }
        Ok(Self { receipt, sources })
    }
    pub(super) fn sources(&self, chunk: ChunkPos) -> &Arc<Vec<crate::history_source::FileVersion>> {
        self.sources
            .get(&(chunk.0.div_euclid(32), chunk.1.div_euclid(32)))
            .expect("删除代际已冻结对应区域来源")
    }
    fn retained_bytes(&self) -> usize {
        self.sources
            .values()
            .map(|versions| {
                versions
                    .iter()
                    .map(crate::history_source::FileVersion::retained_bytes)
                    .sum::<usize>()
            })
            .sum::<usize>()
            + self.receipt.chunks().len() * 64
            + self.receipt.files().len() * 256
    }
}

#[derive(Debug)]
pub(super) struct Content {
    blocks: BTreeMap<Position, State>,
    creation: BTreeMap<ChunkPos, CreationSnapshot>,
    entities: BTreeMap<ChunkPos, Vec<u8>>,
    entity_sources: BTreeMap<ChunkPos, (String, String)>,
    biomes: BTreeMap<ChunkPos, Vec<u8>>,
    biome_sources: BTreeMap<ChunkPos, String>,
    be: BTreeMap<ChunkPos, Vec<u8>>,
    be_sources: BTreeMap<ChunkPos, String>,
    completion: crate::chunk_completion::State,
    generations: BTreeMap<ChunkPos, Arc<Generation>>,
    pub(super) region_bases: BTreeMap<ChunkPos, super::region_history::Base>,
}
fn take_selected<K: Ord + Clone, V>(
    map: &mut BTreeMap<K, V>,
    chunks: &BTreeSet<ChunkPos>,
    coordinate: impl Fn(&K) -> ChunkPos,
) -> BTreeMap<K, V> {
    let keys = map
        .keys()
        .filter(|key| chunks.contains(&coordinate(key)))
        .cloned()
        .collect::<Vec<_>>();
    keys.into_iter()
        .map(|key| {
            let value = map.remove(&key).expect("已枚举的暂存键存在");
            (key, value)
        })
        .collect()
}
fn block_chunk(position: &Position) -> ChunkPos {
    (position.0.div_euclid(16), position.2.div_euclid(16))
}
impl Content {
    pub(super) fn take(session: &mut Session, chunks: &BTreeSet<ChunkPos>) -> Self {
        let content = Self {
            blocks: take_selected(&mut session.overlay, chunks, block_chunk),
            creation: take_selected(&mut session.creation, chunks, |chunk| *chunk),
            entities: take_selected(&mut session.entity_overlay, chunks, |chunk| *chunk),
            entity_sources: take_selected(&mut session.entity_sources, chunks, |chunk| *chunk),
            biomes: take_selected(&mut session.biome_overlay, chunks, |chunk| *chunk),
            biome_sources: take_selected(&mut session.biome_source, chunks, |chunk| *chunk),
            be: take_selected(&mut session.be_overlay, chunks, |chunk| *chunk),
            be_sources: take_selected(&mut session.be_source, chunks, |chunk| *chunk),
            completion: session.completion.take_chunks(chunks),
            generations: take_selected(&mut session.generations, chunks, |chunk| *chunk),
            region_bases: take_selected(&mut session.region_bases, chunks, |chunk| *chunk),
        };
        session
            .block_chunk_counts
            .retain(|chunk, _| !chunks.contains(chunk));
        session.rebuild_creation_regions();
        content
    }
    /// 地形文件换源不接管独立实体 Region 的暂存与历史。
    pub(super) fn take_terrain(session: &mut Session, chunks: &BTreeSet<ChunkPos>) -> Self {
        let separate = chunks
            .iter()
            .filter(|chunk| {
                session
                    .entity_sources
                    .get(chunk)
                    .is_some_and(|(kind, _)| kind.starts_with("entity-region:"))
            })
            .copied()
            .collect();
        let entities = take_selected(&mut session.entity_overlay, &separate, |chunk| *chunk);
        let sources = take_selected(&mut session.entity_sources, &separate, |chunk| *chunk);
        let selected = Self::take(session, chunks);
        session.entity_overlay.extend(entities);
        session.entity_sources.extend(sources);
        selected
    }
    pub(super) fn restore(&self, session: &mut Session, chunks: &BTreeSet<ChunkPos>) {
        session.overlay.extend(
            self.blocks
                .iter()
                .map(|(position, state)| (*position, state.clone())),
        );
        session.creation.extend(
            self.creation
                .iter()
                .map(|(chunk, snapshot)| (*chunk, snapshot.clone())),
        );
        session.entity_overlay.extend(
            self.entities
                .iter()
                .map(|(chunk, raw)| (*chunk, raw.clone())),
        );
        session.entity_sources.extend(
            self.entity_sources
                .iter()
                .map(|(chunk, source)| (*chunk, source.clone())),
        );
        session
            .biome_overlay
            .extend(self.biomes.iter().map(|(chunk, raw)| (*chunk, raw.clone())));
        session.biome_source.extend(
            self.biome_sources
                .iter()
                .map(|(chunk, source)| (*chunk, source.clone())),
        );
        session
            .be_overlay
            .extend(self.be.iter().map(|(chunk, raw)| (*chunk, raw.clone())));
        session.be_source.extend(
            self.be_sources
                .iter()
                .map(|(chunk, source)| (*chunk, source.clone())),
        );
        session.completion.restore_chunks(chunks, &self.completion);
        session.generations.extend(
            self.generations
                .iter()
                .map(|(chunk, generation)| (*chunk, Arc::clone(generation))),
        );
        session.region_bases.extend(
            self.region_bases
                .iter()
                .map(|(chunk, base)| (*chunk, base.clone())),
        );
        for position in self.blocks.keys() {
            *session
                .block_chunk_counts
                .entry(block_chunk(position))
                .or_default() += 1;
        }
        session.rebuild_creation_regions();
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.blocks
            .values()
            .map(|state| std::mem::size_of::<Position>() + compound::state_bytes(state))
            .sum::<usize>()
            + self
                .entities
                .values()
                .chain(self.biomes.values())
                .chain(self.be.values())
                .map(Vec::len)
                .sum::<usize>()
            + self
                .creation
                .values()
                .map(|snapshot| {
                    snapshot.base.len()
                        + snapshot
                            .source_versions
                            .iter()
                            .map(crate::history_source::FileVersion::retained_bytes)
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .entity_sources
                .values()
                .map(|(label, sha)| label.len() + sha.len() + 64)
                .sum::<usize>()
            + self
                .biome_sources
                .values()
                .chain(self.be_sources.values())
                .map(|sha| sha.len() + 64)
                .sum::<usize>()
            + self.completion.retained_bytes()
            + self.generations.len() * 64
            + super::region_history::bases_bytes(self.region_bases.values())
    }
}

#[derive(Debug)]
pub(super) struct Step {
    generation: Arc<Generation>,
    before: Content,
    hashes: BTreeMap<ChunkPos, [u8; 32]>,
}
impl Step {
    pub(super) fn generation(&self) -> &Arc<Generation> {
        &self.generation
    }
    pub(super) fn chunks(&self) -> &BTreeSet<ChunkPos> {
        self.generation.receipt.chunks()
    }
    pub(super) fn previous_region_bases(&self) -> &BTreeMap<ChunkPos, super::region_history::Base> {
        &self.before.region_bases
    }
    pub(super) fn history_entry(&self) -> crate::history::Entry {
        crate::history::Entry {
            label: "删除区块".into(),
            detail: format!(
                "{} · {} 个区块",
                self.generation.receipt.dimension(),
                self.chunks().len()
            ),
        }
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.before.retained_bytes() + self.generation.retained_bytes() + self.hashes.len() * 64
    }
}

impl Session {
    pub fn check_chunk_history_capacity(&self, additional: usize) -> Result<(), WorldError> {
        if self.recording.is_some()
            || self
                .undo
                .len()
                .checked_add(additional)
                .is_none_or(|length| length > MAX_HISTORY)
        {
            return Err(refuse("区块事务需要新的历史槽，请先处理当前操作历史"));
        }
        Ok(())
    }
    /// 完整提交后的来源交接：删除与重填标记进入正式历史，其他来源只推进磁盘头。
    pub fn observe_chunk_transaction(
        mut self,
        archive: crate::history_source::Archive,
        revision: &Arc<()>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        if !Arc::ptr_eq(revision, &self.revision)
            || self.world.root.as_path() != archive.world_root()
        {
            return Err(refuse("区块事务交接的归属或修订改变"));
        }
        let receipts = archive
            .deletions()
            .filter(|receipt| receipt.dimension() == self.dimension)
            .cloned()
            .collect::<Vec<_>>();
        let repopulations = archive
            .repopulations()
            .filter(|proof| proof.dimension == self.dimension)
            .cloned()
            .collect::<Vec<_>>();
        self.check_chunk_history_capacity(receipts.len() + usize::from(!repopulations.is_empty()))?;
        if receipts.is_empty() && repopulations.is_empty() {
            self.observe_saved(archive, revision)?;
            return Ok(self);
        }
        for receipt in receipts {
            let current = self.revision();
            self = self.observe_deletion(&archive, receipt, &current, budget, &mut cancelled)?;
        }
        if !repopulations.is_empty() {
            self = self.observe_repopulation(&archive, &repopulations, budget, &mut cancelled)?;
        }
        Ok(self)
    }
    fn observe_repopulation(
        &self,
        archive: &crate::history_source::Archive,
        proofs: &[Arc<crate::chunk_lifecycle::RepopProof>],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        use crate::nbt_document::{Document, Operation, Value};
        let chunks = proofs
            .iter()
            .flat_map(|proof| proof.chunks.iter().copied())
            .collect::<BTreeSet<_>>();
        if chunks.iter().any(|chunk| self.dirty_chunks.contains(chunk)) {
            return Err(refuse("区块重填标记与尚未保存的编辑重叠"));
        }
        let origin = match &self.origin {
            Some(previous) => previous.merged(archive)?,
            None => archive.clone(),
        };
        origin.verify_current()?;
        let mut work = self.clone();
        work.origin = Some(origin.clone());
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, Some(&origin))?;
        let mut transaction_reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, Some(archive))?;
        work.begin_recording(budget)?;
        for proof in proofs {
            for &chunk in &proof.chunks {
                if cancelled() {
                    return Err(refuse("重填标记会话接入已取消"));
                }
                let physical_before = transaction_reader
                    .snapshot_chunk_bytes(chunk.0, chunk.1)?
                    .ok_or_else(|| refuse("重填标记缺少事务原始区块"))?;
                if crate::chunk_lifecycle::populated_value(&physical_before)? == proof.populated {
                    return Err(refuse("重填标记证明没有实际改变原字段"));
                }
                let (original, source_sha) =
                    work.terrain_payload_base_cached(&mut reader, chunk)?;
                let before = work.be_overlay.get(&chunk).cloned().unwrap_or(original);
                crate::chunk_lifecycle::populated_value(&before)?;
                if Document::new(before.clone())?.inspect("/Level/TerrainPopulated")?
                    != Document::new(physical_before)?.inspect("/Level/TerrainPopulated")?
                {
                    return Err(refuse("重填标记的会话字段与事务原始来源不同"));
                }
                let operations = vec![Operation::Replace {
                    path: "/Level/TerrainPopulated".into(),
                    value: Value::Byte(if proof.populated { 1 } else { 0 }),
                }];
                let metadata = crate::chunk_metadata::Proof::new(&before, chunk, &[operations])?;
                let after = metadata.replay(&before)?;
                let preview = BeNbtPreview {
                    root: self.world.root.clone(),
                    dimension: self.dimension.clone(),
                    chunk,
                    source_sha,
                    before,
                    after,
                    proof: BeNbtProof::ChunkMetadata(metadata),
                    completion: None,
                };
                let group = EditGroup::BlockEntityNbt(preview);
                budget.check(
                    reader
                        .frozen_bytes()
                        .saturating_add(transaction_reader.frozen_bytes())
                        .saturating_add(
                            work.recording.as_ref().expect("已开始重填组合记录").charged,
                        )
                        .saturating_add(compound::group_bytes(&group)?),
                )?;
                let EditGroup::BlockEntityNbt(preview) = group else {
                    unreachable!("刚构造的区块元数据步骤")
                };
                work.apply_be_nbt_preview(preview)?;
            }
        }
        work.finish_recording()?;
        work.accept_committed_chunk_range(&chunks)?;
        if cancelled() {
            return Err(refuse("重填标记会话发布已取消"));
        }
        Ok(work)
    }
    /// 只推进已提交区块的保存点；同维度未参与的本地编辑仍按原保存点计算。
    pub(super) fn accept_committed_chunk_range(
        &mut self,
        chunks: &BTreeSet<ChunkPos>,
    ) -> Result<(), WorldError> {
        let saved = self.capture_saved_chunks(self.origin.as_ref().expect("已接入区块事务来源"))?;
        let mut saved_chunks = (*self.saved_chunks).clone();
        let mut saved_hashes = (*self.saved_hashes).clone();
        for chunk in chunks {
            if let Some(value) = saved.get(chunk) {
                saved_chunks.insert(*chunk, value.clone());
            }
            if let Some(value) = self.content_hashes.get(chunk) {
                saved_hashes.insert(*chunk, *value);
            } else {
                saved_hashes.remove(chunk);
            }
            self.dirty_chunks.remove(chunk);
        }
        self.saved_chunks = Arc::new(saved_chunks);
        self.saved_hashes = Arc::new(saved_hashes);
        self.changed_source_chunks.extend(chunks.iter().copied());
        self.note_change();
        Ok(())
    }
    /// 当前代际的领域证明必须位于最后一次实际删除之后，旧步骤仍留给历史恢复。
    pub(super) fn history_starts(&self) -> BTreeMap<ChunkPos, usize> {
        let mut starts = BTreeMap::new();
        for (index, group) in self.undo.iter().enumerate() {
            if let EditGroup::Deletion(step) = group {
                for chunk in step.chunks() {
                    starts.insert(*chunk, index);
                }
            }
            if let EditGroup::Region(step) = group {
                for chunk in step.chunks() {
                    starts.insert(*chunk, index);
                }
            }
        }
        starts
    }
    pub(super) fn capture_creation_in_generation(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        chunk: ChunkPos,
        profile: crate::chunk_creation::ChunkCreationProfile,
    ) -> Result<CreationSnapshot, WorldError> {
        let reader = match self.generations.get(&chunk) {
            Some(generation) => reader.snapshot_source(generation.sources(chunk))?,
            None => reader,
        };
        CreationSnapshot::capture(reader, chunk, profile)
    }
    pub(super) fn restore_deletion_step(&mut self, after: bool) -> Result<bool, WorldError> {
        let step = match if after {
            self.redo.last()
        } else {
            self.undo.last()
        } {
            Some(EditGroup::Deletion(step)) => Arc::clone(step),
            _ => return Ok(false),
        };
        self.restore_deletion(&step, after)?;
        let (from, ids, to, to_ids) = if after {
            (
                &mut self.redo,
                &mut self.redo_ids,
                &mut self.undo,
                &mut self.undo_ids,
            )
        } else {
            (
                &mut self.undo,
                &mut self.undo_ids,
                &mut self.redo,
                &mut self.redo_ids,
            )
        };
        to.push(from.pop().expect("已核对删除步骤"));
        to_ids.push(ids.pop().expect("删除步骤具有稳定身份"));
        self.changed_source_chunks
            .extend(step.chunks().iter().copied());
        self.note_change();
        Ok(true)
    }
    pub(super) fn bind_generation_source(&mut self) -> Result<(), WorldError> {
        let origin = self
            .origin
            .as_ref()
            .ok_or_else(|| refuse("删除代际缺少真实事务来源"))?;
        let mut grouped = BTreeMap::<usize, (&Generation, BTreeSet<ChunkPos>)>::new();
        for (chunk, generation) in &self.generations {
            grouped
                .entry(Arc::as_ptr(generation) as usize)
                .or_insert_with(|| (generation, BTreeSet::new()))
                .1
                .insert(*chunk);
        }
        let mut masks = crate::history_source::Masks::default();
        let mut regions = crate::history_source::RegionSources::new();
        for (chunk, base) in &self.region_bases {
            regions
                .entry(base.version.path().to_owned())
                .or_default()
                .insert(*chunk, Arc::clone(&base.version));
        }
        for (generation, chunks) in grouped.values() {
            generation
                .receipt
                .mask_into(&self.world.root, chunks, &mut masks)?;
        }
        self.origin = Some(
            origin
                .with_region_sources(Arc::new(regions))?
                .with_masks(masks),
        );
        Ok(())
    }
    /// 后台已提交的删除作为正式步骤接入。调用方仍按冻结修订交接，不覆盖活动编辑。
    pub(crate) fn observe_deletion(
        &self,
        archive: &crate::history_source::Archive,
        receipt: Arc<crate::lifecycle_history::Deletion>,
        revision: &Arc<()>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        if self.world.root.as_path() != archive.world_root()
            || receipt.dimension() != self.dimension
            || !Arc::ptr_eq(revision, &self.revision)
            || self.recording.is_some()
        {
            return Err(refuse("区块删除与会话的世界、维度或修订不同"));
        }
        if receipt
            .chunks()
            .iter()
            .any(|chunk| self.dirty_chunks.contains(chunk))
        {
            return Err(refuse("区块删除与尚未保存的编辑重叠"));
        }
        if cancelled() {
            return Err(refuse("删除会话接入已取消"));
        }
        self.ensure_history_slot(false)?;
        let origin = match &self.origin {
            Some(previous) => previous.merged(archive)?,
            None => archive.clone(),
        };
        origin.verify_current()?;
        let generation = Arc::new(Generation::new(receipt, &origin)?);
        let chunks = generation.receipt.chunks();
        let mut work = self.clone();
        let hashes = self
            .content_hashes
            .iter()
            .filter(|(chunk, _)| chunks.contains(*chunk))
            .map(|(chunk, hash)| (*chunk, *hash))
            .collect();
        let before = Content::take(&mut work, chunks);
        work.origin = Some(origin);
        for chunk in chunks {
            work.generations.insert(*chunk, Arc::clone(&generation));
        }
        work.bind_generation_source()?;
        let step = Arc::new(Step {
            generation,
            before,
            hashes,
        });
        budget.check(step.retained_bytes())?;
        work.record_edit(EditGroup::Deletion(step))?;
        work.note_change();
        // 删除是已发生的物理提交；仅推进本次范围的保存点，不抹掉其他本地修改。
        let saved_range = work
            .undo
            .last()
            .and_then(|group| {
                if let EditGroup::Deletion(step) = group {
                    Some(step.chunks())
                } else {
                    None
                }
            })
            .expect("刚记录删除步骤")
            .clone();
        work.accept_committed_chunk_range(&saved_range)?;
        if cancelled() {
            return Err(refuse("删除会话发布已取消"));
        }
        Ok(work)
    }
    pub(super) fn restore_deletion(&mut self, step: &Step, after: bool) -> Result<(), WorldError> {
        let chunks = step.chunks();
        if after {
            for chunk in chunks {
                if self.content_hashes.get(chunk) != step.hashes.get(chunk) {
                    return Err(refuse("删除重做的区块内容与原步骤不同"));
                }
                let previous = step.before.generations.get(chunk);
                let current = self.generations.get(chunk);
                if !match (current, previous) {
                    (None, None) => true,
                    (Some(current), Some(previous)) => Arc::ptr_eq(current, previous),
                    _ => false,
                } {
                    return Err(refuse("删除重做的区块代际不同"));
                }
            }
        } else {
            for chunk in chunks {
                if !self
                    .generations
                    .get(chunk)
                    .is_some_and(|generation| Arc::ptr_eq(generation, &step.generation))
                    || self.creation.contains_key(chunk)
                    || self.block_chunk_counts.contains_key(chunk)
                    || self.be_overlay.contains_key(chunk)
                    || self.biome_overlay.contains_key(chunk)
                    || self.entity_overlay.contains_key(chunk)
                {
                    return Err(refuse("删除撤销前仍有后续区块修改"));
                }
            }
        }
        Content::take(self, chunks);
        if after {
            for chunk in chunks {
                self.generations
                    .insert(*chunk, Arc::clone(&step.generation));
            }
        } else {
            step.before.restore(self, chunks);
        }
        self.bind_generation_source()?;
        Ok(())
    }
}
