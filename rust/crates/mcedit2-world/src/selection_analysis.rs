//! 原版 Analyze 的流式统计。只统计真实格子和根实体，不合成缺失区块或稀疏格。
use super::*;
use crate::sections::{parse_root, Tag};
use std::{collections::HashMap, io::Write};

#[derive(Debug, Clone)]
pub struct BlockCount {
    pub state: State,
    pub count: usize,
}
impl BlockCount {
    /// 旧格式保留原始数字 ID 与元数据，不通过现代注册表猜名称。
    pub fn numeric_identity(&self) -> Option<(u16, u8)> {
        let id = self.state.name.strip_prefix("numeric:")?.parse().ok()?;
        let data = self
            .state
            .properties
            .iter()
            .find(|(key, _)| key == "data")?
            .1
            .parse()
            .ok()?;
        Some((id, data))
    }
}
#[derive(Debug, Clone)]
pub struct ObjectCount {
    pub id: String,
    pub count: usize,
}
/// 一次完整扫描的不可变结果；取消或源损坏不会返回部分成功。
#[derive(Debug, Clone)]
pub struct Report {
    pub source: String,
    pub region: SelectionRegion,
    pub selected: usize,
    pub counted: usize,
    pub missing_chunk_cells: usize,
    pub missing_section_cells: usize,
    pub undeclared_cells: usize,
    pub chunks: usize,
    pub present_chunks: usize,
    pub blocks: Vec<BlockCount>,
    pub entities: Vec<ObjectCount>,
    pub block_entities: Vec<ObjectCount>,
}
impl Report {
    pub fn work_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.source.len()
            + self
                .blocks
                .iter()
                .map(|row| clipboard::state_bytes(&row.state) + std::mem::size_of::<usize>())
                .sum::<usize>()
            + self
                .entities
                .iter()
                .chain(&self.block_entities)
                .map(|row| std::mem::size_of::<ObjectCount>() + row.id.len())
                .sum::<usize>()
    }
    pub fn entity_count(&self) -> usize {
        self.entities.iter().map(|row| row.count).sum()
    }
    pub fn block_entity_count(&self) -> usize {
        self.block_entities.iter().map(|row| row.count).sum()
    }
    /// 写出全部结果，与界面筛选/分页无关；名称只能来自调用方实际加载的资源。
    pub fn write_delimited(
        &self,
        mut output: impl Write,
        separator: char,
        mut name: impl FnMut(&State) -> Option<String>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        if !matches!(separator, '\t' | ',' | ';') {
            return Err(refuse("分析导出分隔符无效"));
        }
        let mut row = |fields: &[String]| -> Result<(), WorldError> {
            stop(&mut cancelled)?;
            for (index, field) in fields.iter().enumerate() {
                if index != 0 {
                    write!(output, "{separator}")?;
                }
                if field.contains([separator, '"', '\r', '\n']) {
                    write!(output, "\"{}\"", field.replace('"', "\"\""))?;
                } else {
                    write!(output, "{field}")?;
                }
            }
            writeln!(output)?;
            Ok(())
        };
        row(&["Source".into(), self.source.clone()])?;
        row(&[
            "Selection".into(),
            format!(
                "{:?};{:?}",
                self.region.bounds.minimum, self.region.bounds.maximum
            ),
            self.region.shape.to_string(),
        ])?;
        for (label, count) in [
            ("Selected cells", self.selected),
            ("Counted blocks", self.counted),
            ("Missing chunk cells", self.missing_chunk_cells),
            ("Missing section cells", self.missing_section_cells),
            ("Undeclared structure cells", self.undeclared_cells),
            ("Visited chunks", self.chunks),
            ("Present chunks", self.present_chunks),
            ("Root entities", self.entity_count()),
            ("Block entities", self.block_entity_count()),
        ] {
            row(&[label.into(), count.to_string()])?;
        }
        row(&[])?;
        row(&["Blocks:".into()])?;
        row(&[
            "Name".into(),
            "Id".into(),
            "Data".into(),
            "State".into(),
            "Count".into(),
        ])?;
        for entry in &self.blocks {
            let numeric = entry.numeric_identity();
            row(&[
                name(&entry.state).unwrap_or_else(|| entry.state.name.clone()),
                numeric.map_or_else(|| entry.state.name.clone(), |(id, _)| id.to_string()),
                numeric.map_or_else(String::new, |(_, data)| data.to_string()),
                render_state(&entry.state),
                entry.count.to_string(),
            ])?;
        }
        for (title, entries) in [
            ("Entities:", &self.entities),
            ("Block entities:", &self.block_entities),
        ] {
            row(&[])?;
            row(&[title.into()])?;
            row(&["Name".into(), "Count".into()])?;
            for entry in entries {
                row(&[entry.id.clone(), entry.count.to_string()])?;
            }
        }
        Ok(())
    }
}
struct Counter {
    report: Report,
    blocks: HashMap<State, usize>,
    entities: BTreeMap<String, usize>,
    block_entities: BTreeMap<String, usize>,
    retained: usize,
    scratch: usize,
    budget: EditWorkBudget,
}
impl Counter {
    fn new(
        source: String,
        region: SelectionRegion,
        budget: EditWorkBudget,
    ) -> Result<Self, WorldError> {
        region.bounds.volume()?;
        Ok(Self {
            report: Report {
                source,
                region,
                selected: 0,
                counted: 0,
                missing_chunk_cells: 0,
                missing_section_cells: 0,
                undeclared_cells: 0,
                chunks: 0,
                present_chunks: 0,
                blocks: Vec::new(),
                entities: Vec::new(),
                block_entities: Vec::new(),
            },
            blocks: HashMap::new(),
            entities: BTreeMap::new(),
            block_entities: BTreeMap::new(),
            retained: 0,
            scratch: 0,
            budget,
        })
    }
    fn block(&mut self, state: &State) -> Result<(), WorldError> {
        if let Some(count) = self.blocks.get_mut(state) {
            *count += 1;
        } else {
            self.retained += clipboard::state_bytes(state) + 96;
            self.budget
                .check(self.retained.saturating_mul(2).saturating_add(self.scratch))?;
            self.blocks.insert(state.clone(), 1);
        }
        self.report.counted += 1;
        Ok(())
    }
    fn object(&mut self, id: String, block: bool) -> Result<(), WorldError> {
        let entries = if block {
            &mut self.block_entities
        } else {
            &mut self.entities
        };
        if !entries.contains_key(&id) {
            self.retained += id.len() + 96;
            self.budget
                .check(self.retained.saturating_mul(2).saturating_add(self.scratch))?;
        }
        *entries.entry(id).or_default() += 1;
        Ok(())
    }
    fn finish(mut self) -> Report {
        self.report.blocks = self
            .blocks
            .into_iter()
            .map(|(state, count)| BlockCount { state, count })
            .collect();
        self.report.blocks.sort_unstable_by(|a, b| {
            a.state
                .name
                .cmp(&b.state.name)
                .then_with(|| a.state.properties.cmp(&b.state.properties))
        });
        self.report.entities = self
            .entities
            .into_iter()
            .map(|(id, count)| ObjectCount { id, count })
            .collect();
        self.report.block_entities = self
            .block_entities
            .into_iter()
            .map(|(id, count)| ObjectCount { id, count })
            .collect();
        self.report
    }
    fn progress(&self, stage: &'static str, examined: usize, slices: usize) -> EditWorkProgress {
        EditWorkProgress {
            stage,
            examined,
            selected: self.report.selected,
            changed: self.report.counted,
            slices,
        }
    }
    fn raw(
        &mut self,
        raw: &[u8],
        block: bool,
        entity_slot: Option<ChunkPos>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        let prior_scratch = self.scratch;
        self.scratch = self.scratch.saturating_add(raw.len().saturating_mul(8));
        self.budget
            .check(self.retained.saturating_mul(2).saturating_add(self.scratch))?;
        let root = parse_root(raw).map_err(|e| refuse(e.to_string()))?;
        if let Some((cx, cz)) = entity_slot {
            let position = root
                .require("Position", 11)
                .map_err(|e| refuse(e.to_string()))?;
            if position.raw.len() != 12
                || i32::from_be_bytes(position.raw[..4].try_into().expect("已核实数组长度")) != 2
                || i32::from_be_bytes(position.raw[4..8].try_into().expect("已核实数组长度")) != cx
                || i32::from_be_bytes(position.raw[8..12].try_into().expect("已核实数组长度")) != cz
            {
                return Err(refuse("分析来源实体区块的 Position 与 Region 槽不一致"));
            }
        }
        let container = root.field("Level").unwrap_or(&root);
        let list = if block {
            container
                .field("TileEntities")
                .or_else(|| container.field("block_entities"))
        } else {
            container.field("Entities")
        };
        let Some(list) = list else {
            self.scratch = prior_scratch;
            return Ok(());
        };
        if list.kind != 9 || list.items.iter().any(|tag| tag.kind != 10) {
            return Err(refuse("分析来源对象列表不是 Compound 列表"));
        }
        for tag in &list.items {
            stop(&mut cancelled)?;
            let inside = if block {
                let xyz = |name| {
                    tag.require(name, 3)
                        .and_then(Tag::i32)
                        .map_err(|e| refuse(e.to_string()))
                };
                self.report
                    .region
                    .contains((xyz("x")?, xyz("y")?, xyz("z")?))
            } else {
                self.report.region.contains_point(entity_position(tag)?)
            };
            if inside {
                self.object(object_id(tag)?, block)?;
            }
        }
        self.scratch = prior_scratch;
        Ok(())
    }
}
fn stop(cancelled: &mut impl FnMut() -> bool) -> Result<(), WorldError> {
    if cancelled() {
        Err(refuse("选区分析已取消"))
    } else {
        Ok(())
    }
}
fn object_id(tag: &Tag<'_>) -> Result<String, WorldError> {
    tag.require("id", 8)
        .and_then(Tag::string)
        .map_err(|e| refuse(e.to_string()))
}
fn entity_position(tag: &Tag<'_>) -> Result<[f64; 3], WorldError> {
    let pos = tag.require("Pos", 9).map_err(|e| refuse(e.to_string()))?;
    if pos.items.len() != 3
        || pos
            .items
            .iter()
            .any(|tag| tag.kind != 6 || tag.raw.len() != 8)
    {
        return Err(refuse("分析来源实体 Pos 不是三项 Double 列表"));
    }
    let position = std::array::from_fn(|i| {
        f64::from_be_bytes(pos.items[i].raw.try_into().expect("已核实 Double 长度"))
    });
    if !position.into_iter().all(f64::is_finite) {
        return Err(refuse("分析来源实体坐标不是有限数"));
    }
    Ok(position)
}
impl Session {
    /// 逐 Region/区块扫描当前暂存内容，不复制整个选区，也不忽略缺失覆盖。
    pub fn analyze_selection(
        &self,
        region: SelectionRegion,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Report, WorldError> {
        let region = region.resolved(&self.world, &self.dimension)?;
        let mut count = Counter::new(
            format!("{} · {}", self.world.root.display(), self.dimension),
            region,
            budget,
        )?;
        let lo = region.bounds.minimum;
        let hi = region.bounds.maximum;
        let mut examined = 0usize;
        for rz in lo[2].div_euclid(512)..=hi[2].div_euclid(512) {
            for rx in lo[0].div_euclid(512)..=hi[0].div_euclid(512) {
                let mut reader = ReadOnlyRegionCache::for_source(
                    &self.world,
                    &self.dimension,
                    self.origin.as_ref(),
                )?;
                for cz in lo[2].div_euclid(16).max(rz * 32)..=hi[2].div_euclid(16).min(rz * 32 + 31)
                {
                    for cx in
                        lo[0].div_euclid(16).max(rx * 32)..=hi[0].div_euclid(16).min(rx * 32 + 31)
                    {
                        stop(&mut cancelled)?;
                        let (raw, _) = self.visible_chunk_payload_cached(&mut reader, cx, cz)?;
                        count.scratch = reader.frozen_bytes().saturating_add(
                            raw.as_ref().map_or(0, |raw| raw.len().saturating_mul(8)),
                        );
                        budget.check(
                            count
                                .retained
                                .saturating_mul(2)
                                .saturating_add(count.scratch),
                        )?;
                        let chunk = crate::block_lookup::checked_original_chunk_bytes(
                            raw.as_deref(),
                            cx,
                            cz,
                        )?;
                        count.report.chunks += 1;
                        count.report.present_chunks += usize::from(chunk.is_some());
                        for y in lo[1]..=hi[1] {
                            for z in lo[2].max(cz * 16)..=hi[2].min(cz * 16 + 15) {
                                for x in lo[0].max(cx * 16)..=hi[0].min(cx * 16 + 15) {
                                    if examined.is_multiple_of(4096) {
                                        stop(&mut cancelled)?;
                                        progress(count.progress(
                                            "分析选区",
                                            examined,
                                            count.report.chunks,
                                        ));
                                    }
                                    examined += 1;
                                    let p = (x, y, z);
                                    if !region.contains(p) {
                                        continue;
                                    }
                                    count.report.selected += 1;
                                    let original = chunk.as_ref().and_then(|c| {
                                        c.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                                    });
                                    let state = self.display_change(p).or(original);
                                    if let Some(state) = state {
                                        count.block(state)?;
                                    } else if chunk.is_some() {
                                        count.report.missing_section_cells += 1;
                                    } else {
                                        count.report.missing_chunk_cells += 1;
                                    }
                                }
                            }
                        }
                        let staged = self.staged_be_chunk_cached(&mut reader, cx, cz)?;
                        if let Some(be) = staged.or(raw.as_deref()) {
                            count.raw(be, true, None, &mut cancelled)?;
                        }
                        // 分离存储读取真实实体 Region；无分离载荷时复用实际地形内嵌列表。
                        if self.entity_overlay.contains_key(&(cx, cz)) {
                            let input = self.entity_input((cx, cz))?;
                            let slot = input
                                .source
                                .starts_with("entity-region:")
                                .then_some((cx, cz));
                            count.raw(&input.before, false, slot, &mut cancelled)?;
                        } else {
                            match crate::entity_catalog::storage(
                                &self.world,
                                &self.dimension,
                                cx,
                                cz,
                            )? {
                                crate::entity_catalog::Storage::Separate(Some((_, entity_raw))) => {
                                    count.raw(&entity_raw, false, Some((cx, cz)), &mut cancelled)?
                                }
                                crate::entity_catalog::Storage::Separate(None)
                                | crate::entity_catalog::Storage::Embedded => {
                                    if let Some(raw) = &raw {
                                        count.raw(raw, false, None, &mut cancelled)?;
                                    }
                                }
                            }
                        }
                        progress(count.progress("分析选区", examined, count.report.chunks));
                    }
                }
            }
        }
        stop(&mut cancelled)?;
        progress(count.progress("选区分析完成", examined, count.report.chunks));
        Ok(count.finish())
    }
}
/// 独立结构按局部选区读取；原始对象载荷只用于 ID，不借用后台世界的会话或版本。
pub fn structure(
    source: &BlockClipboard,
    source_label: String,
    region: SelectionRegion,
    budget: EditWorkBudget,
    mut cancelled: impl FnMut() -> bool,
    mut progress: impl FnMut(EditWorkProgress),
) -> Result<Report, WorldError> {
    if region.shape == SelectionShape::Chunks {
        return Err(refuse("独立结构不使用世界区块选区"));
    }
    let lo = region.bounds.minimum;
    let hi = region.bounds.maximum;
    if (0..3).any(|axis| lo[axis] < 0 || hi[axis] >= i32::from(source.size[axis])) {
        return Err(refuse("分析选区超出结构局部范围"));
    }
    let mut count = Counter::new(source_label, region, budget)?;
    let mut examined = 0usize;
    for y in lo[1]..=hi[1] {
        for z in lo[2]..=hi[2] {
            for x in lo[0]..=hi[0] {
                if examined.is_multiple_of(4096) {
                    stop(&mut cancelled)?;
                    progress(count.progress("分析结构选区", examined, 0));
                }
                examined += 1;
                if !region.contains((x, y, z)) {
                    continue;
                }
                count.report.selected += 1;
                let index = ((y as usize * usize::from(source.size[2]) + z as usize)
                    * usize::from(source.size[0]))
                    + x as usize;
                if source.mask.as_ref().is_some_and(|mask| !mask[index]) {
                    count.report.undeclared_cells += 1;
                } else {
                    count.block(&source.blocks[index])?;
                }
            }
        }
    }
    for (position, compound, block) in source
        .block_entities
        .iter()
        .map(|entry| (entry.position.map(f64::from), &entry.compound, true))
        .chain(
            source
                .entities
                .iter()
                .map(|entry| (entry.position, &entry.compound, false)),
        )
    {
        stop(&mut cancelled)?;
        if !region.contains_point(position) {
            continue;
        }
        budget.check(
            count
                .retained
                .saturating_mul(2)
                .saturating_add(compound.len().saturating_mul(8)),
        )?;
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(compound);
        let root = parse_root(&raw).map_err(|e| refuse(e.to_string()))?;
        count.object(object_id(&root)?, block)?;
    }
    stop(&mut cancelled)?;
    progress(count.progress("选区分析完成", examined, 0));
    Ok(count.finish())
}
