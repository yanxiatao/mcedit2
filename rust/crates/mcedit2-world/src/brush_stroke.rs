//! 连续笔画的整数路径、居中笔触及真实方块并集。
use super::*;

/// 先对全部笔触取形状并集，再可选地取六邻面暴露的边界。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BrushMask {
    pub shape: Option<BrushShape>,
    pub hollow: bool,
}
impl BrushMask {
    /// 光标与正式准备器共用同一份笔触选择，返回实际选中坐标。
    pub fn cells(
        self,
        strokes: &[Bounds],
        cancelled: impl FnMut() -> bool,
    ) -> Result<BTreeSet<Position>, WorldError> {
        cells(strokes, self, cancelled)
    }
}

/// 同一规则的多个完整源状态映射到一个目标；规则按列表顺序覆盖。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrushReplacement {
    pub sources: Vec<State>,
    pub target: State,
}
impl BrushReplacement {
    /// 大范围替换编译一次实际状态表，不对每个体素重新遍历所有规则。
    pub fn table(rules: &[Self]) -> std::collections::HashMap<&State, &State> {
        let mut result = std::collections::HashMap::new();
        for rule in rules {
            for source in &rule.sources {
                result.insert(source, &rule.target);
            }
        }
        result
    }
    /// 与原版替换表一致：后列规则覆盖同一完整源状态，不连锁套用输出。
    pub fn target<'a>(rules: &'a [Self], before: &State) -> Option<&'a State> {
        rules
            .iter()
            .rev()
            .find(|rule| rule.sources.contains(before))
            .map(|rule| &rule.target)
    }
}

/// 一次笔画内按修改前的有效状态映射，不将替换结果再次交给后续规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrushPaint {
    Fill(State),
    Replace(Vec<BrushReplacement>),
}
impl BrushPaint {
    pub(super) fn validate(&self) -> Result<(), WorldError> {
        if let Self::Replace(rules) = self {
            if rules.is_empty() || rules.iter().any(|r| r.sources.is_empty()) {
                return Err(refuse("替换笔刷需要至少一条含源状态的规则"));
            }
        }
        Ok(())
    }
}
pub(super) enum CellPaint<'a> {
    Uniform(&'a State, Option<&'a State>),
    Brush(&'a BrushPaint),
}
impl CellPaint<'_> {
    pub fn validate_layout(&self, numeric: bool) -> Result<(), WorldError> {
        let check = |state: &State| {
            if state.name.starts_with("numeric:") == numeric {
                Ok(())
            } else {
                Err(refuse("笔刷目标或源状态的数字 / 命名布局与实际区块不同"))
            }
        };
        match self {
            Self::Uniform(target, filter) => {
                check(target)?;
                if let Some(filter) = filter {
                    check(filter)?;
                }
            }
            Self::Brush(BrushPaint::Fill(target)) => check(target)?,
            Self::Brush(BrushPaint::Replace(rules)) => {
                for rule in rules {
                    check(&rule.target)?;
                    for source in &rule.sources {
                        check(source)?;
                    }
                }
            }
        }
        Ok(())
    }
    pub fn target(&self, before: &State) -> Option<&State> {
        match self {
            Self::Uniform(target, filter) => filter
                .is_none_or(|filter| filter == before)
                .then_some(*target),
            Self::Brush(BrushPaint::Fill(target)) => Some(target),
            Self::Brush(BrushPaint::Replace(rules)) => BrushReplacement::target(rules, before),
        }
    }
}

/// 输入阶段只记录线段；整数补点在后台迭代，形状并集负责重叠去重。
#[derive(Debug, Clone, Default)]
pub struct BrushPath {
    segments: Vec<([i32; 3], [i32; 3], bool)>,
    last: Option<[i32; 3]>,
}
impl BrushPath {
    pub fn points(&self) -> impl Iterator<Item = [i32; 3]> + '_ {
        self.segments
            .iter()
            .flat_map(|&(start, end, include_start)| {
                let delta: [i64; 3] =
                    std::array::from_fn(|i| i64::from(end[i]) - i64::from(start[i]));
                let distance = delta.map(i64::abs);
                let length = *distance.iter().max().expect("三条坐标轴");
                (i64::from(!include_start)..=length).map(move |step| {
                    if length == 0 {
                        end
                    } else {
                        std::array::from_fn(|i| {
                            (i64::from(start[i])
                                + delta[i].signum() * ((distance[i] * step + length / 2) / length))
                                as i32
                        })
                    }
                })
            })
    }
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
    pub fn break_segment(&mut self) {
        self.last = None;
    }
    pub fn push(&mut self, end: [i32; 3]) -> Result<(), WorldError> {
        self.push_with_budget(end, EditWorkBudget::default())
    }
    pub fn push_with_budget(
        &mut self,
        end: [i32; 3],
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        Bounds::spatial((end[0], end[1], end[2]), (end[0], end[1], end[2]))?;
        self.append(end, budget)
    }
    /// 独立结构使用 u16 局部尺寸；允许一笔的几何边缘伸出文档，由所属文档裁取实际格。
    pub fn push_local_with_budget(
        &mut self,
        end: [i32; 3],
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        local_bounds(Bounds {
            minimum: end,
            maximum: end,
        })?;
        self.append(end, budget)
    }
    fn append(&mut self, end: [i32; 3], budget: EditWorkBudget) -> Result<(), WorldError> {
        if self.last == Some(end) {
            return Ok(());
        }
        budget.check(
            (self.segments.len() + 1)
                .checked_mul(std::mem::size_of::<([i32; 3], [i32; 3], bool)>() * 4)
                .ok_or_else(|| refuse("笔画输入轨迹大小溢出"))?,
        )?;
        self.segments
            .push((self.last.unwrap_or(end), end, self.last.is_none()));
        self.last = Some(end);
        Ok(())
    }
}

