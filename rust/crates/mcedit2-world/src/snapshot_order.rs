//! 完整冻结结果的每区块根顺序，沿现有实体及 BE 历史发布。
use super::*;
use crate::root_order::{Kind, Proof};

impl PreparedStructure {
    pub(super) fn script_imported_entities(&self) -> Result<Vec<Vec<u8>>, WorldError> {
        self.entity_groups
            .iter()
            .map(|group| {
                let EntityProof::Import(proof) = &group.proof else {
                    return Err(refuse("脚本新增根序列包含非导入证明"));
                };
                let change = group
                    .changes
                    .first()
                    .ok_or_else(|| refuse("实体导入缺少实际结果"))?;
                crate::root_order::last_imported_entity(&change.after, proof.target().1)
            })
            .collect()
    }
}

impl Session {
    /// 输入为最终实际载荷；新增身份已经完成绑定，只在各自真实区块中安排相对次序。
    pub(super) fn reorder_snapshot_objects_cancellable(
        &mut self,
        entities: Vec<Vec<u8>>,
        block_entities: Vec<Vec<u8>>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        let mut entity_chunks = BTreeMap::<ChunkPos, Vec<Vec<u8>>>::new();
        for bytes in entities {
            if cancelled() {
                return Err(refuse("脚本实体顺序整理已取消"));
            }
            let point = snapshot::entity_position(&snapshot::value(&bytes)?)?;
            let chunk = (
                (point[0] / 16.).floor() as i32,
                (point[2] / 16.).floor() as i32,
            );
            entity_chunks.entry(chunk).or_default().push(bytes);
        }
        for (chunk, desired) in entity_chunks {
            if desired.len() < 2 {
                continue;
            }
            let mut change = self.entity_input(chunk)?;
            let kind = if change.source.starts_with("entity-region:") {
                Kind::SeparateEntities
            } else {
                Kind::EmbeddedEntities
            };
            if let Some(proof) = Proof::prepare(
                &change.before,
                chunk,
                kind,
                &desired,
                budget,
                &mut cancelled,
            )? {
                change.after = proof.replay_entities(&change.before)?;
                self.apply_entity_preview(EntityPreview {
                    root: self.world.root.clone(),
                    dimension: self.dimension.clone(),
                    identity_revision: None,
                    changes: vec![change],
                    proof: EntityProof::RootOrder(proof),
                })?;
            }
        }
        let mut be_chunks = BTreeMap::<ChunkPos, Vec<Vec<u8>>>::new();
        for bytes in block_entities {
            if cancelled() {
                return Err(refuse("脚本方块实体顺序整理已取消"));
            }
            let point = snapshot::be_position(&snapshot::value(&bytes)?)?;
            be_chunks
                .entry((point[0].div_euclid(16), point[2].div_euclid(16)))
                .or_default()
                .push(bytes);
        }
        let world = self.world.clone();
        let dimension = self.dimension.clone();
        let mut reader = ReadOnlyRegionCache::for_source(&world, &dimension, self.origin.as_ref())?;
        for (chunk, desired) in be_chunks {
            if desired.len() < 2 {
                continue;
            }
            let (base, source_sha) = self.terrain_payload_base_cached(&mut reader, chunk)?;
            let before = self.be_overlay.get(&chunk).cloned().unwrap_or(base);
            if let Some(proof) = Proof::prepare(
                &before,
                chunk,
                Kind::BlockEntities,
                &desired,
                budget,
                &mut cancelled,
            )? {
                let after = proof.replay_blocks(&before)?;
                self.apply_be_nbt_preview(BeNbtPreview {
                    root: self.world.root.clone(),
                    dimension: self.dimension.clone(),
                    chunk,
                    source_sha,
                    before,
                    after,
                    proof: BeNbtProof::RootOrder(proof),
                    completion: None,
                })?;
            }
        }
        if cancelled() {
            return Err(refuse("脚本根顺序整理已取消"));
        }
        Ok(())
    }
}
