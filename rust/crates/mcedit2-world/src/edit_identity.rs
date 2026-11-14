//! 新实体操作使用当前磁盘身份清单，再原子合入同一世界各会话的逻辑实体状态。
//! 其他维度及玩家来源不回退到最早保存点，旧领域历史仍按各自原始字节重放。
use super::*;
use crate::entity_editing::{DiskIdentitySource, IdentityChunkChange, IdentitySnapshot};
use std::sync::Arc;

/// 共享的实体身份来源；只持有单维度必要载荷，不引用其他会话或撤销历史。
#[derive(Debug)]
pub struct IdentityPeer {
    session: Session,
    chunks: BTreeSet<ChunkPos>,
}

struct Image {
    dimension: String,
    chunk: ChunkPos,
    separate: bool,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
}
fn append(
    images: &mut Vec<Image>,
    dimension: &str,
    chunk: ChunkPos,
    before: Option<(bool, Vec<u8>)>,
    after: Option<(bool, Vec<u8>)>,
    bytes: &mut usize,
    memory_budget: usize,
) -> Result<(), WorldError> {
    if before == after {
        return Ok(());
    }
    *bytes = bytes
        .checked_add(before.as_ref().map_or(0, |(_, raw)| raw.len()))
        .and_then(|bytes| bytes.checked_add(after.as_ref().map_or(0, |(_, raw)| raw.len())))
        .ok_or_else(|| refuse("当前实体身份载荷工作集溢出"))?;
    if *bytes > memory_budget {
        return Err(refuse("当前实体身份载荷超过内存预算"));
    }
    match (before, after) {
        (Some((left, before)), Some((right, after))) if left == right => {
            images.push(Image {
                dimension: dimension.to_owned(),
                chunk,
                separate: left,
                before: Some(before),
                after: Some(after),
            });
        }
        (before, after) => {
            if let Some((separate, raw)) = before {
                images.push(Image {
                    dimension: dimension.to_owned(),
                    chunk,
                    separate,
                    before: Some(raw),
                    after: None,
                });
            }
            if let Some((separate, raw)) = after {
                images.push(Image {
                    dimension: dimension.to_owned(),
                    chunk,
                    separate,
                    before: None,
                    after: Some(raw),
                });
            }
        }
    }
    Ok(())
}

impl Session {
    pub fn has_pending_entity_changes(&self) -> bool {
        !self.pending_entity.is_empty()
    }

    /// 本地来源变化才重新建立投影；同伴上下文变化不复制载荷或历史。
    pub fn identity_peer_snapshot(&mut self) -> Arc<IdentityPeer> {
        if let Some(projection) = &self.identity_projection {
            return Arc::clone(projection);
        }
        let chunks = self.entity_identity_chunks();
        let mut session = Session::new(self.world.clone(), self.dimension.clone())
            .expect("会话维度已经通过构造校验");
        session.origin = self.origin.clone();
        session.generations = self.generations.clone();
        session.region_bases = self.region_bases.clone();
        session.revision = Arc::clone(&self.identity_local_revision);
        session.identity_local_revision = Arc::clone(&self.identity_local_revision);
        session.entity_overlay = self.entity_overlay.clone();
        session.entity_sources = self.entity_sources.clone();
        session.saved_chunks = Arc::clone(&self.saved_chunks);
        session.creation = self
            .creation
            .iter()
            .filter(|(chunk, _)| chunks.contains(*chunk))
            .map(|(chunk, creation)| (*chunk, creation.clone()))
            .collect();
        let projection = Arc::new(IdentityPeer { session, chunks });
        self.identity_projection = Some(Arc::clone(&projection));
        projection
    }

    /// 更新预检来源身份；不影响本地投影代次，避免会话之间反复使彼此失效。
    pub fn set_identity_peers(&mut self, mut peers: Vec<Arc<IdentityPeer>>) {
        peers.sort_by(|left, right| {
            (&left.session.world.root, &left.session.dimension)
                .cmp(&(&right.session.world.root, &right.session.dimension))
        });
        if self.identity_peers.len() == peers.len()
            && self
                .identity_peers
                .iter()
                .zip(&peers)
                .all(|(left, right)| Arc::ptr_eq(left, right))
        {
            return;
        }
        self.identity_peers = peers;
        self.revision = Arc::new(());
    }

