//! 结构传输的计划更新来源捕获；使用与方块实体相同的实际地形载荷基底。
use super::*;
impl Session {
    pub(super) fn apply_snapshot_ticks(
        &mut self,
        source: &BlockClipboard,
        incoming: &[crate::scheduled_ticks::Tick],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        let mut maximum = [0; 3];
        for axis in 0..3 {
            maximum[axis] = source.source_min[axis]
                .checked_add(i32::from(source.size[axis]) - 1)
                .ok_or_else(|| refuse("脚本计划更新范围溢出"))?;
        }
        let bounds = Bounds {
            minimum: source.source_min,
            maximum,
        };
        let (current, _) =
            self.capture_structure_ticks(bounds, source.mask.as_deref(), budget, &mut cancelled)?;
        let mut before = BTreeMap::<[i32; 3], Vec<crate::scheduled_ticks::Tick>>::new();
        for mut tick in current {
            for axis in 0..3 {
                tick.position[axis] += source.source_min[axis];
            }
            before.entry(tick.position).or_default().push(tick);
        }
        let mut after = BTreeMap::<[i32; 3], Vec<crate::scheduled_ticks::Tick>>::new();
        for tick in incoming {
            if !(0..3).all(|axis| {
                tick.position[axis] >= bounds.minimum[axis]
                    && tick.position[axis] <= bounds.maximum[axis]
            }) || source
                .mask
                .as_ref()
                .is_some_and(|mask| !mask[structure_work::index(bounds, tick.position.into())])
            {
                return Err(refuse("脚本计划更新超出冻结选区"));
            }
            if crate::scheduled_ticks::compound_position(&tick.compound)? != tick.position {
                return Err(refuse("脚本计划更新坐标与载荷不一致"));
            }
            after.entry(tick.position).or_default().push(tick.clone());
        }
        let mut touched = BTreeMap::<ChunkPos, BTreeSet<[i32; 3]>>::new();
        for &position in before.keys().chain(after.keys()) {
            if before.get(&position) != after.get(&position) {
                touched
                    .entry((position[0].div_euclid(16), position[2].div_euclid(16)))
                    .or_default()
                    .insert(position);
            }
        }
        let mut group = PreparedStructure::payload_transaction(self, budget);
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        for (chunk, positions) in touched {
            if cancelled() {
                return Err(refuse("脚本计划更新提交已取消"));
            }
            let snapshot = self.creation.get(&chunk);
            let mut input = structure_payload_input(
                &mut reader,
                chunk,
                snapshot,
                snapshot,
                self.be_overlay.get(&chunk),
                self.be_source.get(&chunk),
            )?;
            if incoming
                .iter()
                .any(|tick| positions.contains(&tick.position))
            {
                require_clipboard_payload_version(
                    chunk,
                    "script:scheduled-ticks",
                    clipboard_payload_version(&input.after, chunk, "script:scheduled-ticks")?,
                    source.data_version,
                )?;
            }
            if let Some(proof) =
                crate::scheduled_ticks::Proof::plan(&input.after, chunk, &positions, incoming)?
            {
                input.after = proof.replay(&input.after)?;
                group.target_sources.insert(chunk, input.source_sha.clone());
                if let Some(snapshot) = snapshot {
                    group.creation_before.insert(chunk, Some(snapshot.clone()));
                    group.creation_after.insert(chunk, snapshot.clone());
                    group.source_creation.insert(chunk, snapshot.clone());
                }
                group.be_chunks.push(input);
                group.tick_proofs.push(proof);
                group.check_work(budget)?;
            }
        }
        drop(reader);
        if group.has_changes() {
            self.commit_structure_cancellable(group, cancelled)?;
        }
        Ok(())
    }
    pub(super) fn capture_structure_ticks(
        &self,
        bounds: Bounds,
        mask: Option<&[bool]>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<
        (
            Vec<crate::scheduled_ticks::Tick>,
            BTreeMap<ChunkPos, String>,
        ),
        WorldError,
    > {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut ticks = Vec::new();
        let mut sources = BTreeMap::new();
        let mut retained = 0usize;
        for cx in bounds.minimum[0].div_euclid(16)..=bounds.maximum[0].div_euclid(16) {
            for cz in bounds.minimum[2].div_euclid(16)..=bounds.maximum[2].div_euclid(16) {
                if cancelled() {
                    return Err(refuse("计划更新捕获已取消"));
                }
                // 不读取稀疏选区完全未声明的区块。
                let selected = (bounds.minimum[0].max(cx * 16)
                    ..=bounds.maximum[0].min(cx * 16 + 15))
                    .any(|x| {
                        (bounds.minimum[2].max(cz * 16)..=bounds.maximum[2].min(cz * 16 + 15)).any(
                            |z| {
                                (bounds.minimum[1]..=bounds.maximum[1]).any(|y| {
                                    mask.is_none_or(|mask| {
                                        mask[structure_work::index(bounds, (x, y, z))]
                                    })
                                })
                            },
                        )
                    });
                if !selected {
                    continue;
                }
                let key = (cx, cz);
                let (raw, identity) = self.terrain_payload_base_cached(&mut reader, key)?;
                let staged = self.be_overlay.get(&key);
                if staged.is_some() && self.be_source.get(&key) != Some(&identity) {
                    return Err(refuse("计划更新的暂存来源已变化"));
                }
                let source = staged.unwrap_or(&raw);
                structure_work::check_work(
                    budget,
                    retained.saturating_add(source.len().saturating_mul(8)),
                )?;
                let values = crate::scheduled_ticks::read_chunk(source)?;
                sources.insert(key, crate::scheduled_ticks::fingerprint(source)?);
                retained = retained
                    .checked_add(128)
                    .ok_or_else(|| refuse("计划更新来源工作集溢出"))?;
                let mut version_checked = false;
                for mut tick in values {
                    if (
                        tick.position[0].div_euclid(16),
                        tick.position[2].div_euclid(16),
                    ) != key
                    {
                        return Err(refuse("计划更新不在所属区块"));
                    }
                    if !(0..3).all(|axis| {
                        tick.position[axis] >= bounds.minimum[axis]
                            && tick.position[axis] <= bounds.maximum[axis]
                    }) || mask.is_some_and(|mask| {
                        !mask[structure_work::index(bounds, tick.position.into())]
                    }) {
                        continue;
                    }
                    if !version_checked {
                        require_clipboard_payload_version(
                            key,
                            "terrain:scheduled-ticks",
                            clipboard_payload_version(source, key, "terrain:scheduled-ticks")?,
                            self.world.data_version,
                        )?;
                        version_checked = true;
                    }
                    tick.position =
                        std::array::from_fn(|axis| tick.position[axis] - bounds.minimum[axis]);
                    retained = retained
                        .checked_add(
                            tick.compound.len()
                                + std::mem::size_of::<crate::scheduled_ticks::Tick>(),
                        )
                        .ok_or_else(|| refuse("计划更新载荷工作集溢出"))?;
                    structure_work::check_work(
                        budget,
                        retained.saturating_add(source.len().saturating_mul(8)),
                    )?;
                    ticks.push(tick);
                }
            }
        }
        Ok((ticks, sources))
    }
}
