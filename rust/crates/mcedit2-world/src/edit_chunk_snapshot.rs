//! 从历史来源重放当前会话，再以实际磁盘为写入基线恢复完整槽位。
use super::*;
use crate::{chunk_postprocess::Policy, chunk_snapshot::Storage, entity_catalog};
use sha2::Digest as _;

type Digest = [u8; 32];
const FLAGS: [&str; 2] = ["isLightOn", "LightPopulated"];

/// 只保留逻辑摘要；区块载荷继续读取事务备份，不复制整个保存点。
#[derive(Debug, Clone)]
pub(super) struct SavedChunk {
    removed: bool,
    base: Option<Digest>,
    geometry: Option<Digest>,
    flags: [Option<Digest>; 2],
    status: Option<Digest>,
    payload: Option<Digest>,
    blocks: BTreeMap<Position, Digest>,
    created_slot: bool,
    sections: BTreeSet<i8>,
    be: Option<Digest>,
    entities: Option<Digest>,
    biome: Option<Digest>,
    creation: Option<Digest>,
}

fn parent(raw: &[u8]) -> Result<&'static str, WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| refuse(error.to_string()))?;
    Ok(if root.field("Level").is_some() {
        "/Level"
    } else {
        ""
    })
}

fn light_flags(raw: &[u8]) -> Result<[Option<Digest>; 2], WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| refuse(error.to_string()))?;
    let container = root.field("Level").unwrap_or(&root);
    let mut result = [None; 2];
    for (index, name) in FLAGS.iter().enumerate() {
        let mut fields = container.fields.iter().filter(|(key, _)| key == name);
        if let Some((_, tag)) = fields.next() {
            if tag.kind != 1 || fields.next().is_some() {
                return Err(refuse(format!("光照标记 {name} 类型无效或重复")));
            }
            result[index] = Some(Sha256::digest(tag.raw).into());
        }
    }
    Ok(result)
}
fn status_stamp(raw: &[u8]) -> Result<Option<Digest>, WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| refuse(error.to_string()))?;
    if root.field("Level").is_some() {
        return Ok(None);
    }
    let mut fields = root.fields.iter().filter(|(key, _)| *key == "Status");
    let value = fields.next().map(|(_, value)| value);
    if fields.next().is_some() {
        return Err(refuse("区块完成状态重复"));
    }
    value
        .map(|value| {
            value.string().map_err(|error| refuse(error.to_string()))?;
            Ok(Sha256::digest(value.raw).into())
        })
        .transpose()
}

/// 按 Y 关联区段，不能假定保存前后列表索引不变。
fn sections(raw: &[u8], name: &str) -> Result<BTreeMap<i8, (usize, Vec<u8>)>, WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| refuse(error.to_string()))?;
    let container = root.field("Level").unwrap_or(&root);
    let Some(list) = container.field(name) else {
        return Ok(BTreeMap::new());
    };
    if list.kind != 9 {
        return Err(refuse("光照区段不是列表"));
    }
    let mut result = BTreeMap::new();
    for (index, section) in list.items.iter().enumerate() {
        let y = section
            .require("Y", 1)
            .and_then(crate::sections::Tag::byte)
            .map_err(|error| refuse(error.to_string()))?;
        if result.insert(y, (index, section.raw.to_vec())).is_some() {
            return Err(refuse("光照区段 Y 重复"));
        }
    }
    Ok(result)
}

