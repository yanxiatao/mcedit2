//! 原版连通工具的方向和状态匹配；可达位图复用完整结构替换及一次组合历史。
use super::*;

#[derive(Debug, Clone)]
pub struct Options {
    /// None 表示当前维度的实际地形槽；不跨越或创建区块空洞。
    pub region: Option<SelectionRegion>,
    pub seed: Position,
    pub replacement: State,
    /// 顺序为 X−、X+、Y−、Y+、Z−、Z+；全部关闭时仅处理种子。
    pub directions: [bool; 6],
    /// 数字状态忽略 metadata；命名状态忽略属性。旧草方块与泥土沿用原版同组规则。
    pub ignore_properties: bool,
    /// 缺失区段的隐式空气只在明确合同下物化，不新建地形槽。
    pub creation: Option<crate::chunk_creation::ChunkCreationProfile>,
    pub remove_block_entities: bool,
}
pub(crate) const DIRECTIONS: [(usize, i32); 6] =
    [(0, -1), (0, 1), (1, -1), (1, 1), (2, -1), (2, 1)];
pub(crate) fn matching(seed: &State, state: &State, ignore: bool) -> bool {
    fn name(state: &State, ignore: bool) -> &str {
        if ignore && state.name == "numeric:2" {
            "numeric:3"
        } else {
            &state.name
        }
    }
    name(seed, ignore) == name(state, ignore)
        && (ignore
            || matches!(
                seed.name.as_str(),
                "numeric:8"
                    | "numeric:9"
                    | "numeric:10"
                    | "numeric:11"
                    | "minecraft:water"
                    | "minecraft:lava"
            )
            || seed.properties == state.properties)
}
fn effective(session: &Session, chunk: &Chunk, position: Position) -> Result<State, WorldError> {
    let (x, y, z) = position;
    if let Some(state) = session.overlay.get(&position) {
        return Ok(state.clone());
    }
    match chunk.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8) {
        Some(state) => Ok(state.clone()),
        None => parse_state(
            if matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112) {
                "numeric:0[data=0]"
            } else {
                "minecraft:air"
            },
        ),
    }
}
impl Session {
    pub fn prepare_flood_fill_cancellable(
        &self,
        mut options: Options,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<replacement::Prepared, WorldError> {
        if let Some(region) = options.region {
            options.region = Some(region.resolved(&self.world, &self.dimension)?);
        }
        if let Some(profile) = &options.creation {
            profile.verify_source(&self.world, &self.dimension)?;
        }
        let height = SelectionRegion::height(&self.world, &self.dimension)?;
        let terrain = self.terrain_chunks_cancellable(budget, &mut cancelled)?;
        let seed = options.seed;
        Bounds::spatial(seed, seed)?;
        let within = |p: Position| {
            p.1 >= height.0
                && p.1 <= height.1
                && options.region.is_none_or(|region| region.contains(p))
                && terrain.contains(&(p.0.div_euclid(16), p.2.div_euclid(16)))
        };
        if !within(seed) {
            return Err(refuse("连通填充种子必须位于所选范围的实际地形中"));
        }
        let mut reader = ReadOnlyRegionCache::new(&self.world, &self.dimension)?;
        let mut cache = BTreeMap::new();
        let mut order = VecDeque::new();
        let mut used = 0;
        let (first, _, _, _) = source(
            self,
            &mut reader,
            &mut cache,
            &mut order,
            &mut used,
            (seed.0.div_euclid(16), seed.2.div_euclid(16)),
            budget,
            terrain.len() * 64,
        )?;
        let seed_state = effective(self, first, seed)?;
        let mut visited = BrushSelection::default();
        let mut cells = BrushSelection::default();
        visited.visit(seed, budget)?;
        let mut pending = VecDeque::from([seed]);
        let mut identities = BTreeMap::<ChunkPos, String>::new();
        let mut states = BTreeMap::<String, State>::new();
        let mut states_bytes = 0;
        let mut examined = 0usize;
        let mut matched = 0usize;
        while let Some(position) = pending.pop_front() {
            if examined % 4096 == 0 {
                if cancelled() {
                    return Err(refuse("连通填充已取消"));
                }
                progress(EditWorkProgress {
                    stage: "读取实际可达方块",
                    examined,
                    selected: matched,
                    changed: cells.len(),
                    slices: examined.div_ceil(4096),
                });
            }
            let retained = visited.bytes()
                + cells.bytes()
                + pending.capacity() * std::mem::size_of::<Position>()
                + terrain.len() * 64
                + identities.len() * 160
                + states_bytes;
            budget.check(retained + used)?;
            let key = (position.0.div_euclid(16), position.2.div_euclid(16));
            let (chunk, _, _, identity) = source(
                self,
                &mut reader,
                &mut cache,
                &mut order,
                &mut used,
                key,
                budget,
                retained,
            )?;
            match identities.get(&key) {
                Some(previous) if previous != identity => {
                    return Err(refuse("连通遍历期间源区块已改变"))
                }
                Some(_) => {}
                None => {
                    identities.insert(key, identity.clone());
                }
            }
            let state = effective(self, chunk, position)?;
            examined += 1;
            if !matching(&seed_state, &state, options.ignore_properties) {
                continue;
            }
            brush_stroke::CellPaint::Uniform(&options.replacement, None).validate_layout(
                matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112),
            )?;
            matched += 1;
            if state != options.replacement {
                if chunk
                    .block(
                        position.0.rem_euclid(16) as u8,
                        position.1,
                        position.2.rem_euclid(16) as u8,
                    )
                    .is_none()
                    && options.creation.is_none()
                {
                    return Err(refuse(
                        "连通填充到达缺失区段，请启用补建并明确维度与 biome 合同",
                    ));
                }
                cells.visit(position, budget)?;
                let name = render_state(&state);
                if !states.contains_key(&name) {
                    states_bytes += clipboard::state_bytes(&state) + name.len() + 96;
                    states.insert(name, state);
                }
            }
            for (enabled, (axis, delta)) in options.directions.into_iter().zip(DIRECTIONS) {
                if !enabled {
                    continue;
                }
                let mut next = [position.0, position.1, position.2];
                let coordinate = next[axis]
                    .checked_add(delta)
                    .ok_or_else(|| refuse("连通扩散坐标溢出"))?;
                next[axis] = coordinate;
                let next = next.into();
                if within(next) && visited.visit(next, budget)? {
                    budget.check(
                        retained
                            + used
                            + cells.bytes()
                            + (pending.len() + 1) * std::mem::size_of::<Position>() * 2
                            + states_bytes,
                    )?;
                    pending
                        .try_reserve(1)
                        .map_err(|error| refuse(format!("无法分配连通队列：{error}")))?;
                    pending.push_back(next);
                }
            }
        }
        if cancelled() {
            return Err(refuse("连通填充已取消"));
        }
        if cells.is_empty() {
            return Err(refuse("连通填充没有实际变化"));
        }
        drop(cache);
        drop(reader);
        drop(visited);
        drop(pending);
        drop(terrain);
        progress(EditWorkProgress {
            stage: "准备可达格完整载荷",
            examined,
            selected: matched,
            changed: cells.len(),
            slices: identities.len(),
        });
        let receipts = identities.len() * 160;
        let step_budget = EditWorkBudget::from_bytes(
            budget
                .bytes()
                .checked_sub(receipts)
                .ok_or_else(|| refuse("连通来源证明超出任务预算"))?,
        )?;
        let mut prepared = self.prepare_masked_replacement_cancellable(
            replacement::Options {
                rules: vec![BrushReplacement {
                    sources: states.into_values().collect(),
                    target: options.replacement,
                }],
                region: options.region,
                creation: options.creation,
                remove_block_entities: options.remove_block_entities,
            },
            cells,
            step_budget,
            &mut cancelled,
            &mut progress,
        )?;
        let Some(EditGroup::Structure(group)) = prepared.groups.first_mut() else {
            unreachable!("非空连通替换使用完整结构步骤");
        };
        group.work_budget = budget;
        for (key, identity) in identities {
            if let Some(previous) = group.source_terrain_sha.insert(key, identity.clone()) {
                if previous != identity {
                    return Err(refuse("连通遍历后源区块已改变"));
                }
            }
            if let Some(snapshot) = self.creation.get(&key) {
                group.source_creation.insert(key, snapshot.clone());
            }
        }
        prepared.proof_bytes = prepared.groups.iter().try_fold(0usize, |bytes, group| {
            bytes
                .checked_add(compound::group_bytes(group)?)
                .ok_or_else(|| refuse("连通填充证明大小溢出"))
        })?;
        prepared.use_work_budget(budget)?;
        Ok(prepared)
    }
}
