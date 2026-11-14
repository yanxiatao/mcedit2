//! 生物群系笔画只修改真实 biome 容器，复用结构事务的预览、历史与保存证明。
use super::*;

/// 数字 ID 与命名 ID 属于不同注册表；刷子不在两者之间猜测转换。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BiomeTarget {
    Numeric(i32),
    Named(String),
}
impl BiomeTarget {
    pub fn parse(text: &str) -> Result<Self, WorldError> {
        let text = text.trim();
        if let Ok(id) = text.parse::<i32>() {
            return Ok(Self::Numeric(id));
        }
        if !crate::biome_editing::valid_id(text) {
            return Err(refuse("biome 目标应为数字 ID 或 namespace:path"));
        }
        Ok(Self::Named(text.to_owned()))
    }
}

/// 完整笔画的真实单元差异与待提交事务，多个笔触合并为一次撤销。
#[derive(Debug, Clone)]
pub struct BiomeBrush {
    prepared: PreparedStructure,
    pub columns: usize,
    pub quart_cells: usize,
    /// 原值、目标值和实际存储单元数量；用于没有数字颜色注册表时的审核。
    pub transitions: BTreeMap<(String, String), usize>,
}
impl BiomeBrush {
    pub fn count(&self) -> usize {
        self.columns + self.quart_cells
    }
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "生物群系：{} 个整列、{} 个 4×4×4 单元；一次撤销。",
            self.columns, self.quart_cells
        );
        for ((before, after), count) in &self.transitions {
            summary.push_str(&format!("\n{before} → {after}：{count}"));
        }
        if self
            .prepared
            .biome_proofs
            .iter()
            .any(|p| p.numeric.is_some())
        {
            summary
                .push_str("\n数字 biome 颜色由已加载的显式气候 / 颜色表决定；请同时审核实际 ID。");
        }
        summary
    }
    pub fn into_structure(self) -> PreparedStructure {
        self.prepared
    }
}

