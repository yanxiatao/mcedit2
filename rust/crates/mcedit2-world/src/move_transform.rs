//! 完整结构变换移动：冻结原稿、重采样目标、原身份实体迁移及重叠源清理。
use super::*;
use crate::{entity_editing::DimensionChange, structure_transform::Transform};

fn target_owns(
    target: &BlockClipboard,
    minimum: [i32; 3],
    position: [i32; 3],
    copy_air: bool,
) -> bool {
    let relative =
        std::array::from_fn::<_, 3, _>(|axis| i64::from(position[axis]) - i64::from(minimum[axis]));
    if relative
        .iter()
        .zip(target.size)
        .any(|(&value, size)| value < 0 || value >= i64::from(size))
    {
        return false;
    }
    let index = (relative[1] as usize * usize::from(target.size[2]) + relative[2] as usize)
        * usize::from(target.size[0])
        + relative[0] as usize;
    target.mask.as_ref().is_none_or(|mask| mask[index])
        && (copy_air || !structure_air(&target.blocks[index]))
}

impl Session {
    /// 目标实体保留原有 UUID 与整树身份，放大只重采样方块，不复制实体树。
    /// 所有领域证明先进入私有记录，确认时复核原稿并重放为一次历史。
    pub fn prepare_transformed_move_cancellable(
        &self,
        region: impl Into<SelectionRegion>,
        destination: Position,
        options: TransferOptions,
        transform: Transform,
        copy_air: bool,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedTransfer, WorldError> {
        if options.kind != TransferKind::Move {
            return Err(refuse("变换移动需要 Move 操作"));
        }
        transform.validate()?;
        if self.recording.is_some() {
            return Err(refuse("不能嵌套准备变换移动"));
        }
        self.ensure_history_slot(false)?;
        if options.proof_budget < 1024 * 1024 {
            return Err(refuse("移动证明预算至少需要 1 MiB"));
        }
        let region = region.into().resolved(&self.world, &self.dimension)?;
        let bounds = region.bounds;
        let budget = EditWorkBudget::from_bytes(options.proof_budget)?;
        let source = self.capture_structure_with_budget_cancellable(
            region,
            clipboard::CaptureOptions {
                entities: options.entities,
                biomes: options.biomes,
            },
            budget,
            &mut cancelled,
        )?;
        let (mut target, origins) =
            source.resampled_with_origins_budget_cancellable(transform, budget, &mut cancelled)?;
        let placed = placed_bounds(&target, destination)?;
        let target_minimum = placed.minimum;
        let placement_bounds = Some(placed);
        let mut delta = [0; 3];
        for axis in 0..3 {
            delta[axis] = target_minimum[axis]
                .checked_sub(source.source_min[axis])
                .ok_or_else(|| refuse("移动实体位移溢出"))?;
        }
        let charged = proof_bytes(&source)?
            .checked_add(proof_bytes(&target)?)
            .ok_or_else(|| refuse("变换移动载荷大小溢出"))?;
        budget.check(charged)?;
        let moved_entities = std::mem::take(&mut target.entities);
        assert_eq!(
            source.entities.len(),
            moved_entities.len(),
            "结构重采样保留实体根数量"
        );
        let entities = source.entities.len();
        let mut positions = crate::entity_references::Locations::new();
        for entry in &moved_entities {
            if cancelled() {
                return Err(refuse("移动实体关联整理已取消"));
            }
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&entry.compound);
            let root =
                crate::sections::parse_root(&raw).map_err(|error| refuse(error.to_string()))?;
            for (id, position) in crate::entity_references::locations(&root, delta)? {
                if positions.insert(id, position).is_some() {
                    return Err(refuse("移动选区含重复实体身份"));
                }
            }
        }
        let mut work = self.clone();
        work.recording = Some(Recording {
            groups: Vec::new(),
            charged,
            budget: options.proof_budget,
        });
        target.source_bound = source.source_bound;
        let paste = work.prepare_structure_paste_with_source_cancellable(
            &target,
            &source,
            !transform.is_identity(),
            destination,
            clipboard::PasteOptions {
                include_biomes: options.biomes,
            },
            profile,
            None,
            Some(&origins),
            StructurePaste {
                skip_air: !copy_air,
                moving_entities: &moved_entities,
            },
            budget,
            &mut cancelled,
        )?;
        let overwritten_block_entities = paste.overwritten_block_entities;
        let overwritten_inventory_stacks = paste.overwritten_inventory_stacks;
        let mut diagnostics = paste.diagnostics.clone();
        if paste.has_changes() {
            work.commit_structure_cancellable(paste, &mut cancelled)?;
        }
        for (before, after) in source.entities.iter().zip(moved_entities) {
            if cancelled() {
                return Err(refuse("完整实体变换移动已取消"));
            }
            let compound = crate::entity_editing::offset_compound(
                &after.compound,
                delta,
                source.data_version,
                &DimensionChange::within(Some(&self.dimension)),
                &positions,
            )?;
            work.rewrite_structure_entity_cancellable(
                before,
                Some(&snapshot::value(&compound)?),
                &mut cancelled,
            )?;
        }
        let mut clear =
            work.capture_block_states_with_budget_cancellable(region, budget, &mut cancelled)?;
        clear.source_bound = false;
        clear.mask = Some(vec![false; clear.blocks.len()]);
        let empty = parse_state(if clear.numeric {
            "numeric:0[data=0]"
        } else {
            "minecraft:air"
        })?;
        let width = usize::from(source.size[0]);
        let depth = usize::from(source.size[2]);
        for (index, state) in clear.blocks.iter_mut().enumerate() {
            if index.is_multiple_of(256) && cancelled() {
                return Err(refuse("变换移动源清理已取消"));
            }
            if source.mask.as_ref().is_some_and(|mask| !mask[index]) {
                continue;
            }
            let position = [
                bounds.minimum[0] + (index % width) as i32,
                bounds.minimum[1] + (index / (width * depth)) as i32,
                bounds.minimum[2] + ((index / width) % depth) as i32,
            ];
            if !target_owns(&target, target_minimum, position, copy_air) {
                clear.mask.as_mut().expect("清理掩码已创建")[index] = true;
                *state = empty.clone();
            }
        }
        if clear
            .mask
            .as_ref()
            .expect("清理掩码已创建")
            .iter()
            .any(|value| *value)
        {
            let cleanup = work.prepare_structure_paste_with_budget_cancellable(
                &clear,
                bounds.minimum.into(),
                clipboard::PasteOptions {
                    include_biomes: false,
                },
                None,
                budget,
                &mut cancelled,
            )?;
            diagnostics.extend(cleanup.diagnostics.iter().cloned());
            if cleanup.has_changes() {
                work.commit_structure_cancellable(cleanup, &mut cancelled)?;
            }
        }
        if cancelled() {
            return Err(refuse("变换移动准备已取消"));
        }
        let recording = work.recording.take().expect("移动组合记录存在");
        if recording.groups.is_empty() {
            return Err(refuse("变换移动没有实际变化"));
        }
        let changed_blocks = recording
            .groups
            .iter()
            .map(|group| match group {
                EditGroup::Structure(group) => group.blocks.len(),
                EditGroup::Blocks(changes) => changes.len(),
                _ => 0,
            })
            .sum();
        diagnostics.sort();
        diagnostics.dedup();
        Ok(PreparedTransfer {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            source: Arc::new(source),
            bounds,
            shape: region.shape,
            options,
            groups: recording.groups,
            destination,
            changed_blocks,
            placement_bounds,
            block_entities: target.block_entities.len(),
            entities,
            overwritten_block_entities,
            overwritten_inventory_stacks,
            proof_bytes: recording.charged,
            diagnostics,
            candidate: Arc::new(work),
            copies: 1,
        })
    }
}
