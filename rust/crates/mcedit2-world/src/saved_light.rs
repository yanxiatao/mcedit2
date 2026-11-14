//! 已保存光照的只读快照；保留缺失通道、待重算标记及现代纯光照区段。
use crate::sections::{DecodeError, Layout, Tag};
use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// 光照通道缺失不等于零，也不等于满亮；使用方根据实际格式合同解释。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Levels {
    pub block: Option<u8>,
    pub sky: Option<u8>,
}
impl Levels {
    pub fn description(self) -> String {
        let level = |value: Option<u8>| value.map_or_else(|| "未保存".into(), |n| n.to_string());
        format!("方块光 {} / 天空光 {}", level(self.block), level(self.sky))
    }
}

/// 原格心及六邻格的渲染光照；按已核实的保存合同解释省略值，未知通道仍保留存在位。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Samples([u16; 7]);
impl Samples {
    pub(crate) const OFFSETS: [[i32; 3]; 7] = [
        [0, 0, 0],
        [1, 0, 0],
        [-1, 0, 0],
        [0, 1, 0],
        [0, -1, 0],
        [0, 0, 1],
        [0, 0, -1],
    ];
    pub fn center(self) -> Levels {
        self.get(0)
    }
    /// axis 为 X/Y/Z 的 0/1/2；正负面直接选择实际相邻格。
    pub fn face(self, axis: usize, positive: bool) -> Levels {
        self.get(1 + axis * 2 + usize::from(!positive))
    }
    fn get(self, index: usize) -> Levels {
        let value = self.0[index];
        Levels {
            block: (value & 0x100 != 0).then_some((value & 15) as u8),
            sky: (value & 0x200 != 0).then_some(((value >> 4) & 15) as u8),
        }
    }
    pub(crate) fn capture(position: [i32; 3], mut sample: impl FnMut([i32; 3]) -> Levels) -> Self {
        Self(Self::positions(position).map(|position| {
            let value = sample(position);
            value.block.map_or(0, |n| u16::from(n) | 0x100)
                | value.sky.map_or(0, |n| (u16::from(n) << 4) | 0x200)
        }))
    }
    pub(crate) fn positions(position: [i32; 3]) -> [[i32; 3]; 7] {
        Self::OFFSETS.map(|offset| std::array::from_fn(|i| position[i] + offset[i]))
    }
}

#[derive(Debug, Clone, Default)]
struct Layer {
    block: Option<Arc<[u8]>>,
    sky: Option<Arc<[u8]>>,
}
impl Layer {
    fn sample(&self, index: usize) -> Levels {
        let sample = |array: &Option<Arc<[u8]>>| {
            array
                .as_ref()
                .map(|bytes| bytes[index / 2] >> ((index % 2) * 4) & 15)
        };
        Levels {
            block: sample(&self.block),
            sky: sample(&self.sky),
        }
    }
    fn allocated_bytes(&self) -> usize {
        self.block.as_ref().map_or(0, |b| b.len()) + self.sky.as_ref().map_or(0, |b| b.len())
    }
}

#[derive(Debug, Clone, Default)]
enum Storage {
    #[default]
    Absent,
    McRegion(Layer),
    Sections(BTreeMap<i32, Layer>),
}

/// 只解释核实过的版本；原始数组始终由 sample 原样提供。
#[derive(Debug, Clone, Default)]
enum Interpretation {
    #[default]
    Raw,
    ModernComplete,
    Anvil112 {
        heights: Option<Arc<[i32]>>,
    },
}

