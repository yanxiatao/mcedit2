//! 按区段保存的笔画并集；同一位只保留一次，不按笔触累计体积预分配逐格树。
use super::*;

/// 一次编辑任务的新增工作集预算；不代表整个会话、历史或进程的内存上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditWorkBudget {
    bytes: usize,
}
impl Default for EditWorkBudget {
    fn default() -> Self {
        Self {
            bytes: 256 * 1024 * 1024,
        }
    }
}
impl EditWorkBudget {
    pub fn from_bytes(bytes: usize) -> Result<Self, WorldError> {
        if bytes == 0 || bytes > isize::MAX as usize {
            return Err(refuse("编辑任务预算超出地址范围"));
        }
        Ok(Self { bytes })
    }
    pub fn from_mib(text: &str) -> Result<Self, WorldError> {
        let mib = text
            .trim()
            .parse::<usize>()
            .map_err(|_| refuse("编辑任务预算必须为正整数 MiB"))?;
        let bytes = mib
            .checked_mul(1024 * 1024)
            .filter(|n| *n > 0 && *n <= isize::MAX as usize)
            .ok_or_else(|| refuse("编辑任务预算超出地址范围"))?;
        Ok(Self { bytes })
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
    pub fn check(self, bytes: usize) -> Result<(), WorldError> {
        if bytes > self.bytes {
            Err(refuse(format!(
                "编辑任务工作集预计至少需要 {} MiB，超过 {} MiB 预算",
                bytes.div_ceil(1024 * 1024),
                self.bytes / (1024 * 1024)
            )))
        } else {
            Ok(())
        }
    }
}
/// 一个进度点描述已经完成的工作，不把待处理格或预计结果记为成功修改。
#[derive(Debug, Clone, Copy, Default)]
pub struct EditWorkProgress {
    pub stage: &'static str,
    pub examined: usize,
    pub selected: usize,
    pub changed: usize,
    pub slices: usize,
}
type SectionKey = (i32, i32, i32); // cx, cz, section_y，保证一次处理同一源区块。
const SECTION_BYTES: usize = 512 + std::mem::size_of::<SectionKey>() + 96;
#[derive(Debug, Clone, Default)]
pub struct BrushSelection {
    sections: BTreeMap<SectionKey, Box<[u64; 64]>>,
    count: usize,
}
impl BrushSelection {
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn bytes(&self) -> usize {
        self.sections.len() * SECTION_BYTES
    }
    pub(super) fn section_keys(&self) -> impl Iterator<Item = SectionKey> + '_ {
        self.sections.keys().copied()
    }
    pub(super) fn has_section(&self, key: SectionKey) -> bool {
        self.sections.contains_key(&key)
    }
    pub fn contains(&self, p: Position) -> bool {
        let key = (p.0.div_euclid(16), p.2.div_euclid(16), p.1.div_euclid(16));
        let bit =
            (p.1.rem_euclid(16) * 256 + p.2.rem_euclid(16) * 16 + p.0.rem_euclid(16)) as usize;
        self.sections
            .get(&key)
            .is_some_and(|words| words[bit / 64] & (1_u64 << (bit % 64)) != 0)
    }
    pub(crate) fn visit(
        &mut self,
        p: Position,
        budget: EditWorkBudget,
    ) -> Result<bool, WorldError> {
        let key = (p.0.div_euclid(16), p.2.div_euclid(16), p.1.div_euclid(16));
        let bit =
            (p.1.rem_euclid(16) * 256 + p.2.rem_euclid(16) * 16 + p.0.rem_euclid(16)) as usize;
        if !self.sections.contains_key(&key) {
            budget.check(
                self.bytes()
                    .checked_add(SECTION_BYTES)
                    .ok_or_else(|| refuse("连通选区大小溢出"))?,
            )?;
        }
        let words = self
            .sections
            .entry(key)
            .or_insert_with(|| Box::new([0; 64]));
        let new = words[bit / 64] & (1_u64 << (bit % 64)) == 0;
        words[bit / 64] |= 1_u64 << (bit % 64);
        self.count += usize::from(new);
        Ok(new)
    }
    pub fn iter(&self) -> impl Iterator<Item = Position> + '_ {
        self.sections.iter().flat_map(|(&(cx, cz, sy), words)| {
            words.iter().enumerate().flat_map(move |(i, &word)| {
                let mut rest = word;
                std::iter::from_fn(move || {
                    if rest == 0 {
                        return None;
                    }
                    let bit = i * 64 + rest.trailing_zeros() as usize;
                    rest &= rest - 1;
                    Some((
                        cx * 16 + (bit % 16) as i32,
                        sy * 16 + (bit / 256) as i32,
                        cz * 16 + ((bit / 16) % 16) as i32,
                    ))
                })
            })
        })
    }
    fn add(
        &mut self,
        bounds: Bounds,
        shape: Option<BrushShape>,
        budget: EditWorkBudget,
        cancelled: &mut impl FnMut() -> bool,
        progress: &mut impl FnMut(EditWorkProgress),
        slices: &mut usize,
    ) -> Result<(), WorldError> {
        let [x0, y0, z0] = bounds.minimum;
        let [x1, y1, z1] = bounds.maximum;
        for cx in x0.div_euclid(16)..=x1.div_euclid(16) {
            for cz in z0.div_euclid(16)..=z1.div_euclid(16) {
                for sy in y0.div_euclid(16)..=y1.div_euclid(16) {
                    if cancelled() {
                        return Err(refuse("笔画选区已取消"));
                    }
                    let key = (cx, cz, sy);
                    if self
                        .sections
                        .get(&key)
                        .is_some_and(|words| words.iter().all(|word| *word == u64::MAX))
                    {
                        continue;
                    }
                    let mut selected = [0_u64; 64];
                    for y in y0.max(sy * 16)..=y1.min(sy * 16 + 15) {
                        for z in z0.max(cz * 16)..=z1.min(cz * 16 + 15) {
                            if shape.is_none() {
                                let first = x0.max(cx * 16).rem_euclid(16) as usize;
                                let last = x1.min(cx * 16 + 15).rem_euclid(16) as usize;
                                let bit = (y.rem_euclid(16) * 256 + z.rem_euclid(16) * 16) as usize
                                    + first;
                                selected[bit / 64] |=
                                    ((1_u64 << (last - first + 1)) - 1) << (bit % 64);
                            } else {
                                for x in x0.max(cx * 16)..=x1.min(cx * 16 + 15) {
                                    if brush_inside(
                                        shape.expect("已分支检查形状"),
                                        bounds,
                                        (x, y, z),
                                    ) {
                                        let bit = (y.rem_euclid(16) * 256
                                            + z.rem_euclid(16) * 16
                                            + x.rem_euclid(16))
                                            as usize;
                                        selected[bit / 64] |= 1_u64 << (bit % 64);
                                    }
                                }
                            }
                        }
                    }
                    if selected.iter().any(|word| *word != 0) {
                        if !self.sections.contains_key(&key) {
                            budget.check(
                                self.bytes()
                                    .checked_add(SECTION_BYTES)
                                    .ok_or_else(|| refuse("笔画掩码大小溢出"))?,
                            )?;
                        }
                        let entry = self
                            .sections
                            .entry(key)
                            .or_insert_with(|| Box::new([0; 64]));
                        for (word, new) in entry.iter_mut().zip(selected) {
                            self.count += (new & !*word).count_ones() as usize;
                            *word |= new;
                        }
                    }
                    *slices += 1;
                    progress(EditWorkProgress {
                        stage: "构建笔画并集",
                        selected: self.count,
                        slices: *slices,
                        ..Default::default()
                    });
                }
            }
        }
        Ok(())
    }
    fn shell(
        self,
        budget: EditWorkBudget,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<Self, WorldError> {
        let mut result = Self::default();
        for (&key, words) in &self.sections {
            if cancelled() {
                return Err(refuse("空心笔画已取消"));
            }
            if words.iter().all(|word| *word == u64::MAX)
                && [
                    (key.0 - 1, key.1, key.2),
                    (key.0 + 1, key.1, key.2),
                    (key.0, key.1 - 1, key.2),
                    (key.0, key.1 + 1, key.2),
                    (key.0, key.1, key.2 - 1),
                    (key.0, key.1, key.2 + 1),
                ]
                .into_iter()
                .all(|neighbor| {
                    self.sections
                        .get(&neighbor)
                        .is_some_and(|words| words.iter().all(|word| *word == u64::MAX))
                })
            {
                continue;
            }
            let mut shell = [0_u64; 64];
            for (i, &word) in words.iter().enumerate() {
                let mut bits = word;
                while bits != 0 {
                    let bit = i * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let (x, y, z) = (
                        key.0 * 16 + (bit % 16) as i32,
                        key.2 * 16 + (bit / 256) as i32,
                        key.1 * 16 + ((bit / 16) % 16) as i32,
                    );
                    if [
                        (x - 1, y, z),
                        (x + 1, y, z),
                        (x, y - 1, z),
                        (x, y + 1, z),
                        (x, y, z - 1),
                        (x, y, z + 1),
                    ]
                    .into_iter()
                    .any(|p| !self.contains(p))
                    {
                        shell[i] |= 1_u64 << (bit % 64);
                    }
                }
            }
            if shell.iter().any(|word| *word != 0) {
                budget.check(
                    self.bytes()
                        .checked_add(result.bytes() + SECTION_BYTES)
                        .ok_or_else(|| refuse("空心并集工作集溢出"))?,
                )?;
                result.count += shell
                    .iter()
                    .map(|word| word.count_ones() as usize)
                    .sum::<usize>();
                result.sections.insert(key, Box::new(shell));
            }
        }
        Ok(result)
    }
}
impl BrushMask {
    pub fn select(
        self,
        strokes: impl IntoIterator<Item = Result<Bounds, WorldError>>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<BrushSelection, WorldError> {
        self.select_validated(
            strokes.into_iter().map(|bounds| {
                let bounds = bounds?;
                if Bounds::spatial(bounds.minimum.into(), bounds.maximum.into())? != bounds {
                    return Err(refuse("笔触范围无效"));
                }
                Ok(bounds)
            }),
            budget,
            &mut cancelled,
            &mut progress,
        )
    }
    /// 世界和局部文档共用并集与空心算法，坐标合同由各入口核实。
    pub fn select_local(
        self,
        strokes: impl IntoIterator<Item = Result<Bounds, WorldError>>,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<BrushSelection, WorldError> {
        self.select_validated(
            strokes
                .into_iter()
                .map(|bounds| super::brush_stroke::local_bounds(bounds?)),
            budget,
            cancelled,
            progress,
        )
    }
    fn select_validated(
        self,
        strokes: impl IntoIterator<Item = Result<Bounds, WorldError>>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<BrushSelection, WorldError> {
        let mut selection = BrushSelection::default();
        let mut slices = 0;
        for bounds in strokes {
            selection.add(
                bounds?,
                self.shape,
                budget,
                &mut cancelled,
                &mut progress,
                &mut slices,
            )?;
        }
        if self.hollow {
            selection = selection.shell(budget, &mut cancelled)?;
        }
        Ok(selection)
    }
}
