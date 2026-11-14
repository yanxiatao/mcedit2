//! 冻结来源的完整输出统一交给世界或结构内核，供启动脚本及持久工具共用。
use super::*;

pub struct AppliedOutput {
    pub changed: usize,
    pub objects: Option<snapshot::ObjectResults>,
}
impl Working {
    pub fn apply_frozen_output(
        &mut self,
        source: u64,
        blocks: &[State],
        content: &snapshot::Content,
        generated: Option<Arc<BlockClipboard>>,
        creation: Option<crate::chunk_creation::ChunkCreationProfile>,
    ) -> Result<AppliedOutput, WorldError> {
        self.stop()?;
        let (world, dimension) = {
            let frozen = self
                .snapshots
                .get(&source)
                .ok_or_else(|| refuse("脚本输出来源租约已释放"))?;
            (frozen.world, frozen.dimension.clone())
        };
        if self.world_slots.contains_key(&world) {
            self.entry(world, &dimension)?;
            self.bind_entity_context(world, &dimension);
        }
        let frozen = &self.snapshots[&source];
        let clip = frozen.content.clipboard();
        let readonly = self.read_only(world);
        if readonly {
            if generated.is_some()
                || blocks != clip.blocks
                || *content != snapshot::Content::capture(clip)?
            {
                return Err(refuse("只读来源禁止脚本修改"));
            }
            return Ok(AppliedOutput {
                changed: 0,
                objects: None,
            });
        }
        let budget = self.transient_budget()?;
        let token = Arc::clone(&self.cancelled);
        let (changed, objects) = match &frozen.content {
            FrozenContent::World(clip) => {
                let clip = Arc::clone(clip);
                let entry = self.entry(world, &dimension)?;
                let mut work = entry.working.clone();
                let prepared = if let Some(generated) = generated {
                    work.prepare_script_composition_cancellable(
                        &clip,
                        blocks,
                        content,
                        &generated,
                        creation,
                        budget,
                        || token.load(Ordering::Acquire),
                    )?
                } else {
                    if creation.is_some() {
                        return Err(refuse("没有生成结构时不能提交区块创建合同"));
                    }
                    work.prepare_snapshot_difference_cancellable(
                        &clip,
                        blocks,
                        content,
                        budget,
                        || token.load(Ordering::Acquire),
                    )?
                };
                let objects = prepared
                    .as_ref()
                    .and_then(|prepared| prepared.object_results().cloned());
                let changed = if let Some(prepared) = prepared {
                    let changed = work.commit_snapshot_difference_cancellable(
                        prepared,
                        || token.load(Ordering::Acquire),
                        |_| {},
                    )?;
                    work.fold_script_history(&entry.baseline, budget)?;
                    entry.working = work;
                    changed
                } else {
                    0
                };
                (changed, objects)
            }
            FrozenContent::Structure(source) => {
                if creation.is_some() {
                    return Err(refuse("独立结构不能提交世界区块创建合同"));
                }
                let source = Arc::clone(source);
                let entry = self
                    .structures
                    .get_mut(&world)
                    .expect("冻结结构属于实际结构文档");
                let mut work = entry.working.clone();
                let prepared = if let Some(generated) = generated {
                    work.prepare_script_composition_cancellable(
                        &source,
                        blocks,
                        content,
                        generated,
                        budget,
                        || token.load(Ordering::Acquire),
                    )?
                } else {
                    work.prepare_script_difference_cancellable(
                        &source,
                        blocks,
                        content,
                        budget,
                        || token.load(Ordering::Acquire),
                    )?
                };
                let objects = prepared.object_results().cloned();
                let changed =
                    prepared.changed_blocks + prepared.changed_objects + prepared.changed_biomes;
                work.commit_script_cancellable(&prepared, || token.load(Ordering::Acquire))?;
                entry.working = work;
                (changed, objects)
            }
        };
        self.observed.insert((world, dimension));
        self.stop()?;
        Ok(AppliedOutput { changed, objects })
    }
}
