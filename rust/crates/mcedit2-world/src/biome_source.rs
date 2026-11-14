//! 渲染只读的原始 biome 调色板；不保留方块数据，不把缺失区块补成平原。
use crate::shared_text::SharedText;
use crate::{sections::Chunk, WorldError};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Section {
    y: i8,
    palette: Vec<SharedText>,
    indices: Option<Box<[u16; 64]>>,
}
#[derive(Debug, Clone)]
pub struct Column {
    sections: Vec<Section>,
    numeric: Option<Arc<crate::numeric_biome::NumericBiomes>>,
}
impl Column {
    /// 只复制已解码的真实 biome 数据；全一致 Section 不分配索引数组。
    pub fn from_chunk(chunk: &Chunk) -> Result<Arc<Self>, WorldError> {
        let mut sections = Vec::new();
        let mut names = BTreeMap::<&str, SharedText>::new();
        for source in &chunk.sections {
            let Some(biomes) = &source.biomes else {
                continue;
            };
            if biomes.names.is_empty()
                || biomes.indices.len() != 64
                || biomes
                    .indices
                    .iter()
                    .any(|i| usize::from(*i) >= biomes.names.len())
            {
                return Err(WorldError::Invalid(
                    "渲染 biome 调色板必须包含 64 个有效采样点".into(),
                ));
            }
            let palette = biomes
                .names
                .iter()
                .map(|name| {
                    names
                        .entry(name)
                        .or_insert_with(|| SharedText::from(name.as_str()))
                        .clone()
                })
                .collect();
            let uniform = biomes.indices.iter().all(|i| *i == biomes.indices[0]);
            let (palette, indices) = if uniform {
                (
                    vec![names[biomes.names[usize::from(biomes.indices[0])].as_str()].clone()],
                    None,
                )
            } else {
                (
                    palette,
                    Some(Box::new(biomes.indices.as_slice().try_into().map_err(
                        |_| WorldError::Invalid("渲染 biome 索引长度无效".into()),
                    )?)),
                )
            };
            sections.push(Section {
                y: source.y,
                palette,
                indices,
            });
        }
        Ok(Arc::new(Self {
            sections,
            numeric: chunk.numeric_biomes.clone(),
        }))
    }
    fn quart(&self, x: i32, y: i32, z: i32) -> Option<&str> {
        let sy = i8::try_from(y.div_euclid(4)).ok()?;
        let section = &self.sections[self.sections.binary_search_by_key(&sy, |s| s.y).ok()?];
        let index = (y.rem_euclid(4) * 16 + z.rem_euclid(4) * 4 + x.rem_euclid(4)) as usize;
        let palette = section
            .indices
            .as_ref()
            .map_or(0, |indices| usize::from(indices[index]));
        section.palette.get(palette).map(AsRef::as_ref)
    }
    pub fn allocated_bytes(&self) -> usize {
        let mut names = std::collections::HashSet::new();
        std::mem::size_of::<Self>()
            + self.numeric.as_ref().map_or(0, |v| v.allocated_bytes())
            + self.sections.capacity() * std::mem::size_of::<Section>()
            + self
                .sections
                .iter()
                .map(|s| {
                    s.palette.capacity() * std::mem::size_of::<SharedText>()
                        + s.palette
                            .iter()
                            .filter(|name| names.insert(name.allocation_id()))
                            .map(|name| name.allocation_bytes())
                            .sum::<usize>()
                        + s.indices.as_ref().map_or(0, |_| 128)
                })
                .sum::<usize>()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Region {
    seed: Option<i64>,
    geometry: Option<crate::world_data::Geometry>,
    columns: BTreeMap<(i32, i32), Arc<Column>>,
    preview: Option<Arc<BlockBiomes>>,
    numeric_preview: Option<Arc<NumericBlockBiomes>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericSample {
    /// 容器存在但高度合同未绑定时，没有可解释的坐标样本。
    pub id: Option<i32>,
    pub data_version: Option<i32>,
}
#[derive(Debug)]
struct NumericBlockBiomes {
    origin: [i32; 3],
    volume: crate::numeric_biome::Volume,
}
#[derive(Debug)]
struct BlockBiomes {
    origin: [i32; 3],
    size: [u16; 3],
    cells: Vec<SharedText>,
}
impl Region {
    pub fn new(world_seed: Option<i64>) -> Self {
        // Guava hashLong / HashCode.asLong 均按小端序处理 long。
        let seed = world_seed.map(|seed| {
            let hash = Sha256::digest(seed.to_le_bytes());
            i64::from_le_bytes(hash[..8].try_into().expect("SHA-256 至少有八字节"))
        });
        Self {
            seed,
            geometry: None,
            columns: BTreeMap::new(),
            preview: None,
            numeric_preview: None,
        }
    }
    pub fn insert(&mut self, x: i32, z: i32, column: Arc<Column>) {
        if column.sections.is_empty() && column.numeric.is_none() {
            self.columns.remove(&(x, z));
        } else {
            self.columns.insert((x, z), column);
        }
    }
    /// 重读不可变颜色快照后复用内容相同的列；保持邻域缓存身份，不保留整块 NBT。
    pub fn reuse_columns(&mut self, previous: &Self) {
        for (coordinate, column) in &mut self.columns {
            if let Some(old) = previous.columns.get(coordinate) {
                let numeric_equal = match (&column.numeric, &old.numeric) {
                    (Some(a), Some(b)) => {
                        a.storage == b.storage
                            && a.data_version == b.data_version
                            && a.values == b.values
                    }
                    (None, None) => true,
                    _ => false,
                };
                if column.sections == old.sections && numeric_equal {
                    *column = Arc::clone(old);
                }
            }
        }
    }
    pub fn set_seed(&mut self, world_seed: Option<i64>) {
        self.seed = Self::new(world_seed).seed;
    }
    /// 只接受世界来源已解析的维度高度；缺失合同保持未知，不从 Section 推测。
    pub fn set_geometry(&mut self, geometry: crate::world_data::Geometry) {
        self.geometry = Some(geometry);
    }
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty() && self.preview.is_none() && self.numeric_preview.is_none()
    }
    /// 合并独立显示 patch 的颜色邻域时复用不可变列，不复制区块或调色板。
    pub fn columns(&self) -> impl Iterator<Item = ((i32, i32), &Arc<Column>)> {
        self.columns
            .iter()
            .map(|(&coordinate, column)| (coordinate, column))
    }
    /// 只读原始数字采样；没有注册表时不生成命名 ID 或套用现代颜色取样规则。
    pub fn numeric_sample(&self, position: (i32, i32, i32)) -> Option<i32> {
        self.numeric_at(position)?.id
    }
    pub fn numeric_at(&self, position: (i32, i32, i32)) -> Option<NumericSample> {
        if let Some(preview) = &self.preview {
            let p = [position.0, position.1, position.2];
            if (0..3).all(|i| {
                let local = i64::from(p[i]) - i64::from(preview.origin[i]);
                local >= 0 && local < i64::from(preview.size[i])
            }) {
                return None;
            }
        }
        if let Some(preview) = &self.numeric_preview {
            let p = [position.0, position.1, position.2];
            let local: [i64; 3] =
                std::array::from_fn(|i| i64::from(p[i]) - i64::from(preview.origin[i]));
            if local
                .iter()
                .zip(preview.volume.size)
                .all(|(&p, s)| p >= 0 && p < i64::from(s))
            {
                let index = (local[1] as usize * usize::from(preview.volume.size[2])
                    + local[2] as usize)
                    * usize::from(preview.volume.size[0])
                    + local[0] as usize;
                return Some(NumericSample {
                    id: Some(preview.volume.cells[index]),
                    data_version: preview.volume.data_version,
                });
            }
        }
        let data = self
            .columns
            .get(&(position.0.div_euclid(16), position.2.div_euclid(16)))?
            .numeric
            .as_ref()?;
        Some(NumericSample {
            id: data.sample(
                position.0.rem_euclid(16) as u8,
                position.1,
                position.2.rem_euclid(16) as u8,
            ),
            data_version: data.data_version,
        })
    }
    /// 独立结构预览保留真实数字 ID 与源区块版本，不借用目标世界的颜色。
    pub fn with_numeric_biomes(
        mut self,
        origin: [i32; 3],
        volume: &crate::numeric_biome::Volume,
    ) -> Result<Self, WorldError> {
        volume.validate(volume.size)?;
        if self.preview.is_some() {
            return Err(WorldError::Invalid(
                "预览不能同时声明数字和命名 biome".into(),
            ));
        }
        self.numeric_preview = Some(Arc::new(NumericBlockBiomes {
            origin,
            volume: volume.clone(),
        }));
        Ok(self)
    }
    /// 结构预览使用其明确携带的逐格 biome，边界以外仍查询目标世界邻域。
    pub fn with_block_biomes(
        self,
        origin: [i32; 3],
        size: [u16; 3],
        cells: &[String],
    ) -> Result<Self, WorldError> {
        self.with_block_biomes_with_budget(
            origin,
            size,
            cells,
            crate::editing::EditWorkBudget::default(),
        )
    }
    pub fn with_block_biomes_with_budget(
        mut self,
        origin: [i32; 3],
        size: [u16; 3],
        cells: &[String],
        budget: crate::editing::EditWorkBudget,
    ) -> Result<Self, WorldError> {
        if self.numeric_preview.is_some() {
            return Err(WorldError::Invalid(
                "预览不能同时声明数字和命名 biome".into(),
            ));
        }
        let count = size
            .iter()
            .try_fold(1_usize, |n, &s| n.checked_mul(usize::from(s)))
            .ok_or_else(|| WorldError::Invalid("预览 biome 尺寸溢出".into()))?;
        crate::clipboard::check_volume(count, budget)?;
        if count != cells.len() || size.contains(&0) {
            return Err(WorldError::Invalid("预览 biome 体积必须完整".into()));
        }
        let mut palette = BTreeMap::<&str, SharedText>::new();
        let cells = cells
            .iter()
            .map(|id| {
                palette
                    .entry(id)
                    .or_insert_with(|| SharedText::from(id.as_str()))
                    .clone()
            })
            .collect();
        self.preview = Some(Arc::new(BlockBiomes {
            origin,
            size,
            cells,
        }));
        Ok(self)
    }
    /// 颜色只依赖一圈源区块；相邻 patch 到达或暂存 biome 改变时使相应网格失效。
    pub fn same_neighborhood(&self, other: &Self, cx: i32, cz: i32) -> bool {
        let preview_equal = match (&self.preview, &other.preview) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        let numeric_preview_equal = match (&self.numeric_preview, &other.numeric_preview) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        self.seed == other.seed
            && self.geometry == other.geometry
            && preview_equal
            && numeric_preview_equal
            && (-1..=1).all(|dx| {
                (-1..=1).all(|dz| {
                    match (
                        self.columns.get(&(cx + dx, cz + dz)),
                        other.columns.get(&(cx + dx, cz + dz)),
                    ) {
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    }
                })
            })
    }
    fn quart(&self, x: i32, y: i32, z: i32) -> Option<&str> {
        let y = self.geometry.map_or(y, |geometry| geometry.quart_y(y));
        self.columns
            .get(&(x.div_euclid(4), z.div_euclid(4)))?
            .quart(x, y, z)
    }
    /// 等价于原版 BiomeManager 的八角点模糊缩放；未知种子不能冒充种子 0。
    /// 原版在选定角点后按维度钳制 quart Y；未知合同或范围内缺失数据仍为未知。
    pub fn sample(&self, position: (i32, i32, i32)) -> Option<&str> {
        if let Some(preview) = &self.preview {
            let p = [position.0, position.1, position.2];
            let local: [i64; 3] =
                std::array::from_fn(|i| i64::from(p[i]) - i64::from(preview.origin[i]));
            if local
                .iter()
                .zip(preview.size)
                .all(|(&p, s)| p >= 0 && p < i64::from(s))
            {
                let index = (local[1] as usize * usize::from(preview.size[2]) + local[2] as usize)
                    * usize::from(preview.size[0])
                    + local[0] as usize;
                return preview.cells.get(index).map(AsRef::as_ref);
            }
        }
        if self.numeric_at(position).is_some() {
            return None;
        }
        let p = [
            position.0.checked_sub(2)?,
            position.1.checked_sub(2)?,
            position.2.checked_sub(2)?,
        ];
        let q = p.map(|v| v.div_euclid(4));
        let fraction = p.map(|v| f64::from(v.rem_euclid(4)) / 4.);
        let corners: [[i32; 3]; 8] = std::array::from_fn(|i| {
            [
                q[0] + ((i & 4 != 0) as i32),
                q[1] + ((i & 2 != 0) as i32),
                q[2] + ((i & 1 != 0) as i32),
            ]
        });
        let first = self.quart(q[0], q[1], q[2]);
        // 单一 biome 区域直接返回，省去八次种子扰动，也可处理缺失 seed 的一致数据。
        if first.is_some()
            && corners[1..]
                .iter()
                .all(|p| self.quart(p[0], p[1], p[2]) == first)
        {
            return first;
        }
        let seed = self.seed?;
        let mut selected = 0;
        let mut nearest = f64::INFINITY;
        for (index, point) in corners.iter().enumerate() {
            let mut mixed = seed;
            for coordinate in point.iter().chain(point.iter()) {
                mixed = mix(mixed, i64::from(*coordinate));
            }
            let dx = fraction[0] - if index & 4 != 0 { 1. } else { 0. } + fiddle(mixed);
            mixed = mix(mixed, seed);
            let dy = fraction[1] - if index & 2 != 0 { 1. } else { 0. } + fiddle(mixed);
            mixed = mix(mixed, seed);
            let dz = fraction[2] - if index & 1 != 0 { 1. } else { 0. } + fiddle(mixed);
            let distance = dz * dz + dy * dy + dx * dx;
            if distance < nearest {
                nearest = distance;
                selected = index;
            }
        }
        let p = corners[selected];
        self.quart(p[0], p[1], p[2])
    }
}
fn mix(seed: i64, salt: i64) -> i64 {
    seed.wrapping_mul(
        seed.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407),
    )
    .wrapping_add(salt)
}
fn fiddle(seed: i64) -> f64 {
    ((seed >> 24).rem_euclid(1024) as f64 / 1024. - 0.5) * 0.9
}
