//! 世界会话保存点与历史来源；内容摘要按实际变化的区块更新，不在界面重绘中扫描编辑。
use super::*;
use std::sync::Arc;

fn feed(hash: &mut Sha256, kind: u8, bytes: &[u8]) {
    hash.update([kind]);
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
impl Session {
    pub fn history_source(&self) -> Option<&crate::history_source::Archive> {
        self.origin.as_ref()
    }
    /// 采用后台验证的独立文件通知；期间新增的逻辑编辑和历史原样保留。
    pub fn accept_source_observation(
        &mut self,
        observation: crate::history_source::Observation,
    ) -> Result<(), WorldError> {
        let same = match (&self.origin, &observation.previous) {
            (None, None) => true,
            (Some(current), Some(previous)) => current.same_registry(previous),
            _ => false,
        };
        if !same
            || self.world.root.as_path() != observation.next.world_root()
            || self.recording.is_some()
        {
            return Err(refuse("独立文件保存通知的世界或原归档身份已改变"));
        }
        let next = match &self.origin {
            Some(current) => observation.next.with_projection_from(current),
            None => observation.next,
        };
        let regions = self.changed_derived_regions(&next)?;
        self.origin = Some(next);
        self.changed_source_chunks
            .extend(self.display_source.chunks());
        self.note_change();
        self.display_source.update_regions(regions, &self.revision);
        Ok(())
    }
    /// 渲染、分析和工具读取同一逻辑来源；候选写入另行验证真实磁盘头。
    pub fn source_reader(&self) -> Result<ReadOnlyRegionCache<'_>, WorldError> {
        ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())
    }
    pub fn shared_source_reader(
        &self,
        budget: usize,
    ) -> Result<crate::block_lookup::SharedRegionBatch, WorldError> {
        match &self.origin {
            Some(origin) => crate::block_lookup::SharedRegionBatch::with_history_source(
                &self.world,
                &self.dimension,
                budget,
                origin,
            ),
            None => {
                crate::block_lookup::SharedRegionBatch::new(&self.world, &self.dimension, budget)
            }
        }
    }
    pub(crate) fn file_source(&self) -> crate::history_source::View<'_> {
        crate::history_source::View(self.origin.as_ref())
    }
    pub(crate) fn source_chunk_bytes(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<Option<Vec<u8>>, WorldError> {
        self.source_reader()?.read_chunk_bytes(cx, cz)
    }
    /// 仅接收实际成功提交且仍属于该修订的来源；历史顺序和操作身份保持不变。
    pub fn accept_saved(
        &mut self,
        archive: crate::history_source::Archive,
        revision: &Arc<()>,
    ) -> Result<(), WorldError> {
        if self.world.root.as_path() != archive.world_root()
            || !Arc::ptr_eq(&self.revision, revision)
            || self.recording.is_some()
        {
            return Err(refuse("保存结果与世界会话的来源或当前修订不一致"));
        }
        let origin = match &self.origin {
            Some(previous) => previous.merged(&archive)?,
            None => archive,
        };
        origin.verify_current()?;
        let saved_chunks = self.capture_saved_chunks(&origin)?;
        let saved_be_steps = self.collect_scalar_steps()?;
        let regions = self.changed_derived_regions(&origin)?;
        let completion_sources = self.capture_completion_sources(
            &mut ReadOnlyRegionCache::for_source(&self.world, &self.dimension, Some(&origin))?,
        )?;
        self.completion.saved(completion_sources);
        self.origin = Some(origin);
        self.saved_chunks = Arc::new(saved_chunks);
        self.saved_be_steps = Arc::new(saved_be_steps);
        self.dirty_chunks.clear();
        self.changed_source_chunks
            .extend(self.content_hashes.keys().copied());
        self.changed_source_chunks
            .extend(self.display_source.chunks());
        self.note_change();
        self.saved_hashes = Arc::new(self.content_hashes.clone());
        self.dirty_chunks.clear();
        self.display_source.update_regions(regions, &self.revision);
        Ok(())
    }
    /// 同一世界的其他来源完成保存时，推进实际磁盘头，不改变本会话的保存点。
    pub fn observe_saved(
        &mut self,
        archive: crate::history_source::Archive,
        revision: &Arc<()>,
    ) -> Result<(), WorldError> {
        if self.world.root.as_path() != archive.world_root()
            || !Arc::ptr_eq(&self.revision, revision)
            || self.recording.is_some()
        {
            return Err(refuse("保存通知与世界会话的来源或修订不一致"));
        }
        let origin = match &self.origin {
            Some(previous) => previous.merged(&archive)?,
            None => archive,
        };
        origin.verify_current()?;
        let regions = self.changed_derived_regions(&origin)?;
        let completion_sources = self.capture_completion_sources(
            &mut ReadOnlyRegionCache::for_source(&self.world, &self.dimension, Some(&origin))?,
        )?;
        self.completion.observed(completion_sources);
        self.origin = Some(origin);
        self.changed_source_chunks
            .extend(self.completion.coordinates());
        self.changed_source_chunks
            .extend(self.display_source.chunks());
        self.note_change();
        self.display_source.update_regions(regions, &self.revision);
        Ok(())
    }
    /// 实际规划确认没有文件变化时，仅移动保存点，不创建空事务或伪造备份。
    pub fn accept_unchanged(
        &mut self,
        policy: &crate::chunk_postprocess::Policy,
        timestamp: u32,
        revision: &Arc<()>,
    ) -> Result<(), WorldError> {
        if !Arc::ptr_eq(&self.revision, revision) || self.recording.is_some() {
            return Err(refuse("无文件变化的保存点与会话修订不一致"));
        }
        if !self
            .prepare_postprocessed_candidates(policy, timestamp, || false)?
            .is_empty()
        {
            return Err(refuse("当前会话仍需要实际文件写入，不能只移动保存点"));
        }
        let origin = self
            .origin
            .as_ref()
            .ok_or_else(|| refuse("无文件变化的保存缺少已有历史来源"))?;
        origin.verify_current()?;
        let saved_chunks = self.capture_saved_chunks(origin)?;
        let saved_be_steps = self.collect_scalar_steps()?;
        let completion_sources = self.capture_completion_sources(
            &mut ReadOnlyRegionCache::for_source(&self.world, &self.dimension, Some(origin))?,
        )?;
        self.completion.saved(completion_sources);
        self.saved_chunks = Arc::new(saved_chunks);
        self.saved_be_steps = Arc::new(saved_be_steps);
        self.dirty_chunks.clear();
        self.changed_source_chunks
            .extend(self.display_source.chunks());
        self.note_change();
        self.saved_hashes = Arc::new(self.content_hashes.clone());
        self.dirty_chunks.clear();
        Ok(())
    }
    fn chunk_content_hash(&self, chunk: ChunkPos) -> Option<[u8; 32]> {
        if !self.block_chunk_counts.contains_key(&chunk)
            && !self.creation.contains_key(&chunk)
            && !self.entity_overlay.contains_key(&chunk)
            && !self.be_overlay.contains_key(&chunk)
            && !self.biome_overlay.contains_key(&chunk)
            && !self.completion.has_pending(chunk)
            && !self.generations.contains_key(&chunk)
            && !self.region_bases.contains_key(&chunk)
        {
            return None;
        }
        let mut hash = Sha256::new();
        if let Some(base) = self.region_bases.get(&chunk) {
            feed(&mut hash, 254, &base.nbt);
        }
        if self.generations.contains_key(&chunk) {
            hash.update([255]);
        }
        if let Some(created) = self.creation.get(&chunk) {
            feed(&mut hash, 0, &created.base);
        }
        for (position, state) in self.display_changes_in_chunk(chunk.0, chunk.1) {
            hash.update([1]);
            hash.update(position.0.to_be_bytes());
            hash.update(position.1.to_be_bytes());
            hash.update(position.2.to_be_bytes());
            feed(&mut hash, 2, state.name.as_bytes());
            hash.update((state.properties.len() as u64).to_be_bytes());
            for (key, value) in &state.properties {
                feed(&mut hash, 3, key.as_bytes());
                feed(&mut hash, 4, value.as_bytes());
            }
        }
        for (kind, overlay) in [
            (5, &self.be_overlay),
            (6, &self.biome_overlay),
            (7, &self.entity_overlay),
        ] {
            if let Some(bytes) = overlay.get(&chunk) {
                feed(&mut hash, kind, bytes);
            }
        }
        if !self.generations.contains_key(&chunk) || self.creation.contains_key(&chunk) {
            self.completion.hash(chunk, &mut hash);
        }
        Some(hash.finalize().into())
    }
    pub(super) fn update_content_hashes(&mut self, chunks: &BTreeSet<ChunkPos>) {
        for &chunk in chunks {
            if self.pending_geometry_chunk(chunk) {
                self.pending_geometry.insert(chunk);
            } else {
                self.pending_geometry.remove(&chunk);
            }
            if self.pending_entity_chunk(chunk) {
                self.pending_entity.insert(chunk);
            } else {
                self.pending_entity.remove(&chunk);
            }
            if self.pending_be_chunk(chunk) {
                self.pending_be.insert(chunk);
            } else {
                self.pending_be.remove(&chunk);
            }
            let blocks = self.pending_blocks_in_chunk(chunk);
            if blocks == 0 {
                self.pending_block_counts.remove(&chunk);
            } else {
                self.pending_block_counts.insert(chunk, blocks);
            }
            let hash = self.chunk_content_hash(chunk);
            match hash {
                Some(hash) => {
                    self.content_hashes.insert(chunk, hash);
                }
                None => {
                    self.content_hashes.remove(&chunk);
                }
            }
            if hash.as_ref() == self.saved_hashes.get(&chunk) {
                self.dirty_chunks.remove(&chunk);
            } else {
                self.dirty_chunks.insert(chunk);
            }
        }
    }
}
