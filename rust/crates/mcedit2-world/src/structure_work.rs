//! 密集结构的分区块采样与已解码工作集计量。
use super::*;
pub(super) fn positions(bounds: Bounds) -> impl Iterator<Item = Position> {
    let [x0, y0, z0] = bounds.minimum;
    let [x1, y1, z1] = bounds.maximum;
    (x0.div_euclid(16)..=x1.div_euclid(16)).flat_map(move |cx| {
        (z0.div_euclid(16)..=z1.div_euclid(16)).flat_map(move |cz| {
            (y0..=y1).flat_map(move |y| {
                (z0.max(cz * 16)..=z1.min(cz * 16 + 15)).flat_map(move |z| {
                    (x0.max(cx * 16)..=x1.min(cx * 16 + 15)).map(move |x| (x, y, z))
                })
            })
        })
    })
}
pub(super) fn index(bounds: Bounds, (x, y, z): Position) -> usize {
    let w = (bounds.maximum[0] - bounds.minimum[0] + 1) as usize;
    let d = (bounds.maximum[2] - bounds.minimum[2] + 1) as usize;
    ((y - bounds.minimum[1]) as usize * d + (z - bounds.minimum[2]) as usize) * w
        + (x - bounds.minimum[0]) as usize
}
pub(super) fn chunk_bytes(chunk: &Chunk) -> usize {
    std::mem::size_of::<Chunk>()
        + chunk.block_entities.len() * 64
        + chunk.light.allocated_bytes()
        + chunk
            .numeric_biomes
            .as_ref()
            .map_or(0, |b| b.allocated_bytes())
        + chunk
            .sections
            .iter()
            .map(|section| {
                std::mem::size_of::<crate::sections::Section>()
                    + section.indices.len() * 2
                    + section
                        .palette
                        .iter()
                        .map(clipboard::state_bytes)
                        .sum::<usize>()
                    + section.biomes.as_ref().map_or(0, |b| {
                        b.indices.len() * 2
                            + b.names
                                .iter()
                                .map(|s| std::mem::size_of::<String>() + s.len())
                                .sum::<usize>()
                    })
            })
            .sum::<usize>()
}
pub(super) fn check_work(budget: EditWorkBudget, bytes: usize) -> Result<(), WorldError> {
    budget.check(
        bytes
            .checked_mul(4)
            .ok_or_else(|| refuse("结构工作集溢出"))?,
    )
}
impl PreparedStructure {
    pub(super) fn check_work(&self, budget: EditWorkBudget) -> Result<(), WorldError> {
        structure_work::check_work(budget, compound::structure_bytes(self)?)
    }
}
impl Session {
    /// 方块专用剪贴板仍拒绝附加载荷；分区块读取与完整结构使用相同预算合同。
    pub fn prepare_paste_clipboard_with_budget_cancellable(
        &self,
        clip: &BlockClipboard,
        destination: Position,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Preview, WorldError> {
        clip.validate_with_budget(&self.world, &self.dimension, budget)?;
        if clip.has_non_block_payload() {
            return Err(refuse(
                "方块专用粘贴不能丢弃结构附加载荷，请使用完整结构入口",
            ));
        }
        let end = [destination.0, destination.1, destination.2];
        let max: Vec<i32> = end
            .iter()
            .zip(clip.size)
            .map(|(&p, n)| {
                p.checked_add(i32::from(n) - 1)
                    .ok_or_else(|| refuse("剪贴板目标坐标溢出"))
            })
            .collect::<Result<_, _>>()?;
        let bounds = Bounds::spatial(destination, (max[0], max[1], max[2]))?;
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut current = None;
        let mut source = None;
        let mut changes = Vec::new();
        let mut chunks = 0;
        let mut examined = 0;
        let mut retained = clip.work_bytes()?;
        for (visited, position) in positions(bounds).enumerate() {
            if visited.is_multiple_of(4096) && cancelled() {
                return Err(refuse("剪贴板目标准备已取消"));
            }
            if clip
                .mask
                .as_ref()
                .is_some_and(|mask| !mask[index(bounds, position)])
            {
                continue;
            }
            if examined % 4096 == 0 && cancelled() {
                return Err(refuse("剪贴板目标准备已取消"));
            }
            let (x, y, z) = position;
            let key = (x.div_euclid(16), z.div_euclid(16));
            if current != Some(key) {
                drop(source.take());
                source = Some(batch_paint::load_sources(
                    self,
                    &mut reader,
                    key,
                    budget,
                    retained * 4,
                )?);
                current = Some(key);
                chunks += 1;
            }
            let (chunk, be, scratch) = source.as_ref().expect("当前源区块已读取");
            if clip.numeric != matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112)
            {
                return Err(refuse("剪贴板与目标的数字 / 命名布局不同"));
            }
            if be.as_ref().unwrap_or(chunk).has_block_entity(x, y, z) {
                return Err(refuse("方块专用剪贴板不能覆盖目标方块实体"));
            }
            let original = chunk
                .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| refuse("剪贴板目标区段不存在"))?;
            let before = self.overlay.get(&position).unwrap_or(original);
            let after = &clip.blocks[index(bounds, position)];
            examined += 1;
            if before != after {
                let change = Change {
                    position,
                    original: original.clone(),
                    before: before.clone(),
                    after: after.clone(),
                };
                retained = retained
                    .checked_add(compound::change_bytes(std::slice::from_ref(&change)))
                    .ok_or_else(|| refuse("剪贴板候选工作集溢出"))?;
                check_work(budget, retained + scratch)?;
                changes.push(change);
            }
        }
        if cancelled() {
            return Err(refuse("剪贴板候选发布已取消"));
        }
        Ok(Preview {
            work_budget: budget,
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            data_version: self.world.data_version,
            changes,
            source_checks: Vec::new(),
            is_copy: true,
            is_move: false,
            is_fill: false,
            examined,
            matched: examined,
            skipped: 0,
            source_chunks: chunks,
        })
    }
}