/// 与解码区块共存的不可变光照；克隆只共享各 nibble 数组，不重复复制原数据。
#[derive(Debug, Clone, Default)]
pub struct Chunk {
    /// 现代 isLightOn 或旧 LightPopulated；缺少标记保持 None。
    pub ready: Option<bool>,
    status: Option<String>,
    storage: Storage,
    interpretation: Interpretation,
}
impl Chunk {
    pub(crate) fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }
    /// 显示采用已保存派生数据及显式元数据差异；几何变化时停止完整光照的省略值推导。
    pub(crate) fn for_display(
        &self,
        layout: &Layout,
        version: Option<i32>,
        ready: Option<Option<bool>>,
        status: Option<Option<String>>,
        pending_geometry: bool,
    ) -> Self {
        let mut result = self.clone();
        if let Some(ready) = ready {
            result.ready = ready;
        }
        if let Some(status) = status {
            result.status = status;
        }
        if pending_geometry {
            result.ready = Some(false);
            result.interpretation = Interpretation::Raw;
        } else if matches!(layout, Layout::Modern118) {
            result.interpretation = if matches!(version, Some(3465 | 3955))
                && result.ready == Some(true)
                && matches!(result.status.as_deref(), Some("full" | "minecraft:full"))
            {
                Interpretation::ModernComplete
            } else {
                Interpretation::Raw
            };
        }
        result
    }
    pub fn sample(&self, x: u8, y: i32, z: u8) -> Levels {
        if x >= 16 || z >= 16 {
            return Levels::default();
        }
        match &self.storage {
            Storage::Absent => Levels::default(),
            Storage::McRegion(layer) if (0..128).contains(&y) => {
                layer.sample((usize::from(x) * 16 + usize::from(z)) * 128 + y as usize)
            }
            Storage::McRegion(_) => Levels::default(),
            Storage::Sections(sections) => {
                sections
                    .get(&y.div_euclid(16))
                    .map_or_else(Levels::default, |layer| {
                        layer.sample(
                            (y.rem_euclid(16) as usize * 16 + usize::from(z)) * 16 + usize::from(x),
                        )
                    })
            }
        }
    }
    /// 调用方持有真实非空气源格，采样点在该格或六邻格，因而位于已分配光照的段。
    /// 完整现代存档省略的齐次零层为零；不用于没有这种来源证明的任意空气点或实体。
    fn sample_allocated(&self, x: u8, y: i32, z: u8) -> Levels {
        match &self.interpretation {
            Interpretation::ModernComplete => {
                let raw = self.sample(x, y, z);
                Levels {
                    block: Some(raw.block.unwrap_or(0)),
                    sky: Some(raw.sky.unwrap_or(0)),
                }
            }
            Interpretation::Anvil112 { heights } => {
                // World 的原始光照入口将负 Y 归到 0；高度上界外使用 LightLayer 默认值。
                let y = y.max(0);
                if y >= 256 {
                    return Levels {
                        block: Some(0),
                        sky: Some(15),
                    };
                }
                let Storage::Sections(sections) = &self.storage else {
                    unreachable!("1.12 Anvil 使用区段存储")
                };
                if sections.contains_key(&y.div_euclid(16)) {
                    return self.sample(x, y, z);
                }
                Levels {
                    block: Some(0),
                    sky: heights.as_ref().map(|heights| {
                        if y >= heights[usize::from(z) * 16 + usize::from(x)] {
                            15
                        } else {
                            0
                        }
                    }),
                }
            }
            Interpretation::Raw => self.sample(x, y, z),
        }
    }
    pub fn allocated_bytes(&self) -> usize {
        let height_bytes = match &self.interpretation {
            Interpretation::Anvil112 { heights } => heights
                .as_ref()
                .map_or(0, |h| h.len() * std::mem::size_of::<i32>()),
            _ => 0,
        };
        height_bytes
            + self.status.as_ref().map_or(0, String::len)
            + match &self.storage {
                Storage::Absent => 0,
                Storage::McRegion(layer) => layer.allocated_bytes(),
                Storage::Sections(sections) => {
                    sections.values().map(Layer::allocated_bytes).sum::<usize>()
                        + sections.len() * std::mem::size_of::<(i32, Layer)>()
                }
            }
    }
    pub(crate) fn read(
        source: &Tag<'_>,
        layout: &Layout,
        version: Option<i32>,
    ) -> Result<Self, DecodeError> {
        let ready = one(
            source,
            if matches!(layout, Layout::Modern118) {
                "isLightOn"
            } else {
                "LightPopulated"
            },
        )?
        .map(|value| value.byte().map(|value| value != 0))
        .transpose()?;
        let storage = if matches!(layout, Layout::McRegion128) {
            Storage::McRegion(layer(source, 16384)?)
        } else {
            let name = if matches!(layout, Layout::Modern118) {
                "sections"
            } else {
                "Sections"
            };
            let list = one(source, name)?.ok_or_else(|| invalid(format!("光照来源缺少 {name}")))?;
            if list.kind != 9 || list.items.iter().any(|entry| entry.kind != 10) {
                return Err(invalid("光照区段必须为 Compound 列表"));
            }
            let mut sections = BTreeMap::new();
            for entry in &list.items {
                let y = i32::from(
                    one(entry, "Y")?
                        .ok_or_else(|| invalid("光照区段缺少 Y"))?
                        .byte()?,
                );
                if sections.insert(y, layer(entry, 2048)?).is_some() {
                    return Err(invalid(format!("光照区段 Y 重复：{y}")));
                }
            }
            Storage::Sections(sections)
        };
        let status = if matches!(layout, Layout::Modern118) {
            one(source, "Status")?.map(Tag::string).transpose()?
        } else {
            None
        };
        let interpretation = match (layout, version, ready) {
            (Layout::Modern118, Some(3465 | 3955), Some(true)) => {
                if matches!(status.as_deref(), Some("full" | "minecraft:full")) {
                    Interpretation::ModernComplete
                } else {
                    Interpretation::Raw
                }
            }
            (Layout::NumericAnvil112, Some(1343), _) => Interpretation::Anvil112 {
                heights: heights(source)?,
            },
            _ => Interpretation::Raw,
        };
        Ok(Self {
            ready,
            status,
            storage,
            interpretation,
        })
    }
}

