//! 已解决旧 Region 内容变化的正式步骤；局部暂存和真实版本引用各自持有归属。
use super::*;
use crate::history_source::{Archive, FileVersion};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Base {
    pub(super) version: Arc<FileVersion>,
    pub(super) nbt: [u8; 32],
    pub(super) geometry: Option<[u8; 32]>,
    pub(super) payload: Option<[u8; 32]>,
}
pub(super) fn bases_bytes<'a>(bases: impl Iterator<Item = &'a Base>) -> usize {
    let mut seen = BTreeSet::new();
    bases
        .map(|base| {
            std::mem::size_of::<Base>()
                + 64
                + if seen.insert(Arc::as_ptr(&base.version) as usize) {
                    base.version.retained_bytes()
                } else {
                    0
                }
        })
        .sum()
}
#[derive(Debug)]
pub(super) struct Step {
    changes: Vec<Arc<crate::region_history::Change>>,
    before: lifecycle::Content,
    after: BTreeMap<ChunkPos, Base>,
    chunks: BTreeSet<ChunkPos>,
    terrain_entities: BTreeSet<ChunkPos>,
    before_hashes: BTreeMap<ChunkPos, [u8; 32]>,
    after_hashes: BTreeMap<ChunkPos, [u8; 32]>,
}
impl Step {
    pub(super) fn chunks(&self) -> &BTreeSet<ChunkPos> {
        &self.chunks
    }
    pub(super) fn history_entry(&self) -> crate::history::Entry {
        crate::history::Entry {
            label: if self.changes.iter().all(|change| change.rollback()) {
                "回滚旧 Region 内容"
            } else {
                "接入旧 Region 内容"
            }
            .into(),
            detail: format!(
                "{} 个 Region · {} 个区块",
                self.changes.len(),
                self.chunks.len()
            ),
        }
    }
    pub(super) fn retained_bytes(&self) -> usize {
        self.before.retained_bytes()
            + bases_bytes(self.after.values())
            + self
                .changes
                .iter()
                .map(|change| change.retained_bytes())
                .sum::<usize>()
            + self.chunks.len() * 160
            + (self.before_hashes.len() + self.after_hashes.len()) * 64
    }
    pub(super) fn replay_source(
        &self,
        context: &mut Session,
        replayed: &mut BTreeMap<ChunkPos, Vec<u8>>,
    ) -> Result<(), WorldError> {
        for chunk in &self.chunks {
            context.creation.remove(chunk);
            context.generations.remove(chunk);
            context.region_bases.remove(chunk);
            if self.terrain_entities.contains(chunk) {
                replayed.remove(chunk);
            }
        }
        context.region_bases.extend(
            self.after
                .iter()
                .map(|(chunk, base)| (*chunk, base.clone())),
        );
        context.bind_generation_source()
    }
}
impl Session {
    /// 接收确已解决的旧事务；整组在私有工作会话中构造，只发布完整步骤。
    pub fn observe_region_transaction(
        mut self,
        archive: Archive,
        revision: &Arc<()>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        if self.world.root.as_path() != archive.world_root()
            || !Arc::ptr_eq(revision, &self.revision)
        {
            return Err(refuse("旧 Region 内容交接的世界或修订不同"));
        }
        let changes = archive
            .region_changes()
            .filter(|change| change.dimension() == self.dimension)
            .filter(|change| !change.slots.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if changes.is_empty() {
            self.observe_saved(archive, revision)?;
            return Ok(self);
        }
        self.check_chunk_history_capacity(1)?;
        let chunks = changes
            .iter()
            .flat_map(|change| change.slots.keys().copied())
            .collect::<BTreeSet<_>>();
        if chunks.iter().any(|chunk| self.dirty_chunks.contains(chunk)) {
            return Err(refuse("旧 Region 内容与未保存的区块编辑重叠"));
        }
        let origin = match &self.origin {
            Some(previous) => previous.merged(&archive)?,
            None => archive.clone(),
        };
        origin.verify_current()?;
        if cancelled() {
            return Err(refuse("旧 Region 内容接入已取消"));
        }
        let terrain_entities = self
            .leaf_groups()
            .flat_map(|group| match group {
                EditGroup::Entities(group) => group.changes.iter().collect::<Vec<_>>(),
                EditGroup::Structure(group) => group
                    .entity_groups
                    .iter()
                    .flat_map(|group| group.changes.iter())
                    .collect(),
                _ => Vec::new(),
            })
            .filter(|change| {
                chunks.contains(&change.chunk) && !change.source.starts_with("entity-region:")
            })
            .map(|change| change.chunk)
            .collect();
        let mut work = self.clone();
        work.origin = Some(origin);
        let mut after = BTreeMap::new();
        for change in &changes {
            let before_version = Arc::new(change.before.clone());
            let after_version = Arc::new(change.after.clone());
            for (chunk, slot) in &change.slots {
                if cancelled() {
                    return Err(refuse("旧 Region 来源冻结已取消"));
                }
                let geometry = self.geometry_hash(*chunk);
                let payload = self.payload_hash(*chunk);
                // 未曾登记过此文件的会话在回滚前读到替换版本；保留这一实际基底。
                if change.rollback()
                    && self
                        .origin
                        .as_ref()
                        .is_none_or(|origin| !origin.has_file(change.before.path()))
                {
                    work.region_bases.insert(
                        *chunk,
                        Base {
                            version: Arc::clone(&before_version),
                            nbt: slot.before_nbt,
                            geometry,
                            payload,
                        },
                    );
                }
                after.insert(
                    *chunk,
                    Base {
                        version: Arc::clone(&after_version),
                        nbt: slot.after_nbt,
                        geometry: if slot.before_geometry == slot.after_geometry {
                            geometry
                        } else {
                            Some(slot.after_geometry)
                        },
                        payload: if slot.before_payload == slot.after_payload {
                            payload
                        } else {
                            Some(slot.after_payload)
                        },
                    },
                );
            }
        }
        work.bind_generation_source()?;
        work.changed_source_chunks.extend(chunks.iter().copied());
        work.note_change();
        let before_hashes = selected_hashes(&work, &chunks);
        let before = lifecycle::Content::take_terrain(&mut work, &chunks);
        work.region_bases
            .extend(after.iter().map(|(chunk, base)| (*chunk, base.clone())));
        work.bind_generation_source()?;
        work.changed_source_chunks.extend(chunks.iter().copied());
        work.note_change();
        let step = Arc::new(Step {
            changes,
            before,
            after,
            chunks: chunks.clone(),
            terrain_entities,
            before_hashes,
            after_hashes: selected_hashes(&work, &chunks),
        });
        budget.check(step.retained_bytes())?;
        work.record_edit(EditGroup::Region(step))?;
        work.note_change();
        work.accept_committed_chunk_range(&chunks)?;
        if cancelled() {
            return Err(refuse("旧 Region 内容发布已取消"));
        }
        Ok(work)
    }
    pub(super) fn initial_region_bases(&self) -> BTreeMap<ChunkPos, Base> {
        let mut result = self.region_bases.clone();
        let mut seen = BTreeSet::new();
        let mut pending = self
            .undo
            .iter()
            .chain(self.redo.iter().rev())
            .rev()
            .collect::<Vec<_>>();
        while let Some(group) = pending.pop() {
            match group {
                EditGroup::Compound(groups) => pending.extend(groups.iter().rev()),
                EditGroup::Region(step) => {
                    for chunk in step.chunks() {
                        if seen.insert(*chunk) {
                            if let Some(base) = step.before.region_bases.get(chunk) {
                                result.insert(*chunk, base.clone());
                            } else {
                                result.remove(chunk);
                            }
                        }
                    }
                }
                EditGroup::Deletion(step) => {
                    for chunk in step.chunks() {
                        if seen.insert(*chunk) {
                            if let Some(base) = step.previous_region_bases().get(chunk) {
                                result.insert(*chunk, base.clone());
                            } else {
                                result.remove(chunk);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        result
    }
    pub(super) fn restore_region_step(&mut self, after: bool) -> Result<bool, WorldError> {
        let step = match if after {
            self.redo.last()
        } else {
            self.undo.last()
        } {
            Some(EditGroup::Region(step)) => Arc::clone(step),
            _ => return Ok(false),
        };
        let expected = if after {
            &step.before_hashes
        } else {
            &step.after_hashes
        };
        let bases = if after {
            &step.before.region_bases
        } else {
            &step.after
        };
        for chunk in &step.chunks {
            if self.content_hashes.get(chunk) != expected.get(chunk)
                || self.region_bases.get(chunk) != bases.get(chunk)
            {
                return Err(refuse("旧 Region 历史的内容或来源基底不同"));
            }
        }
        lifecycle::Content::take_terrain(self, &step.chunks);
        if after {
            self.region_bases.extend(
                step.after
                    .iter()
                    .map(|(chunk, base)| (*chunk, base.clone())),
            );
        } else {
            step.before.restore(self, &step.chunks);
        }
        self.bind_generation_source()?;
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
        to.push(from.pop().expect("旧 Region 步骤存在"));
        to_ids.push(ids.pop().expect("旧 Region 步骤具有稳定身份"));
        self.changed_source_chunks
            .extend(step.chunks.iter().copied());
        self.note_change();
        Ok(true)
    }
}
fn selected_hashes(session: &Session, chunks: &BTreeSet<ChunkPos>) -> BTreeMap<ChunkPos, [u8; 32]> {
    session
        .content_hashes
        .iter()
        .filter(|(chunk, _)| chunks.contains(*chunk))
        .map(|(chunk, hash)| (*chunk, *hash))
        .collect()
}
