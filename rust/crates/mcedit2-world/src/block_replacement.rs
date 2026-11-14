//! 原版多规则维度替换：每格只匹配修改前状态，完整载荷证明合为一次历史。
use super::*;
use crate::chunk_creation::ChunkCreationProfile;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Options {
    pub rules: Vec<BrushReplacement>,
    pub region: Option<SelectionRegion>,
    /// 仅补建已存在区块中的区段；不把维度边界内的空洞创建为新地形槽。
    pub creation: Option<ChunkCreationProfile>,
    pub remove_block_entities: bool,
}
#[derive(Debug, Clone)]
pub struct Prepared {
    root: PathBuf,
    dimension: String,
    revision: Arc<()>,
    pub(super) groups: Vec<EditGroup>,
    budget: EditWorkBudget,
    pub matched: usize,
    pub changed: usize,
    pub chunks: usize,
    pub sections: usize,
    pub created_sections: usize,
    pub removed_block_entities: usize,
    pub removed_inventory_stacks: usize,
    pub proof_bytes: usize,
    pub region: Option<SelectionRegion>,
}
impl Prepared {
    pub(super) fn use_work_budget(&mut self, budget: EditWorkBudget) -> Result<(), WorldError> {
        budget.check(self.proof_bytes.saturating_mul(4))?;
        self.budget = budget;
        Ok(())
    }
    pub fn summary(&self) -> String {
        format!("{}：扫描 {} 个区块 / {} 个区段，匹配 {} 格，更改 {} 格，补建 {} 个区段，移除 {} 个不兼容方块实体及 {} 个库存槽；证明 {} 字节。一次撤销，尚未保存。",
            if self.region.is_some(){"选区多规则替换"}else{"整个维度多规则替换"},self.chunks,self.sections,self.matched,self.changed,self.created_sections,
            self.removed_block_entities,self.removed_inventory_stacks,self.proof_bytes)
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(refuse("方块批量操作已取消"))
    } else {
        Ok(())
    }
}
fn intersection(a: Bounds, b: Bounds) -> Option<Bounds> {
    let minimum = std::array::from_fn(|i| a.minimum[i].max(b.minimum[i]));
    let maximum = std::array::from_fn(|i| a.maximum[i].min(b.maximum[i]));
    (0..3)
        .all(|i| minimum[i] <= maximum[i])
        .then_some(Bounds { minimum, maximum })
}
impl Session {
    /// 真实槽按区段处理，已有 BE 通过完整差异器保留或按明确授权删除。
    pub fn prepare_block_replacement_cancellable(
        &self,
        options: Options,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<Prepared, WorldError> {
        self.prepare_state_changes(options, false, None, budget, cancelled, progress)?
            .ok_or_else(|| refuse("批量替换没有实际变化"))
    }
    /// 连通工具提供真实可达格位图，逐区段载荷和提交仍复用同一替换内核。
    pub(super) fn prepare_masked_replacement_cancellable(
        &self,
        options: Options,
        cells: BrushSelection,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<Prepared, WorldError> {
        self.prepare_state_changes(options, false, Some(cells), budget, cancelled, progress)?
            .ok_or_else(|| refuse("连通填充没有实际变化"))
    }
    /// 选区删除复用逐区段准备器；只处理实际存在的区段，不为清空操作创建空气地形。
    pub(super) fn prepare_block_clear_cancellable(
        &self,
        region: SelectionRegion,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<Option<Prepared>, WorldError> {
        self.prepare_state_changes(
            Options {
                rules: Vec::new(),
                region: Some(region),
                creation: None,
                remove_block_entities: true,
            },
            true,
            None,
            budget,
            cancelled,
            progress,
        )
    }
    fn prepare_state_changes(
        &self,
        mut options: Options,
        clear: bool,
        cells: Option<BrushSelection>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Option<Prepared>, WorldError> {
        let stage = if clear {
            "准备选区方块删除"
        } else {
            "准备维度替换"
        };
        let paint = BrushPaint::Replace(options.rules);
        if !clear {
            paint.validate()?;
        }
        let BrushPaint::Replace(rules) = &paint else {
            unreachable!("已构造替换规则")
        };
        let targets = BrushReplacement::table(rules);
        let rules_bytes = cells.as_ref().map_or(0, BrushSelection::bytes)
            + rules
                .iter()
                .map(|rule| {
                    std::mem::size_of::<BrushReplacement>()
                        + clipboard::state_bytes(&rule.target)
                        + rule
                            .sources
                            .iter()
                            .map(|state| clipboard::state_bytes(state) + 64)
                            .sum::<usize>()
                })
                .sum::<usize>();
        budget.check(rules_bytes)?;
        if let Some(region) = options.region {
            options.region = Some(region.resolved(&self.world, &self.dimension)?);
        }
        if let Some(profile) = &options.creation {
            profile.verify_source(&self.world, &self.dimension)?;
        }
        let mut chunks = self.terrain_chunks_cancellable(budget, &mut cancelled)?;
        if let Some(cells) = &cells {
            let selected_chunks: BTreeSet<_> =
                cells.section_keys().map(|(x, z, _)| (x, z)).collect();
            budget.check(rules_bytes + selected_chunks.len() * 64)?;
            chunks.retain(|key| selected_chunks.contains(key));
        }
        if let Some(region) = options.region {
            chunks.retain(|&(x, z)| {
                x >= region.bounds.minimum[0].div_euclid(16)
                    && x <= region.bounds.maximum[0].div_euclid(16)
                    && z >= region.bounds.minimum[2].div_euclid(16)
                    && z <= region.bounds.maximum[2].div_euclid(16)
            });
        }
        let enumeration = chunks.len().saturating_mul(64);
        let mut chunks: Vec<_> = chunks.into_iter().collect();
        chunks.sort_unstable_by_key(|&(x, z)| (x.div_euclid(32), z.div_euclid(32), x, z));
        let mut work = self.clone();
        work.begin_recording(budget)?;
        let mut result = Prepared {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            revision: self.revision(),
            groups: Vec::new(),
            budget,
            matched: 0,
            changed: 0,
            chunks: 0,
            sections: 0,
            created_sections: 0,
            removed_block_entities: 0,
            removed_inventory_stacks: 0,
            proof_bytes: 0,
            region: options.region,
        };
        let mut examined = 0usize;
        let mut selected = 0usize;
        let mut reader_region = None;
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        for (cx, cz) in chunks {
            stop(&mut cancelled)?;
            let region_key = (cx.div_euclid(32), cz.div_euclid(32));
            if reader_region != Some(region_key) {
                reader = ReadOnlyRegionCache::for_source(
                    &self.world,
                    &self.dimension,
                    self.origin.as_ref(),
                )?;
                reader_region = Some(region_key);
            }
            let (chunk, _) = self.visible_chunk_base_cached(&mut reader, cx, cz)?;
            let chunk = chunk
                .ok_or_else(|| refuse(format!("待替换的实际地形槽 {cx},{cz} 在扫描期间消失")))?;
            let numeric = matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112);
            if clear
                && chunk.block_entities.iter().any(|&(x, y, z)| {
                    options.region.is_some_and(|r| r.contains((x, y, z)))
                        && !chunk
                            .sections
                            .iter()
                            .any(|s| i32::from(s.y) == y.div_euclid(16))
                })
            {
                return Err(refuse(
                    "选区内方块实体没有可读取的源区段，无法核对删除的方块载荷",
                ));
            }
            let air = parse_state(if numeric {
                "numeric:0[data=0]"
            } else {
                "minecraft:air"
            })?;
            let mapper = if clear {
                brush_stroke::CellPaint::Uniform(&air, None)
            } else {
                brush_stroke::CellPaint::Brush(&paint)
            };
            mapper.validate_layout(numeric)?;
            let air_target = targets.get(&air).copied().filter(|target| **target != air);
            let section_ys: Vec<i32> = if air_target.is_some() {
                let (min, max) = SelectionRegion::height(&self.world, &self.dimension)?;
                (min.div_euclid(16)..=max.div_euclid(16)).collect()
            } else {
                chunk.sections.iter().map(|s| i32::from(s.y)).collect()
            };
            let scratch = structure_work::chunk_bytes(&chunk)
                .saturating_add(reader.frozen_bytes())
                .saturating_add(enumeration)
                .saturating_add(rules_bytes);
            result.chunks += 1;
            for sy in section_ys {
                stop(&mut cancelled)?;
                if cells
                    .as_ref()
                    .is_some_and(|cells| !cells.has_section((cx, cz, sy)))
                {
                    continue;
                }
                let bounds = Bounds::spatial(
                    (cx * 16, sy * 16, cz * 16),
                    (cx * 16 + 15, sy * 16 + 15, cz * 16 + 15),
                )?;
                let bounds = if let Some(region) = options.region {
                    let Some(bounds) = intersection(bounds, region.bounds) else {
                        continue;
                    };
                    bounds
                } else {
                    bounds
                };
                let volume = bounds.volume()?;
                let mut mask = Vec::with_capacity(volume);
                for y in bounds.minimum[1]..=bounds.maximum[1] {
                    for z in bounds.minimum[2]..=bounds.maximum[2] {
                        for x in bounds.minimum[0]..=bounds.maximum[0] {
                            mask.push(
                                cells.as_ref().is_none_or(|cells| cells.contains((x, y, z)))
                                    && options
                                        .region
                                        .is_none_or(|region| region.contains((x, y, z))),
                            );
                        }
                    }
                }
                examined += volume;
                let selected_here = mask.iter().filter(|&&value| value).count();
                if selected_here == 0 {
                    continue;
                }
                selected += selected_here;
                result.sections += 1;
                let retained = work.recording.as_ref().expect("已开始组合记录").charged;
                let remaining = budget
                    .bytes()
                    .checked_sub(retained.saturating_mul(4).saturating_add(scratch))
                    .filter(|&value| value > 0)
                    .ok_or_else(|| refuse("批量替换的证明与区块工作集超出预算，请增加任务预算"))?;
                let step_budget = EditWorkBudget::from_bytes(remaining)?;
                let exists = chunk
                    .sections
                    .iter()
                    .any(|section| i32::from(section.y) == sy);
                if !clear
                    && exists
                    && !chunk
                        .sections
                        .iter()
                        .filter(|section| i32::from(section.y) == sy)
                        .flat_map(|section| &section.palette)
                        .any(|state| targets.contains_key(state))
                    && !self
                        .display_changes_in_chunk(cx, cz)
                        .any(|(position, state)| {
                            position.1.div_euclid(16) == sy && targets.contains_key(state)
                        })
                {
                    progress(EditWorkProgress {
                        stage,
                        examined,
                        selected,
                        changed: result.changed,
                        slices: result.sections,
                    });
                    continue;
                }
                let prepared = if exists {
                    let mut snapshot = work.capture_block_states_masked(
                        bounds,
                        true,
                        step_budget,
                        Some(&mask),
                        &mut cancelled,
                    )?;
                    if clear {
                        snapshot.source_bound = false;
                        snapshot.block_entities.clear();
                        for (index, state) in snapshot.blocks.iter_mut().enumerate() {
                            if mask[index] {
                                *state = air.clone();
                                result.matched += 1;
                            }
                        }
                        let cleared = work.prepare_structure_paste_with_budget_cancellable(
                            &snapshot,
                            bounds.minimum.into(),
                            clipboard::PasteOptions {
                                include_biomes: false,
                            },
                            None,
                            step_budget,
                            &mut cancelled,
                        )?;
                        cleared.has_changes().then_some(cleared)
                    } else {
                        let mut replacements = snapshot.blocks.clone();
                        for (index, before) in snapshot.blocks.iter().enumerate() {
                            if !mask[index] {
                                continue;
                            }
                            if let Some(after) = targets.get(before) {
                                result.matched += 1;
                                replacements[index] = (**after).clone();
                            }
                        }
                        work.prepare_block_difference_with_budget_cancellable(
                            &snapshot,
                            &replacements,
                            step_budget,
                            &mut cancelled,
                        )?
                    }
                } else {
                    // 缺失区段的空气只在准确的创建合同下物化，不推断 biome 或天空光。
                    let target = air_target.expect("只有替换空气才枚举缺失区段");
                    let profile = options.creation.clone().ok_or_else(|| {
                        refuse("替换空气遇到缺失区段，请启用补建并指定准确维度与 biome 合同")
                    })?;
                    let size =
                        std::array::from_fn(|i| (bounds.maximum[i] - bounds.minimum[i] + 1) as u16);
                    let mut clip = clipboard::new_with_budget(
                        &self.world,
                        &self.dimension,
                        numeric,
                        bounds.minimum,
                        size,
                        vec![target.clone(); volume],
                        step_budget,
                    )?;
                    clip.mask = Some(mask);
                    result.matched += selected_here;
                    let prepared = work.prepare_structure_paste_with_budget_cancellable(
                        &clip,
                        bounds.minimum.into(),
                        clipboard::PasteOptions {
                            include_biomes: false,
                        },
                        Some(profile),
                        step_budget,
                        &mut cancelled,
                    )?;
                    result.created_sections += 1;
                    Some(prepared)
                };
                if let Some(prepared) = prepared {
                    if prepared.overwritten_block_entities > 0 && !options.remove_block_entities {
                        return Err(refuse(
                            "批量替换将移除不兼容方块实体，请明确允许删除载荷后重新预检",
                        ));
                    }
                    result.changed += prepared.blocks.len();
                    result.removed_block_entities += prepared.overwritten_block_entities;
                    result.removed_inventory_stacks += prepared.overwritten_inventory_stacks;
                    work.commit_structure_cancellable(prepared, &mut cancelled)?;
                }
                progress(EditWorkProgress {
                    stage,
                    examined,
                    selected,
                    changed: result.changed,
                    slices: result.sections,
                });
            }
        }
        stop(&mut cancelled)?;
        let recording = work.recording.take().expect("已开始组合记录");
        if recording.groups.is_empty() {
            return Ok(None);
        }
        budget.check(
            recording
                .charged
                .saturating_mul(4)
                .saturating_add(enumeration),
        )?;
        result.proof_bytes = recording.charged;
        result.groups = recording.groups;
        progress(EditWorkProgress {
            stage: if clear {
                "选区方块删除准备完成"
            } else {
                "维度替换准备完成"
            },
            examined,
            selected,
            changed: result.changed,
            slices: result.sections,
        });
        Ok(Some(result))
    }
    /// 在私有 Session 重放实际结构证明，全部成功后只发布一条组合历史。
    pub fn commit_block_replacement_cancellable(
        &mut self,
        prepared: Prepared,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<usize, WorldError> {
        if prepared.root != self.world.root
            || prepared.dimension != self.dimension
            || !Arc::ptr_eq(&prepared.revision, &self.revision())
            || self.recording.is_some()
        {
            return Err(refuse("批量替换候选已过期或属于其他会话"));
        }
        self.ensure_history_slot(false)?;
        prepared
            .budget
            .check(prepared.proof_bytes.saturating_mul(4))?;
        let mut work = self.clone();
        work.clear_redo();
        let start = work.undo.len();
        let total = prepared.groups.len();
        work.queue_redo(prepared.groups.into_iter().rev());
        let mut replayed = 0usize;
        for index in 0..total {
            stop(&mut cancelled)?;
            let changed = match work.redo.last() {
                Some(EditGroup::Structure(group)) => group.blocks.len(),
                _ => unreachable!("批量替换只记录完整结构步骤"),
            };
            if !work.redo_with_cancel(&mut cancelled)? {
                return Err(refuse("批量替换重放步骤缺失"));
            }
            replayed += changed;
            progress(EditWorkProgress {
                stage: "重放维度替换",
                examined: replayed,
                selected: prepared.changed,
                changed: replayed,
                slices: index + 1,
            });
        }
        let groups = work.undo.split_off(start);
        work.undo_ids.truncate(start);
        work.record_edit(EditGroup::Compound(groups))?;
        stop(&mut cancelled)?;
        *self = work;
        progress(EditWorkProgress {
            stage: "维度替换暂存完成",
            examined: prepared.changed,
            selected: prepared.changed,
            changed: prepared.changed,
            slices: total,
        });
        Ok(prepared.changed)
    }
}