struct Column {
    complete: bool,
    allocated: BTreeSet<i32>,
}
/// 以原始段与真实邻块重建光照层分配；暂存方块不作为新光源或已分配段的证明。
pub(crate) struct Sampler<'a, C: Borrow<crate::sections::Chunk>> {
    chunks: &'a BTreeMap<(i32, i32), Option<C>>,
    columns: BTreeMap<(i32, i32), Column>,
}
impl<'a, C: Borrow<crate::sections::Chunk>> Sampler<'a, C> {
    pub(crate) fn new(chunks: &'a BTreeMap<(i32, i32), Option<C>>) -> Self {
        if !chunks.values().flatten().any(|chunk| {
            let chunk: &crate::sections::Chunk = chunk.borrow();
            matches!(chunk.light.interpretation, Interpretation::ModernComplete)
        }) {
            return Self {
                chunks,
                columns: BTreeMap::new(),
            };
        }
        let occupied: BTreeMap<_, Vec<_>> = chunks
            .iter()
            .filter_map(|(key, chunk)| {
                let chunk: &crate::sections::Chunk = chunk.as_ref()?.borrow();
                let sections = chunk
                    .sections
                    .iter()
                    .filter(|section| {
                        let air: Vec<_> =
                            section.palette.iter().map(crate::surface::is_air).collect();
                        section
                            .indices
                            .iter()
                            .any(|&index| !air[usize::from(index)])
                    })
                    .map(|section| i32::from(section.y))
                    .collect();
                Some((*key, sections))
            })
            .collect();
        let columns = chunks
            .iter()
            .filter_map(|(&(cx, cz), chunk)| {
                let chunk: &crate::sections::Chunk = chunk.as_ref()?.borrow();
                let mut column = Column {
                    complete: true,
                    allocated: BTreeSet::new(),
                };
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let key = (cx + dx, cz + dz);
                        column.complete &= chunks.contains_key(&key);
                        if let Some(sections) = occupied.get(&key) {
                            for &sy in sections {
                                column.allocated.extend(sy - 1..=sy + 1);
                            }
                        }
                    }
                }
                // 保存的任意光照数组同样证明该段实际持有光照层，包括纯光照区段。
                if let Storage::Sections(sections) = &chunk.light.storage {
                    column
                        .allocated
                        .extend(sections.iter().filter_map(|(&sy, layer)| {
                            (layer.block.is_some() || layer.sky.is_some()).then_some(sy)
                        }));
                }
                Some(((cx, cz), column))
            })
            .collect();
        Self { chunks, columns }
    }
    /// 仅补齐确实缺少分配来源的邻块；原始非空气地形的邻面通常已经有充分证明。
    pub(crate) fn required(&self, [x, y, z]: [i32; 3], out: &mut BTreeSet<(i32, i32)>) {
        let key = (x.div_euclid(16), z.div_euclid(16));
        match self.chunks.get(&key) {
            Some(Some(chunk)) => {
                let chunk: &crate::sections::Chunk = chunk.borrow();
                if !matches!(chunk.light.interpretation, Interpretation::ModernComplete) {
                    return;
                }
                let raw = chunk
                    .light
                    .sample(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8);
                let column = &self.columns[&key];
                if raw.sky.is_some()
                    || column.allocated.contains(&y.div_euclid(16))
                    || column.complete
                {
                    return;
                }
            }
            Some(None) => return,
            None => (),
        }
        for dz in -1..=1 {
            for dx in -1..=1 {
                let next = (key.0 + dx, key.1 + dz);
                if !self.chunks.contains_key(&next) {
                    out.insert(next);
                }
            }
        }
    }
    pub(crate) fn sample(&self, [x, y, z]: [i32; 3]) -> Levels {
        let key = (x.div_euclid(16), z.div_euclid(16));
        let Some(chunk) = self.chunks.get(&key).and_then(Option::as_ref) else {
            return Levels::default();
        };
        let chunk: &crate::sections::Chunk = chunk.borrow();
        let (x, z) = (x.rem_euclid(16) as u8, z.rem_euclid(16) as u8);
        match &chunk.light.interpretation {
            Interpretation::Raw => chunk.light.sample(x, y, z),
            Interpretation::Anvil112 { .. } => chunk.light.sample_allocated(x, y, z),
            Interpretation::ModernComplete => {
                let column = &self.columns[&key];
                let sy = y.div_euclid(16);
                if column.allocated.contains(&sy) {
                    return chunk.light.sample_allocated(x, y, z);
                }
                let raw = chunk.light.sample(x, y, z);
                let sky = if column.complete {
                    // 未分配空气段读取上方第一个实际光照层的底行；高于全部来源时为天空默认值。
                    Some(
                        column
                            .allocated
                            .range(sy + 1..)
                            .next()
                            .map_or(15, |&above| {
                                chunk.light.sample(x, above * 16, z).sky.unwrap_or(0)
                            }),
                    )
                } else {
                    raw.sky
                };
                Levels {
                    block: Some(raw.block.unwrap_or(0)),
                    sky,
                }
            }
        }
    }
}