    pub(super) fn scan_current_identities(
        &self,
        extra: &BTreeMap<ChunkPos, CreationSnapshot>,
        memory_budget: usize,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<IdentitySnapshot, WorldError> {
        let mut world = self.world.clone();
        let mut dimensions = BTreeSet::from([self.dimension.as_str()]);
        for peer in &self.identity_peers {
            let source = &peer.session;
            if source.world.root != world.root || !dimensions.insert(source.dimension.as_str()) {
                return Err(refuse(
                    "实体身份来源包含其他世界或重复维度，请先解决会话来源冲突",
                ));
            }
            let dimension = source
                .world
                .dimensions
                .iter()
                .find(|dimension| dimension.name == source.dimension)
                .ok_or_else(|| refuse("实体身份来源缺少维度定义"))?;
            if let Some(current) = world
                .dimensions
                .iter()
                .find(|entry| entry.name == dimension.name)
            {
                if current.region_dir != dimension.region_dir
                    || current.extension != dimension.extension
                {
                    return Err(refuse("实体身份来源的维度存储定义不同"));
                }
            } else {
                world.dimensions.push(dimension.clone());
            }
        }
        let mut identities = crate::entity_editing::scan_identities_with(
            &DiskIdentitySource,
            &world,
            "",
            &BTreeMap::new(),
            memory_budget,
            &mut cancelled,
        )?;
        let mut images = Vec::new();
        let mut bytes = 0usize;
        let chunks = self
            .entity_overlay
            .keys()
            .chain(self.saved_chunks.keys())
            .chain(self.creation.keys())
            .chain(extra.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        self.collect_identity_images(
            extra,
            &chunks,
            &mut images,
            &mut bytes,
            memory_budget,
            &mut cancelled,
        )?;
        for peer in &self.identity_peers {
            peer.session.collect_identity_images(
                &BTreeMap::new(),
                &peer.chunks,
                &mut images,
                &mut bytes,
                memory_budget,
                &mut cancelled,
            )?;
        }
        identities.replace_chunks(
            &images
                .iter()
                .map(|image| IdentityChunkChange {
                    dimension: &image.dimension,
                    chunk: image.chunk,
                    separate: image.separate,
                    before: image.before.as_deref(),
                    after: image.after.as_deref(),
                })
                .collect::<Vec<_>>(),
            memory_budget,
        )?;
        identities.manifest.verify(&mut cancelled)?;
        Ok(identities)
    }

    fn collect_identity_images(
        &self,
        extra: &BTreeMap<ChunkPos, CreationSnapshot>,
        chunks: &BTreeSet<ChunkPos>,
        images: &mut Vec<Image>,
        bytes: &mut usize,
        memory_budget: usize,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        if let Some(origin) = &self.origin {
            origin.verify_current()?;
        }
        let mut actual = ReadOnlyRegionCache::new(&self.world, &self.dimension)?;
        let mut original = self.source_reader()?;
        for &chunk in chunks {
            if cancelled() {
                return Err(refuse("当前实体身份扫描已取消"));
            }
            let before = match crate::entity_catalog::storage(
                &self.world,
                &self.dimension,
                chunk.0,
                chunk.1,
            )? {
                crate::entity_catalog::Storage::Separate(Some((_, raw))) => Some((true, raw)),
                _ => actual
                    .snapshot_chunk_bytes(chunk.0, chunk.1)?
                    .map(|raw| (false, raw)),
            };
            let after = if let Some(raw) = self.entity_overlay.get(&chunk) {
                self.entity_input(chunk)?;
                let (source, _) = self
                    .entity_sources
                    .get(&chunk)
                    .ok_or_else(|| refuse("当前实体暂存缺少来源身份"))?;
                Some((source.starts_with("entity-region:"), raw.clone()))
            } else {
                match crate::entity_catalog::storage_from(
                    &self.world,
                    &self.dimension,
                    chunk.0,
                    chunk.1,
                    self.origin.as_ref(),
                )? {
                    crate::entity_catalog::Storage::Separate(Some((_, raw))) => Some((true, raw)),
                    _ => {
                        let creation = self.creation.get(&chunk).or_else(|| extra.get(&chunk));
                        if let Some(created) = creation.filter(|created| {
                            created.source_chunk_sha.is_none()
                                && created.profile.data_version == 1343
                        }) {
                            let (_, raw) =
                                self.entity_source_with_creation(chunk, Some(created))?;
                            Some((false, raw))
                        } else {
                            original
                                .snapshot_chunk_bytes(chunk.0, chunk.1)?
                                .map(|raw| (false, raw))
                        }
                    }
                }
            };
            append(
                images,
                &self.dimension,
                chunk,
                before,
                after,
                bytes,
                memory_budget,
            )?;
        }
        Ok(())
    }
}