/// 几何未变时保留实际保存的派生数据；手工编辑过的完成标记仍属于当前内容。
fn preserve_lighting(
    target: &[u8],
    actual: &[u8],
    saved_flags: &[Option<Digest>; 2],
) -> Result<Vec<u8>, WorldError> {
    let flags = light_flags(target)?;
    let mut output = preserve_derived_arrays(target, actual)?;
    for (index, name) in FLAGS.iter().enumerate() {
        if flags[index] == saved_flags[index] {
            output = crate::shared_chunk_save::copy(&output, name, actual)?;
        }
    }
    Ok(output)
}
/// 只更新不可编辑的数组，不把显示中的完成标记冒充历史字段证明的基底。
fn preserve_derived_arrays(target: &[u8], actual: &[u8]) -> Result<Vec<u8>, WorldError> {
    let mut output = target.to_vec();
    for name in ["HeightMap", "Heightmaps", "BlockLight", "SkyLight"] {
        output = crate::shared_chunk_save::copy(&output, name, actual)?;
    }
    let base = parent(target)?;
    let name = if base.is_empty() {
        "sections"
    } else {
        "Sections"
    };
    let target_sections = sections(target, name)?;
    let actual_sections = sections(actual, name)?;
    if target_sections.is_empty() {
        return crate::shared_chunk_save::copy(&output, name, actual);
    }
    let empty = [10, 0, 0, 0];
    for (&y, (index, _)) in &target_sections {
        let target_parent = format!("{base}/{name}/{index}");
        for field in ["BlockLight", "SkyLight"] {
            output = if let Some((source_index, _)) = actual_sections.get(&y) {
                crate::nbt_document::copy_raw_field(
                    &output,
                    &target_parent,
                    field,
                    actual,
                    &format!("{base}/{name}/{source_index}"),
                )?
            } else {
                crate::nbt_document::copy_raw_field(&output, &target_parent, field, &empty, "")?
            };
        }
    }
    let mut length = target_sections.len();
    for (y, (_, payload)) in actual_sections {
        if !target_sections.contains_key(&y) {
            output = crate::nbt_document::insert_raw_compound_list_item(
                &output,
                &format!("{base}/{name}"),
                length,
                &payload,
            )?;
            length += 1;
        }
    }
    Ok(output)
}

