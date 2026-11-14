//! 启动脚本结构分支；真正的文件文档参与同一来源租约和整组完成流程。
use super::*;
impl Working {
    pub(super) fn invoke_structure(&mut self, call: Call) -> Result<Value, WorldError> {
        let slot = call.world;
        let dimension = self.structures[&slot].working.content().dimension.clone();
        if call.dimension != dimension {
            return Err(refuse("结构文件只包含其真实维度"));
        }
        let key = (slot, dimension.clone());
        self.observed.insert(key.clone());
        let token = Arc::clone(&self.cancelled);
        let budget = self.transient_budget()?;
        let entry = &self.structures[&slot];
        let mut work = entry.working.script_scratch();
        let value = match call.action {
            Action::Bounds => return Ok(json!({"bounds":work.script_bounds()?})),
            Action::Read { position } => {
                return serde_json::to_value(work.read_script_block(position)?)
                    .map_err(|error| refuse(error.to_string()))
            }
            Action::Resolve { state } => {
                let content = work.content();
                return serde_json::to_value(resolve_state(
                    state,
                    content.numeric,
                    content.source_release.as_deref(),
                )?)
                .map_err(|error| refuse(error.to_string()));
            }
            Action::Mask { region } => {
                let local = work.script_local_region(region)?;
                return serde_json::to_value(local.mask(budget, || token.load(Ordering::Acquire))?)
                    .map_err(|error| refuse(error.to_string()));
            }
            Action::Capture {
                region,
                entities,
                biomes,
            } => return self.capture(slot, dimension, region, entities, biomes, None),
            Action::CaptureMasked {
                region,
                mask,
                entities,
                biomes,
            } => return self.capture(slot, dimension, region, entities, biomes, mask),
            Action::Selection { region } => {
                if let Some(region) = region {
                    work.script_local_region(region)?;
                }
                self.selections.insert(key.clone(), region);
                self.changed_selections.insert(key);
                return serde_json::to_value(region).map_err(|error| refuse(error.to_string()));
            }
            Action::UseDimension => return Ok(Value::Null),
            Action::Focus { position, distance } => {
                if !position.iter().all(|value| value.is_finite())
                    || !distance.is_finite()
                    || distance <= 0.
                {
                    return Err(refuse("定位需要有限坐标及正距离"));
                }
                self.focus = Some(Focus {
                    world: slot,
                    dimension,
                    position,
                    distance,
                });
                return Ok(Value::Null);
            }
            Action::Save => {
                if self.readonly || work.read_only() {
                    return Err(refuse("只读结构禁止脚本保存"));
                }
                if !self.save_worlds.contains(&slot) {
                    self.save_worlds.push(slot);
                }
                return Ok(Value::Null);
            }
            Action::CreationProfile { .. } => {
                return Err(refuse("结构文件没有世界地形维度，不能创建世界区块合同"))
            }
            Action::Copy {
                source,
                target,
                input,
            } => {
                let source = self
                    .snapshots
                    .get(&source)
                    .ok_or_else(|| refuse("复制的实际冻结来源已经释放"))?
                    .content
                    .clipboard();
                let target = self.frozen(target, slot, &dimension)?;
                let result =
                    source.copy_script_dimension_cancellable(target, input, budget, || {
                        token.load(Ordering::Acquire)
                    })?;
                self.stop()?;
                return serde_json::to_value(result).map_err(|error| refuse(error.to_string()));
            }
            Action::Generate { source, input } => {
                let result = self
                    .frozen(source, slot, &dimension)?
                    .materialize_generated(input, budget)?;
                self.stop()?;
                return serde_json::to_value(result).map_err(|error| refuse(error.to_string()));
            }
            Action::Apply {
                snapshot,
                blocks,
                content,
            } => {
                let source = self
                    .snapshots
                    .iter()
                    .find_map(|(&id, frozen)| {
                        let current = match &frozen.content {
                            FrozenContent::Structure(source) => source.matches(&work),
                            FrozenContent::World(_) => false,
                        };
                        (current
                            && frozen.world == slot
                            && frozen.dimension == dimension
                            && frozen.content.clipboard() == &snapshot)
                            .then_some(id)
                    })
                    .ok_or_else(|| refuse("结构脚本提交不属于当前实际捕获的来源"))?;
                return self.invoke(Call {
                    world: slot,
                    dimension,
                    action: Action::ApplyFrozen {
                        source,
                        blocks,
                        content,
                    },
                });
            }
            action => {
                if self.readonly || work.read_only() {
                    return Err(refuse("只读模式禁止结构脚本修改"));
                }
                match action {
                    Action::Fill {
                        region,
                        state,
                        only_if,
                    } => json!(work.fill_startup(region, state, only_if, budget, || token
                        .load(Ordering::Acquire))?),
                    Action::Delete {
                        region,
                        mode,
                        keep_ids,
                    } => {
                        let region = work.script_local_region(region)?;
                        let mode = match mode.as_str() {
                            "blocks" => deletion::Mode::Blocks,
                            "entities" => deletion::Mode::Entities,
                            "entities_without_items" => deletion::Mode::EntitiesWithoutItems,
                            "all" => deletion::Mode::All,
                            _ => return Err(refuse("未知脚本删除方式")),
                        };
                        let counts = work.delete_selection(
                            deletion::Options {
                                region,
                                mode,
                                keep_ids,
                            },
                            budget,
                            || token.load(Ordering::Acquire),
                        )?;
                        json!(counts.blocks + counts.block_entities + counts.entities)
                    }
                    Action::PasteSource {
                        source,
                        input,
                        destination,
                        creation,
                        ..
                    } => {
                        if creation.is_some() {
                            return Err(refuse("结构文件不能使用世界区块创建合同"));
                        }
                        let biomes = input.biomes;
                        let source = self
                            .snapshots
                            .get(&source)
                            .ok_or_else(|| refuse("复制的真实来源已释放"))?
                            .content
                            .clipboard();
                        let clip = source.script_source_cancellable(input, budget, || {
                            token.load(Ordering::Acquire)
                        })?;
                        json!(
                            work.paste_startup(&clip, destination, biomes, budget, || token
                                .load(Ordering::Acquire))?
                        )
                    }
                    _ => unreachable!("只剩结构修改操作"),
                }
            }
        };
        self.stop()?;
        self.structures
            .get_mut(&slot)
            .expect("结构脚本会话存在")
            .working = work;
        Ok(value)
    }
}