/// 与原版一致：奇数尺寸围绕中心，偶数尺寸的最小坐标为中心减尺寸的一半。
pub fn brush_bounds(center: [i32; 3], size: [u16; 3]) -> Result<Bounds, WorldError> {
    let bounds = brush_geometry(center, size)?;
    Bounds::spatial(bounds.minimum.into(), bounds.maximum.into())
}
/// 结构笔触仍使用同一居中规则，不借用世界 Y 合同。
pub fn brush_bounds_local(center: [i32; 3], size: [u16; 3]) -> Result<Bounds, WorldError> {
    let bounds = brush_geometry(center, size)?;
    local_bounds(bounds)
}
pub(super) fn local_bounds(bounds: Bounds) -> Result<Bounds, WorldError> {
    if (0..3).any(|axis| {
        bounds.minimum[axis] < -512
            || bounds.maximum[axis] > i32::from(u16::MAX) + 512
            || bounds.maximum[axis] < bounds.minimum[axis]
    }) {
        return Err(refuse("笔触超出结构局部坐标与最大笔刷边缘范围"));
    }
    bounds.volume()?;
    Ok(bounds)
}
fn brush_geometry(center: [i32; 3], size: [u16; 3]) -> Result<Bounds, WorldError> {
    if size.contains(&0) || size.iter().any(|n| *n > 512) {
        return Err(refuse("笔刷各轴尺寸必须为 1..512"));
    }
    let mut minimum = [0; 3];
    let mut maximum = [0; 3];
    for i in 0..3 {
        let min = i64::from(center[i]) - i64::from(size[i] / 2);
        minimum[i] = i32::try_from(min).map_err(|_| refuse("笔刷坐标溢出"))?;
        maximum[i] =
            i32::try_from(min + i64::from(size[i]) - 1).map_err(|_| refuse("笔刷坐标溢出"))?;
    }
    Ok(Bounds { minimum, maximum })
}

pub(super) fn cells(
    strokes: &[Bounds],
    mask: BrushMask,
    mut cancelled: impl FnMut() -> bool,
) -> Result<BTreeSet<Position>, WorldError> {
    let budget = EditWorkBudget::default();
    let selection = mask.select(
        strokes.iter().copied().map(Ok),
        budget,
        &mut cancelled,
        |_| {},
    )?;
    budget.check(
        selection
            .bytes()
            .checked_add(
                selection
                    .len()
                    .checked_mul(64)
                    .ok_or_else(|| refuse("逐格选区大小溢出"))?,
            )
            .ok_or_else(|| refuse("逐格选区大小溢出"))?,
    )?;
    Ok(selection.iter().collect())
}

impl Session {
    /// 所有笔触先取并集，每个真实源区块只解码一次，再生成一份普通撤销事务。
    pub fn prepare_brush_stroke_cancellable(
        &self,
        strokes: &[Bounds],
        mask: BrushMask,
        paint: &BrushPaint,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Preview, WorldError> {
        paint.validate()?;
        let budget = EditWorkBudget::default();
        let selection = mask.select(
            strokes.iter().copied().map(Ok),
            budget,
            &mut cancelled,
            |_| {},
        )?;
        self.prepare_brush_selection_cancellable(selection, paint, budget, cancelled, |_| {})
    }

    /// 复核画面实际命中的源方块，允许已有暂存但不把插值点伪装成射线命中。
    pub fn verify_brush_sources(&self, witnesses: &[([i32; 3], State)]) -> Result<(), WorldError> {
        self.verify_brush_sources_cancellable(witnesses, EditWorkBudget::default(), || false)
    }
    pub fn verify_brush_sources_cancellable(
        &self,
        witnesses: &[([i32; 3], State)],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        let mut grouped: BTreeMap<ChunkPos, Vec<&([i32; 3], State)>> = BTreeMap::new();
        let retained = witnesses.iter().try_fold(0usize, |bytes, (_, state)| {
            bytes
                .checked_add((std::mem::size_of::<[i32; 3]>() + compound::state_bytes(state)) * 4)
                .ok_or_else(|| refuse("笔画源采样工作集溢出"))
        })?;
        budget.check(retained)?;
        for witness in witnesses {
            let p = witness.0;
            grouped
                .entry((p[0].div_euclid(16), p[2].div_euclid(16)))
                .or_default()
                .push(witness);
        }
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        for (key, witnesses) in grouped {
            if cancelled() {
                return Err(refuse("笔画源复核已取消"));
            }
            let (chunk, _, _) =
                super::batch_paint::load_sources(self, &mut reader, key, budget, retained)?;
            for (p, expected) in witnesses {
                let original = chunk
                    .block(p[0].rem_euclid(16) as u8, p[1], p[2].rem_euclid(16) as u8)
                    .ok_or_else(|| refuse("笔画命中的源区段已消失"))?;
                if self.effective_state((p[0], p[1], p[2]), original) != *expected {
                    return Err(refuse(format!(
                        "笔画源方块 {p:?} 与画面不同，请等待重新建模"
                    )));
                }
            }
        }
        Ok(())
    }
}