fn add(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
fn raw_hash(raw: Option<&Vec<u8>>) -> Option<Digest> {
    raw.map(|raw| Sha256::digest(raw).into())
}

impl Session {
    pub(super) fn display_derived_payload(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        chunk: ChunkPos,
        raw: Vec<u8>,
    ) -> Result<Vec<u8>, WorldError> {
        if self.origin.is_none()
            || self.pending_geometry.contains(&chunk)
            || self.region_base_changed(chunk)
        {
            return Ok(raw);
        }
        let actual = reader
            .current_chunk_bytes(chunk.0, chunk.1)?
            .ok_or_else(|| refuse("区块派生数据显示的实际磁盘槽已缺失"))?;
        let source = decode_sections(&raw).map_err(|error| refuse(error.to_string()))?;
        let current =
            crate::block_lookup::checked_original_chunk_bytes(Some(&actual), chunk.0, chunk.1)?
                .expect("实际派生数据载荷已存在");
        if source.layout != current.layout || source.data_version != current.data_version {
            return Err(refuse("区块派生数据显示的历史与当前格式不同"));
        }
        preserve_derived_arrays(&raw, &actual)
    }
    pub(super) fn pending_geometry_chunk(&self, chunk: ChunkPos) -> bool {
        self.geometry_hash(chunk)
            != self
                .saved_chunks
                .get(&chunk)
                .and_then(|saved| saved.geometry)
    }
    pub(super) fn display_completion(
        &self,
        chunk: ChunkPos,
        source: &Chunk,
    ) -> Result<display::Completion, WorldError> {
        use crate::chunk_completion::Field;
        let mut completion = self.raw_display_completion(chunk, source)?;
        let flag = if matches!(source.layout, Layout::Modern118) {
            Field::LightOn
        } else {
            Field::LightPopulated
        };
        if self.completion.tracked(chunk, flag) {
            completion.ready = self
                .completion
                .desired_value(chunk, flag)
                .map(|value| value.flag());
        }
        if matches!(source.layout, Layout::Modern118)
            && self.completion.tracked(chunk, Field::Status)
        {
            completion.status = self
                .completion
                .desired_value(chunk, Field::Status)
                .map(|value| value.status());
        }
        Ok(completion)
    }
    fn raw_display_completion(
        &self,
        chunk: ChunkPos,
        source: &Chunk,
    ) -> Result<display::Completion, WorldError> {
        let Some(raw) = self
            .be_overlay
            .get(&chunk)
            .or_else(|| self.creation.get(&chunk).map(|created| &created.base))
        else {
            return Ok(display::Completion::default());
        };
        let root = crate::sections::parse_root(raw).map_err(|error| refuse(error.to_string()))?;
        let container = root.field("Level").unwrap_or(&root);
        let index = usize::from(!matches!(source.layout, Layout::Modern118));
        let value = container
            .field(FLAGS[index])
            .map(|value| value.byte())
            .transpose()
            .map_err(|error| refuse(error.to_string()))?
            .map(|value| value != 0);
        let changed = match self.saved_chunks.get(&chunk) {
            Some(saved) => light_flags(raw)?[index] != saved.flags[index],
            None => value != source.light.ready,
        };
        let mut completion = display::Completion {
            ready: changed.then_some(value),
            status: None,
        };
        if matches!(source.layout, Layout::Modern118) {
            let value = container
                .field("Status")
                .map(|value| value.string())
                .transpose()
                .map_err(|error| refuse(error.to_string()))?;
            let changed = match self.saved_chunks.get(&chunk) {
                Some(saved) => status_stamp(raw)? != saved.status,
                None => value.as_deref() != source.light.status(),
            };
            completion.status = changed.then_some(value);
        }
        Ok(completion)
    }
    pub(super) fn pending_entity_chunk(&self, chunk: ChunkPos) -> bool {
        self.lifecycle_slot_changed(chunk)
            || self.region_base_changed(chunk)
            || raw_hash(self.entity_overlay.get(&chunk))
                != self
                    .saved_chunks
                    .get(&chunk)
                    .and_then(|saved| saved.entities)
    }
    pub(super) fn entity_identity_chunks(&self) -> BTreeSet<ChunkPos> {
        self.entity_overlay
            .keys()
            .copied()
            .chain(self.saved_chunks.iter().filter_map(|(chunk, saved)| {
                (saved.entities.is_some() || saved.removed).then_some(*chunk)
            }))
            .chain(self.generations.keys().copied())
            .chain(self.region_bases.keys().copied())
            .collect()
    }
    pub(super) fn pending_be_chunk(&self, chunk: ChunkPos) -> bool {
        if self.region_base_changed(chunk) {
            return self.payload_hash(chunk)
                != self
                    .saved_chunks
                    .get(&chunk)
                    .and_then(|saved| saved.payload)
                || self.completion.has_pending(chunk);
        }
        raw_hash(self.be_overlay.get(&chunk))
            != self.saved_chunks.get(&chunk).and_then(|saved| saved.be)
            || (self.completion.has_pending(chunk)
                && (!self.generations.contains_key(&chunk) || self.creation.contains_key(&chunk)))
    }
    pub(super) fn scalar_checkpoint_metadata_unchanged(
        &self,
        chunk: ChunkPos,
        original: &[u8],
        logical: &[u8],
        actual: &[u8],
    ) -> Result<(), WorldError> {
        if self.completion.has_pending(chunk) {
            return Err(refuse(
                "标量 BE 日志不承载完成字段的显式赋值，请使用完整保存",
            ));
        }
        let saved_flags = match self.saved_chunks.get(&chunk) {
            Some(saved) => saved.flags,
            None => light_flags(original)?,
        };
        if light_flags(logical)? != saved_flags
            || crate::chunk_metadata::scalar_checkpoint_metadata(logical)?
                != crate::chunk_metadata::scalar_checkpoint_metadata(actual)?
        {
            return Err(refuse(
                "标量 BE 日志不能包含未保存的区块元数据，请使用完整保存",
            ));
        }
        Ok(())
    }
    pub(super) fn lifecycle_slot_changed(&self, chunk: ChunkPos) -> bool {
        self.generations.contains_key(&chunk)
            != self
                .saved_chunks
                .get(&chunk)
                .is_some_and(|saved| saved.removed)
            || self
                .creation
                .get(&chunk)
                .is_some_and(|created| created.source_chunk_sha.is_none())
                != self
                    .saved_chunks
                    .get(&chunk)
                    .is_some_and(|saved| saved.created_slot)
    }
    pub(super) fn restores_saved_chunk(&self, chunk: ChunkPos) -> bool {
        self.saved_chunks
            .get(&chunk)
            .is_some_and(|saved| saved.removed)
            && !self.generations.contains_key(&chunk)
    }
    pub(super) fn has_lifecycle_slot_changes(&self) -> bool {
        self.dirty_chunks
            .iter()
            .any(|chunk| self.lifecycle_slot_changed(*chunk) || self.region_base_changed(*chunk))
    }
    pub(super) fn region_base_changed(&self, chunk: ChunkPos) -> bool {
        self.region_bases.get(&chunk).map(|base| base.nbt)
            != self.saved_chunks.get(&chunk).and_then(|saved| saved.base)
    }
    /// 外部方块日志只描述最近保存点之后的实际方块差异，不重导已保存的暂存基底。
    pub(super) fn pending_patch_blocks(&self) -> Result<BTreeMap<Position, State>, WorldError> {
        if self.has_lifecycle_slot_changes() {
            return Err(refuse("方块日志不承载区块删除或恢复，请使用完整混合保存"));
        }
        if let Some(origin) = &self.origin {
            origin.verify_current()?;
        }
        let mut reader = self.source_reader()?;
        let mut result = BTreeMap::new();
        for &chunk in &self.dirty_chunks {
            if self.completion.has_pending(chunk) {
                return Err(refuse("方块日志不能表示完成字段的显式赋值，请使用完整保存"));
            }
            let saved = self.saved_chunks.get(&chunk);
            if raw_hash(self.be_overlay.get(&chunk)) != saved.and_then(|saved| saved.be)
                || raw_hash(self.entity_overlay.get(&chunk))
                    != saved.and_then(|saved| saved.entities)
                || raw_hash(self.biome_overlay.get(&chunk)) != saved.and_then(|saved| saved.biome)
            {
                return Err(refuse(
                    "方块日志不能包含未保存的 BE、实体、区块元数据或群系；请使用完整结构导出",
                ));
            }
            let creation: Option<Digest> = self
                .creation
                .get(&chunk)
                .map(|created| Sha256::digest(&created.base).into());
            if creation != saved.and_then(|saved| saved.creation) {
                return Err(refuse("方块日志不能表示区块 / 区段的创建或移除"));
            }
            let current = self.block_hashes(chunk);
            let previous = saved.map(|saved| &saved.blocks);
            let positions = current
                .keys()
                .chain(previous.into_iter().flat_map(|blocks| blocks.keys()))
                .copied()
                .collect::<BTreeSet<_>>();
            let (base, _) = self.visible_chunk_base_cached(&mut reader, chunk.0, chunk.1)?;
            for position in positions {
                if current.get(&position) == previous.and_then(|blocks| blocks.get(&position)) {
                    continue;
                }
                let state = self
                    .overlay
                    .get(&position)
                    .or_else(|| {
                        base.as_ref()?.block(
                            position.0.rem_euclid(16) as u8,
                            position.1,
                            position.2.rem_euclid(16) as u8,
                        )
                    })
                    .ok_or_else(|| refuse("待导出方块的真实历史基底不存在"))?;
                result.insert(position, state.clone());
            }
        }
        Ok(result)
    }
    fn block_hashes(&self, chunk: ChunkPos) -> BTreeMap<Position, Digest> {
        self.display_changes_in_chunk(chunk.0, chunk.1)
            .map(|(position, state)| {
                let mut hash = Sha256::new();
                add(&mut hash, state.name.as_bytes());
                hash.update((state.properties.len() as u64).to_be_bytes());
                for (key, value) in &state.properties {
                    add(&mut hash, key.as_bytes());
                    add(&mut hash, value.as_bytes());
                }
                (position, hash.finalize().into())
            })
            .collect()
    }
    pub(super) fn pending_blocks_in_chunk(&self, chunk: ChunkPos) -> usize {
        let current = self.block_hashes(chunk);
        let saved = self.saved_chunks.get(&chunk).map(|saved| &saved.blocks);
        current
            .iter()
            .filter(|(position, hash)| saved.and_then(|saved| saved.get(*position)) != Some(*hash))
            .count()
            + saved.map_or(0, |saved| {
                saved
                    .keys()
                    .filter(|position| !current.contains_key(*position))
                    .count()
            })
    }
    pub(super) fn unsaved_creation(&self, chunk: ChunkPos, y: i32) -> bool {
        let Some(created) = self.creation.get(&chunk) else {
            return false;
        };
        let saved = self.saved_chunks.get(&chunk);
        (created.source_chunk_sha.is_none() && !saved.is_some_and(|saved| saved.created_slot))
            || created.section_proofs.iter().any(|proof| {
                i32::from(proof.section_y) == y.div_euclid(16)
                    && !saved.is_some_and(|saved| saved.sections.contains(&proof.section_y))
            })
    }
    pub(super) fn geometry_hash(&self, chunk: ChunkPos) -> Option<Digest> {
        if !self.creation.contains_key(&chunk)
            && !self.block_chunk_counts.contains_key(&chunk)
            && !self.generations.contains_key(&chunk)
        {
            return self.region_bases.get(&chunk).and_then(|base| base.geometry);
        }
        let mut hash = Sha256::new();
        if let Some(base) = self.region_bases.get(&chunk).and_then(|base| base.geometry) {
            hash.update([254]);
            hash.update(base);
        }
        if self.generations.contains_key(&chunk) {
            hash.update([255]);
        }
        if let Some(created) = self.creation.get(&chunk) {
            hash.update([0]);
            add(&mut hash, &created.base);
        }
        for (position, state) in self.display_changes_in_chunk(chunk.0, chunk.1) {
            hash.update([1]);
            hash.update(position.0.to_be_bytes());
            hash.update(position.1.to_be_bytes());
            hash.update(position.2.to_be_bytes());
            add(&mut hash, state.name.as_bytes());
            hash.update((state.properties.len() as u64).to_be_bytes());
            for (key, value) in &state.properties {
                add(&mut hash, key.as_bytes());
                add(&mut hash, value.as_bytes());
            }
        }
        Some(hash.finalize().into())
    }

    pub(super) fn payload_hash(&self, chunk: ChunkPos) -> Option<Digest> {
        if !self.be_overlay.contains_key(&chunk) && !self.entity_overlay.contains_key(&chunk) {
            return self.region_bases.get(&chunk).and_then(|base| base.payload);
        }
        let mut hash = Sha256::new();
        if let Some(base) = self.region_bases.get(&chunk).and_then(|base| base.payload) {
            hash.update([254]);
            hash.update(base);
        }
        for (kind, overlay) in [(0, &self.be_overlay), (1, &self.entity_overlay)] {
            hash.update([kind]);
            if let Some(raw) = overlay.get(&chunk) {
                add(&mut hash, raw);
            }
        }
        Some(hash.finalize().into())
    }

    pub(super) fn capture_saved_chunks(
        &self,
        origin: &crate::history_source::Archive,
    ) -> Result<BTreeMap<ChunkPos, SavedChunk>, WorldError> {
        let chunks = self
            .content_hashes
            .keys()
            .chain(self.saved_chunks.keys())
            .chain(self.dirty_chunks.iter())
            .copied()
            .collect::<BTreeSet<_>>();
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, Some(origin))?
                .with_work_budget(self.save_work_budget);
        let mut result = BTreeMap::new();
        for chunk in chunks {
            let (flags, status) = if let Some(raw) = self.be_overlay.get(&chunk) {
                (light_flags(raw)?, status_stamp(raw)?)
            } else if let Some(created) = self.creation.get(&chunk) {
                (light_flags(&created.base)?, status_stamp(&created.base)?)
            } else {
                match reader.batch_chunk_bytes(chunk.0, chunk.1)? {
                    Some(raw) => (light_flags(&raw)?, status_stamp(&raw)?),
                    None => ([None; 2], None),
                }
            };
            result.insert(
                chunk,
                SavedChunk {
                    removed: self.generations.contains_key(&chunk),
                    base: self.region_bases.get(&chunk).map(|base| base.nbt),
                    geometry: self.geometry_hash(chunk),
                    flags,
                    status,
                    payload: self.payload_hash(chunk),
                    blocks: self.block_hashes(chunk),
                    created_slot: self
                        .creation
                        .get(&chunk)
                        .is_some_and(|created| created.source_chunk_sha.is_none()),
                    sections: self
                        .creation
                        .get(&chunk)
                        .into_iter()
                        .flat_map(|created| {
                            created.section_proofs.iter().map(|proof| proof.section_y)
                        })
                        .collect(),
                    be: raw_hash(self.be_overlay.get(&chunk)),
                    entities: raw_hash(self.entity_overlay.get(&chunk)),
                    biome: raw_hash(self.biome_overlay.get(&chunk)),
                    creation: self
                        .creation
                        .get(&chunk)
                        .map(|created| Sha256::digest(&created.base).into()),
                },
            );
        }
        Ok(result)
    }

    pub(super) fn prepare_saved_chunk_candidates(
        &self,
        policy: &Policy,
        timestamp: u32,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<crate::mixed_transaction::MixedCandidate>, WorldError> {
        use crate::{entity_editing::IdentitySource, mixed_transaction::MixedCandidate};
        if let Some(origin) = &self.origin {
            origin.verify_current()?;
        }
        self.verify_entity_history()?;
        let mut reader = self
            .source_reader()?
            .with_work_budget(self.save_work_budget);
        let mut terrain = self.clone();
        terrain.entity_overlay.clear();
        terrain.entity_sources.clear();
        let stages = if !terrain.creation.is_empty()
            || !terrain.be_overlay.is_empty()
            || !terrain.biome_overlay.is_empty()
        {
            terrain
                .prepare_terrain_chunk_candidates_in(&mut reader, None, false)?
                .0
        } else if !terrain.overlay.is_empty() {
            terrain.prepare_palette_candidates(false, false, &mut reader, None)?
        } else {
            Vec::new()
        };
        let mut stages = stages
            .into_iter()
            .map(|stage| ((stage.chunk_x, stage.chunk_z), stage.patched_nbt))
            .collect::<BTreeMap<_, _>>();
        let mut actual = ReadOnlyRegionCache::new(&self.world, &self.dimension)?
            .with_work_budget(self.save_work_budget);
        let mut targets = BTreeMap::new();
        let mut entities = BTreeMap::new();
        let mut terrain_history = BTreeSet::new();
        let mut entity_history = BTreeSet::new();
        let mut geometry_changes = BTreeSet::new();
        let mut nbt_changes = BTreeSet::new();
        for &chunk in &self.dirty_chunks {
            if cancelled() {
                return Err(refuse("区块快照规划已取消"));
            }
            let source = reader.snapshot_chunk_bytes(chunk.0, chunk.1)?;
            let mut desired = match stages.remove(&chunk) {
                Some(raw) => Some(raw),
                None => source.clone(),
            };
            desired = desired
                .map(|raw| {
                    self.completion.project(
                        chunk,
                        &raw,
                        self.region_base_changed(chunk) || self.restores_saved_chunk(chunk),
                    )
                })
                .transpose()?;
            let saved = self.saved_chunks.get(&chunk);
            let historical = (self.lifecycle_slot_changed(chunk)
                || self.region_base_changed(chunk))
                && !self.generations.contains_key(&chunk);
            if historical && source.is_some() && desired.is_some() {
                terrain_history.insert(chunk);
            }
            let mut geometry_changed =
                self.geometry_hash(chunk) != saved.and_then(|saved| saved.geometry);
            if geometry_changed
                && desired.is_none()
                && actual.snapshot_chunk_bytes(chunk.0, chunk.1)?.is_none()
            {
                geometry_changed = false;
            }
            if geometry_changed {
                geometry_changes.insert(chunk);
            }
            if self.payload_hash(chunk) != saved.and_then(|saved| saved.payload) {
                nbt_changes.insert(chunk);
            }
            if !geometry_changed {
                if let (Some(raw), Some(current)) = (
                    desired.as_deref(),
                    actual.snapshot_chunk_bytes(chunk.0, chunk.1)?,
                ) {
                    let flags = match saved {
                        Some(saved) => saved.flags,
                        None => match source.as_deref() {
                            Some(raw) => light_flags(raw)?,
                            None => [None; 2],
                        },
                    };
                    desired = Some(preserve_lighting(raw, &current, &flags)?);
                }
                desired = desired
                    .map(|raw| {
                        self.completion.project(
                            chunk,
                            &raw,
                            self.region_base_changed(chunk) || self.restores_saved_chunk(chunk),
                        )
                    })
                    .transpose()?;
            } else if matches!(policy, Policy::NativeInvalidation) {
                desired = desired
                    .map(|raw| crate::chunk_postprocess::invalidate(&raw))
                    .transpose()?;
            }
            if let Some(raw) = self.entity_overlay.get(&chunk) {
                let (kind, _) = self
                    .entity_sources
                    .get(&chunk)
                    .ok_or_else(|| refuse("实体快照缺少实际来源身份"))?;
                if kind.starts_with("entity-region:") {
                    if historical && kind != "entity-region:missing" {
                        entity_history.insert(chunk);
                    }
                    entities.insert(chunk, Some(raw.clone()));
                } else {
                    let target = desired
                        .as_ref()
                        .ok_or_else(|| refuse("内嵌实体没有地形槽"))?;
                    desired = Some(crate::shared_chunk_save::copy(target, "Entities", raw)?);
                }
            } else {
                match entity_catalog::storage_from(
                    &self.world,
                    &self.dimension,
                    chunk.0,
                    chunk.1,
                    self.origin.as_ref(),
                )? {
                    entity_catalog::Storage::Separate(original) => {
                        if historical && original.is_some() {
                            entity_history.insert(chunk);
                        }
                        entities.insert(chunk, original.map(|(_, raw)| raw));
                    }
                    entity_catalog::Storage::Embedded => {
                        // 撤销新建独立实体槽时原来源可以没有 entities 目录。
                        if matches!(self.world.data_version, Some(3465 | 3955)) {
                            entities.insert(chunk, None);
                        }
                    }
                }
            }
            targets.insert(chunk, desired);
        }
        if let Policy::LegacyRelight {
            profile,
            traits,
            operation_budget,
            memory_budget,
        } = policy
        {
            profile.verify_source(&self.world, &self.dimension)?;
            if !geometry_changes.is_empty() {
                let rebuilt = crate::legacy_lighting::rebuild_snapshots(
                    profile,
                    traits,
                    &geometry_changes,
                    *operation_budget,
                    *memory_budget,
                    |chunk| match targets.get(&chunk) {
                        Some(raw) => Ok(raw.clone()),
                        None => actual.snapshot_chunk_bytes(chunk.0, chunk.1),
                    },
                    &mut cancelled,
                )?;
                targets.extend(rebuilt.into_iter().map(|(chunk, raw)| (chunk, Some(raw))));
            }
        }
        let sources = actual.snapshot_read_sources()?;
        let mut result = Vec::new();
        if reader.alpha_storage() {
            for (chunk, raw) in targets {
                let image = match raw {
                    Some(raw) => {
                        let path =
                            crate::alpha::light_source_path(&self.world, &self.dimension, chunk)?;
                        let original = self
                            .file_source()
                            .read(&path)?
                            .ok_or_else(|| refuse("Alpha 快照缺少原始 GZip 包装"))?;
                        Some(
                            crate::alpha::decode_chunk_file(&original, chunk)?
                                .0
                                .encode(&raw)?,
                        )
                    }
                    None => None,
                };
                if let Some(mut candidate) = crate::chunk_snapshot::prepare_alpha(
                    &self.world,
                    &self.dimension,
                    chunk,
                    image,
                    nbt_changes.contains(&chunk),
                )? {
                    candidate.set_sources(&sources)?;
                    result.push(MixedCandidate::ChunkSnapshot(candidate));
                }
            }
            if !entities.is_empty() {
                return Err(refuse("Alpha 不能包含独立实体槽"));
            }
            return Ok(result);
        }
        for (entity_storage, chunks) in [(false, targets), (true, entities)] {
            let mut groups = BTreeMap::<ChunkPos, BTreeMap<ChunkPos, Option<Vec<u8>>>>::new();
            for (chunk, raw) in chunks {
                groups
                    .entry((chunk.0.div_euclid(32), chunk.1.div_euclid(32)))
                    .or_default()
                    .insert(chunk, raw);
            }
            for (region, chunks) in groups {
                if cancelled() {
                    return Err(refuse("区块快照规划已取消"));
                }
                let nbt = chunks.keys().any(|chunk| nbt_changes.contains(chunk));
                let storage = if entity_storage {
                    Storage::Entities { region }
                } else {
                    let path =
                        crate::chunk_creation::region_path(&self.world, &self.dimension, region)?;
                    let extension = path
                        .extension()
                        .and_then(|value| value.to_str())
                        .ok_or_else(|| refuse("地形 Region 扩展名无效"))?
                        .to_owned();
                    Storage::Terrain { region, extension }
                };
                let anchors = if entity_storage {
                    &entity_history
                } else {
                    &terrain_history
                };
                let scope = chunks
                    .keys()
                    .filter(|chunk| anchors.contains(*chunk))
                    .copied()
                    .collect::<BTreeSet<_>>();
                let versions = if entity_storage {
                    BTreeMap::new()
                } else {
                    chunks
                        .iter()
                        .filter(|(_, raw)| raw.is_some())
                        .filter_map(|(chunk, _)| {
                            self.region_bases
                                .get(chunk)
                                .map(|base| (*chunk, base.version.as_ref().clone()))
                        })
                        .collect::<BTreeMap<_, _>>()
                };
                let candidate = if scope.is_empty() && versions.is_empty() {
                    crate::chunk_snapshot::prepare_region_with_budget(
                        &self.world,
                        &self.dimension,
                        storage,
                        chunks,
                        nbt,
                        timestamp,
                        0,
                        self.save_work_budget,
                    )?
                } else {
                    let origin = self
                        .origin
                        .as_ref()
                        .ok_or_else(|| refuse("历史槽位恢复缺少已提交来源"))?;
                    crate::chunk_snapshot::prepare_region_with_history_versions(
                        &self.world,
                        &self.dimension,
                        storage,
                        chunks,
                        nbt,
                        timestamp,
                        origin,
                        &scope,
                        &versions,
                        &mut cancelled,
                        self.save_work_budget,
                    )?
                };
                if let Some(mut candidate) = candidate {
                    candidate.set_sources(&sources)?;
                    result.push(MixedCandidate::ChunkSnapshot(candidate));
                }
            }
        }
        let mut poi = BTreeMap::<ChunkPos, (BTreeSet<ChunkPos>, BTreeSet<ChunkPos>)>::new();
        for chunk in &self.dirty_chunks {
            let removed = self.generations.contains_key(chunk);
            if removed
                == self
                    .saved_chunks
                    .get(chunk)
                    .is_some_and(|saved| saved.removed)
            {
                continue;
            }
            let range = poi
                .entry((chunk.0.div_euclid(32), chunk.1.div_euclid(32)))
                .or_default();
            if removed {
                range.1.insert(*chunk);
            } else {
                range.0.insert(*chunk);
            }
        }
        for (region, (restored, deleted)) in poi {
            if cancelled() {
                return Err(refuse("POI 保存规划已取消"));
            }
            let origin = self
                .origin
                .as_ref()
                .ok_or_else(|| refuse("POI 恢复缺少历史来源"))?;
            if let Some(candidate) = crate::chunk_snapshot::prepare_poi_lifecycle(
                &self.world,
                &self.dimension,
                region,
                &restored,
                &deleted,
                origin,
                timestamp,
                &mut cancelled,
                self.save_work_budget,
            )? {
                result.push(MixedCandidate::ChunkSnapshot(candidate));
            }
        }
        if cancelled() {
            return Err(refuse("区块保存候选发布已取消"));
        }
        Ok(result)
    }
}