fn invalid(message: impl Into<String>) -> DecodeError {
    DecodeError(message.into())
}
fn heights(source: &Tag<'_>) -> Result<Option<Arc<[i32]>>, DecodeError> {
    let Some(value) = one(source, "HeightMap")? else {
        return Ok(None);
    };
    if value.kind != 11
        || value.raw.len() != 4 + 256 * 4
        || i32::from_be_bytes(value.raw[..4].try_into().unwrap()) != 256
    {
        return Err(invalid("1.12 HeightMap 必须为 256 项 Int 数组"));
    }
    Ok(Some(
        value.raw[4..]
            .chunks_exact(4)
            .map(|bytes| i32::from_be_bytes(bytes.try_into().unwrap()))
            .collect(),
    ))
}
fn one<'a, 'b>(tag: &'a Tag<'b>, name: &str) -> Result<Option<&'a Tag<'b>>, DecodeError> {
    let mut values = tag.fields.iter().filter(|(key, _)| key == name);
    let result = values.next().map(|(_, value)| value);
    if values.next().is_some() {
        return Err(invalid(format!("光照来源字段重复：{name}")));
    }
    Ok(result)
}
fn array(tag: &Tag<'_>, name: &str, length: usize) -> Result<Option<Arc<[u8]>>, DecodeError> {
    let Some(value) = one(tag, name)? else {
        return Ok(None);
    };
    if value.kind != 7
        || value.raw.len() != length + 4
        || i32::from_be_bytes(value.raw[..4].try_into().unwrap()) != length as i32
    {
        return Err(invalid(format!(
            "{name} 必须为 {length} 字节的 nibble 数组"
        )));
    }
    Ok(Some(Arc::from(&value.raw[4..])))
}
fn layer(tag: &Tag<'_>, length: usize) -> Result<Layer, DecodeError> {
    Ok(Layer {
        block: array(tag, "BlockLight", length)?,
        sky: array(tag, "SkyLight", length)?,
    })
}