impl Session {
    /// 将笔触并集投影到实际存储单元。旧二维格式改变整列，三维格式改变触及的完整四格单元。
    /// 不创建缺失区块、section 或 biome；任一目标错误时不发布部分笔画。
    pub fn prepare_biome_brush_cancellable(
        &self,
        strokes: &[Bounds],
        mask: BrushMask,
        target: &BiomeTarget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<BiomeBrush, WorldError> {
        self.plan_biome_brush_cancellable(strokes, mask, target, cancelled)?
            .ok_or_else(|| refuse("biome 笔画没有实际变化"))
    }
    /// 材质光标也需要无变化的有效计划；None 明确表示目标值已经相同。
    pub fn plan_biome_brush_cancellable(
        &self,
        strokes: &[Bounds],
        mask: BrushMask,
        target: &BiomeTarget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<BiomeBrush>, WorldError> {
        let budget = EditWorkBudget::default();
        let selection = mask.select(
            strokes.iter().copied().map(Ok),
            budget,
            &mut cancelled,
            |_| {},
        )?;
        self.plan_biome_selection_cancellable(selection, target, budget, cancelled, |_| {})
    }
    pub fn plan_biome_selection_cancellable(
        &self,
        selection: BrushSelection,
        target: &BiomeTarget,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Option<BiomeBrush>, WorldError> {
        let mut positions = selection.iter().peekable();
        let mut retained = selection.bytes();
        let mut examined = 0;
        let mut slices = 0;
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut prepared = PreparedStructure::payload_transaction(self, budget);
        let mut columns = 0;
        let mut quart_cells = 0;
        let mut transitions = BTreeMap::new();
        while let Some(&(x, _, z)) = positions.peek() {
            let chunk = (x.div_euclid(16), z.div_euclid(16));
            if cancelled() {
                return Err(refuse("biome 笔画已取消"));
            }
            let snapshot = self.creation.get(&chunk);
            let mut input = structure_payload_input(
                &mut reader,
                chunk,
                snapshot,
                snapshot,
                self.biome_overlay.get(&chunk),
                self.biome_source.get(&chunk),
            )?;
            budget.check(
                retained
                    .checked_add(
                        (input.before.len() + input.after.len())
                            .checked_mul(8)
                            .ok_or_else(|| refuse("群系区块工作集溢出"))?,
                    )
                    .ok_or_else(|| refuse("群系任务工作集溢出"))?,
            )?;
            let decoded = decode_sections(&input.after).map_err(|e| refuse(e.to_string()))?;
            let cells = std::iter::from_fn(|| {
                positions
                    .peek()
                    .copied()
                    .filter(|p| (p.0.div_euclid(16), p.2.div_euclid(16)) == chunk)?;
                positions.next()
            });
            if decoded.x != Some(chunk.0) || decoded.z != Some(chunk.1) {
                return Err(refuse("biome 区块坐标与目标不符"));
            }
            let proof = match target {
                BiomeTarget::Numeric(replacement) => {
                    let data = decoded.numeric_biomes.as_ref().ok_or_else(|| {
                        refuse("目标没有数字 biome 容器；请使用该世界真实的命名 ID")
                    })?;
                    let mut indices = BTreeSet::new();
                    for (x, y, z) in cells {
                        if examined % 4096 == 0 && cancelled() {
                            return Err(refuse("群系编辑准备已取消"));
                        }
                        examined += 1;
                        indices.insert(
                            data.index(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                                .ok_or_else(|| refuse("数字 biome 高度合同未绑定或坐标越界"))?,
                        );
                    }
                    let mut edits = Vec::new();
                    for index in indices {
                        let original = data.values[index];
                        if original != *replacement {
                            *transitions
                                .entry((original.to_string(), replacement.to_string()))
                                .or_insert(0) += 1;
                            edits.push(crate::numeric_biome::Edit {
                                index: index as u16,
                                original,
                                replacement: *replacement,
                            });
                        }
                    }
                    if data.storage == crate::numeric_biome::Storage::IntQuarts {
                        quart_cells += edits.len();
                    } else {
                        columns += edits.len();
                    }
                    crate::biome_editing::Proof {
                        chunk,
                        edits: Vec::new(),
                        numeric: Some(crate::numeric_biome::Proof {
                            storage: data.storage,
                            edits,
                        }),
                    }
                }
                BiomeTarget::Named(replacement) => {
                    if !crate::biome_editing::valid_id(replacement) {
                        return Err(refuse("biome 命名 ID 无效"));
                    }
                    if decoded.numeric_biomes.is_some() {
                        return Err(refuse("目标使用数字 biome；请提供该世界真实的数字 ID"));
                    }
                    let mut quarts = BTreeSet::new();
                    for (x, y, z) in cells {
                        if examined % 4096 == 0 && cancelled() {
                            return Err(refuse("群系编辑准备已取消"));
                        }
                        examined += 1;
                        quarts.insert((
                            x.rem_euclid(16) as u8 / 4,
                            y.div_euclid(4),
                            z.rem_euclid(16) as u8 / 4,
                        ));
                    }
                    let mut edits = Vec::new();
                    for (x, y, z) in quarts {
                        let original = decoded
                            .biome(x * 4, y * 4, z * 4)
                            .ok_or_else(|| refuse("目标 biome 或 section 缺失"))?;
                        if original != replacement {
                            *transitions
                                .entry((original.to_owned(), replacement.clone()))
                                .or_insert(0) += 1;
                            edits.push(crate::biome_editing::Edit {
                                chunk,
                                quart_x: x,
                                quart_y: y,
                                quart_z: z,
                                original: original.to_owned(),
                                replacement: replacement.clone(),
                            });
                        }
                    }
                    quart_cells += edits.len();
                    crate::biome_editing::Proof {
                        chunk,
                        edits,
                        numeric: None,
                    }
                }
            };
            let changed = !proof.edits.is_empty()
                || proof.numeric.as_ref().is_some_and(|p| !p.edits.is_empty());
            if changed {
                let proof_cost = compound::proof_bytes(&proof)?;
                input.after = proof.replay(&input.after)?;
                retained = retained
                    .checked_add(
                        (input.before.len() + input.after.len() + proof_cost)
                            .checked_mul(4)
                            .ok_or_else(|| refuse("群系候选工作集溢出"))?,
                    )
                    .ok_or_else(|| refuse("群系候选工作集溢出"))?;
                budget.check(retained)?;
                prepared
                    .target_sources
                    .insert(chunk, input.source_sha.clone());
                if let Some(snapshot) = snapshot {
                    prepared
                        .creation_before
                        .insert(chunk, Some(snapshot.clone()));
                    prepared.creation_after.insert(chunk, snapshot.clone());
                }
                prepared.biome_chunks.push(input);
                prepared.biome_proofs.push(proof);
            }
            slices += 1;
            progress(EditWorkProgress {
                stage: "准备生物群系候选",
                examined,
                selected: selection.len(),
                changed: columns + quart_cells,
                slices,
            });
        }
        if cancelled() {
            return Err(refuse("群系编辑准备已取消"));
        }
        if !prepared.has_changes() {
            return Ok(None);
        }
        Ok(Some(BiomeBrush {
            prepared,
            columns,
            quart_cells,
            transitions,
        }))
    }
}
