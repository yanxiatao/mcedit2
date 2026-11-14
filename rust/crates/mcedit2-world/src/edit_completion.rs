//! 完成字段的实际读取、编辑准入与保存点交接；不把自动重算结果写入历史原字节。
use super::*;
use crate::chunk_completion::{self, Field, Values};

impl Session {
    pub(super) fn physical_completion_source(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        chunk: ChunkPos,
    ) -> Result<Vec<u8>, WorldError> {
        if self.region_base_changed(chunk) {
            return self
                .terrain_payload_base_cached(reader, chunk)
                .map(|(raw, _)| raw);
        }
        if let Some(raw) = reader.current_chunk_bytes(chunk.0, chunk.1)? {
            crate::block_lookup::checked_original_chunk_bytes(Some(&raw), chunk.0, chunk.1)?;
            return Ok(raw);
        }
        // 撤销已提交的删除后，磁盘仍缺席，字段编辑的基线是明确恢复中的原槽。
        if self.restores_saved_chunk(chunk) {
            return self
                .terrain_payload_base_cached(reader, chunk)
                .map(|(raw, _)| raw);
        }
        match self.creation.get(&chunk) {
            Some(created) if created.source_chunk_sha.is_none() => Ok(created.base.clone()),
            _ => Err(refuse("完成字段的实际区块缺失，且没有已核对的新槽合同")),
        }
    }
    pub(super) fn display_completion_payload(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        chunk: ChunkPos,
        raw: Vec<u8>,
    ) -> Result<Vec<u8>, WorldError> {
        let physical = self.physical_completion_source(reader, chunk)?;
        let physical = chunk_completion::read(&physical)?;
        let values = Field::ALL
            .into_iter()
            .map(|field| {
                (
                    field,
                    self.completion.effective(chunk, field, &physical[&field]),
                )
            })
            .collect();
        chunk_completion::write(&raw, &values)
    }
    pub(super) fn capture_completion_sources(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
    ) -> Result<BTreeMap<ChunkPos, Option<Values>>, WorldError> {
        self.completion
            .coordinates()
            .map(|chunk| {
                let values = match reader.current_chunk_bytes(chunk.0, chunk.1)? {
                    Some(raw) => {
                        crate::block_lookup::checked_original_chunk_bytes(
                            Some(&raw),
                            chunk.0,
                            chunk.1,
                        )?;
                        Some(chunk_completion::read(&raw)?)
                    }
                    None => None,
                };
                Ok((chunk, values))
            })
            .collect()
    }
}
