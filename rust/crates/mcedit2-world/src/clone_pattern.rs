//! 原版式重复克隆：冻结来源、累加旋转及旋转位移；所有副本合为一个内存事务。
use super::*;
use crate::structure_transform::Transform;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClonePattern {
    pub transform: Transform,
    pub copies: u32,
    pub rotate_repeats: bool,
    pub rotate_offset: bool,
    pub copy_air: bool,
}
impl Default for ClonePattern {
    fn default() -> Self {
        Self {
            transform: Transform::default(),
            copies: 1,
            rotate_repeats: false,
            rotate_offset: false,
            copy_air: false,
        }
    }
}
impl ClonePattern {
    /// 世界与独立结构使用同一累加旋转、位移取整和旋转位移规则。
    pub fn placements(
        self,
        origin: [i32; 3],
        destination: [i32; 3],
    ) -> Result<impl Iterator<Item = Result<([i32; 3], Transform), WorldError>>, WorldError> {
        self.validate()?;
        let mut position = origin.map(f64::from);
        let mut offset: [f64; 3] =
            std::array::from_fn(|axis| f64::from(destination[axis]) - f64::from(origin[axis]));
        let mut copy = 0;
        Ok(std::iter::from_fn(move || {
            if copy == self.copies {
                return None;
            }
            copy += 1;
            let mut anchor = [0; 3];
            for axis in 0..3 {
                position[axis] += offset[axis];
                if !position[axis].is_finite()
                    || position[axis].floor() < f64::from(i32::MIN)
                    || position[axis].floor() > f64::from(i32::MAX)
                {
                    copy = self.copies;
                    return Some(Err(refuse("重复克隆位移超出坐标范围")));
                }
                anchor[axis] = position[axis].floor() as i32;
            }
            let mut transform = self.transform;
            transform.rotation = transform.rotation.map(|angle| {
                (angle.rem_euclid(360.) * f64::from(if self.rotate_repeats { copy } else { 1 }))
                    .rem_euclid(360.)
            });
            if self.rotate_offset {
                offset = self.transform.rotate_offset(offset);
            }
            Some(Ok((anchor, transform)))
        }))
    }
    pub fn validate(self) -> Result<(), WorldError> {
        self.transform.validate()?;
        if self.copies == 0 || self.copies > 10000 {
            return Err(refuse("重复克隆份数必须在 1..10000 内"));
        }
        Ok(())
    }
}
impl Session {
    /// 来源只捕获一次，后续副本读取冻结快照；重叠目标按副本顺序覆盖。
    /// 不把变换后的格子当作磁盘源，提交时重新核验整份原始来源及会话修订。
    pub fn prepare_clone_pattern_cancellable(
        &self,
        region: impl Into<SelectionRegion>,
        destination: Position,
        options: TransferOptions,
        pattern: ClonePattern,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedTransfer, WorldError> {
        let region = region.into().resolved(&self.world, &self.dimension)?;
        let bounds = region.bounds;
        pattern.validate()?;
        if options.kind != TransferKind::Clone {
            return Err(refuse("重复及空间变换仅用于克隆；移动工具执行原位移语义"));
        }
        if self.recording.is_some() {
            return Err(refuse("不能嵌套准备组合克隆"));
        }
        self.ensure_history_slot(false)?;
        if options.proof_budget < 1024 * 1024 {
            return Err(refuse("克隆证明预算至少为 1 MiB"));
        }
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
        let source_bytes = proof_bytes(&source)?;
        if source_bytes > options.proof_budget {
            return Err(refuse("冻结来源超过克隆证明与载荷预算"));
        }
        let mut work = self.clone();
        let mut frozen_pairs = BTreeSet::new();
        for entry in &source.block_entities {
            let [x, y, z] = entry.position.map(|v| v as usize);
            let block = &source.blocks
                [(y * usize::from(source.size[2]) + z) * usize::from(source.size[0]) + x];
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&entry.compound);
            let root = crate::sections::parse_root(&raw).map_err(|e| refuse(e.to_string()))?;
            let id = root
                .require("id", 8)
                .and_then(crate::sections::Tag::string)
                .map_err(|e| refuse(e.to_string()))?;
            frozen_pairs.insert((block.name.clone(), id));
        }
        work.recording = Some(Recording {
            groups: Vec::new(),
            charged: source_bytes,
            budget: options.proof_budget,
        });
        let mut cache = std::collections::HashMap::new();
        let mut block_entities = 0;
        let mut entities = 0;
        let mut overwritten_block_entities = 0;
        let mut overwritten_inventory_stacks = 0;
        let mut diagnostics = Vec::new();
        let mut placement_bounds = None;
        for placement in pattern.placements(
            bounds.minimum,
            [destination.0, destination.1, destination.2],
        )? {
            if cancelled() {
                return Err(refuse("重复克隆准备已取消"));
            }
            let (anchor, transform) = placement?;
            let key = (
                transform.rotation.map(f64::to_bits),
                transform.scale.map(f64::to_bits),
                transform.rotate_states,
            );
            if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(key) {
                let target =
                    source.resampled_with_budget_cancellable(transform, budget, &mut cancelled)?;
                let bytes = proof_bytes(&target)?;
                let recording = work.recording.as_mut().expect("克隆处于组合记录中");
                recording.charged = recording
                    .charged
                    .checked_add(bytes)
                    .ok_or_else(|| refuse("克隆载荷大小溢出"))?;
                if recording.charged > recording.budget {
                    return Err(refuse("变换后的结构超过克隆证明与载荷预算"));
                }
                entry.insert(target);
            }
            let target = &cache[&key];
            if placement_bounds.is_none() {
                placement_bounds = Some(placed_bounds(target, anchor.into())?);
            }
            let paste = work.prepare_structure_paste_with_source_cancellable(
                target,
                target,
                false,
                anchor.into(),
                clipboard::PasteOptions {
                    include_biomes: options.biomes,
                },
                profile.clone(),
                Some(&frozen_pairs),
                None,
                StructurePaste {
                    skip_air: !pattern.copy_air,
                    ..Default::default()
                },
                budget,
                &mut cancelled,
            )?;
            block_entities += target.block_entities.len();
            entities += target.entities.len();
            overwritten_block_entities += paste.overwritten_block_entities;
            overwritten_inventory_stacks += paste.overwritten_inventory_stacks;
            diagnostics.extend(paste.diagnostics.iter().cloned());
            if paste.has_changes() {
                work.commit_structure_cancellable(paste, &mut cancelled)?;
            }
        }
        let recording = work.recording.take().expect("克隆组合记录存在");
        if recording.groups.is_empty() {
            return Err(refuse("克隆没有实际变化"));
        }
        if cancelled() {
            return Err(refuse("克隆准备已取消"));
        }
        let changed_blocks = recording
            .groups
            .iter()
            .map(|g| match g {
                EditGroup::Structure(g) => g.blocks.len(),
                EditGroup::Blocks(c) => c.len(),
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
            block_entities,
            entities,
            overwritten_block_entities,
            overwritten_inventory_stacks,
            proof_bytes: recording.charged,
            diagnostics,
            candidate: Arc::new(work),
            copies: pattern.copies,
        })
    }
}
