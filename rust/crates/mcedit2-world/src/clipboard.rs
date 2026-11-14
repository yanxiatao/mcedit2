//! 带原始载荷和版本来源的结构剪贴板。
use crate::editing::{parse_state, render_state};
use crate::sections::State;
use crate::WorldError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const LEGACY_FORMAT: &str = "MCEDIT2_RUST_BLOCK_ONLY_CLIPBOARD";
const FORMAT: &str = "MCEDIT2_RUST_STRUCTURE_CLIPBOARD";
const VERSION: u32 = 2;
#[path = "biome_samples.rs"]
mod biome_samples;
#[path = "script_generation.rs"]
mod generation;
#[path = "script_copy.rs"]
mod script_copy;
#[path = "clipboard_work.rs"]
mod work;
use crate::editing::EditWorkBudget;
pub use crate::scheduled_ticks::{Kind as TickKind, Tick as OpaqueScheduledTick};
pub use biome_samples::ExpandBiomes;
pub use generation::Generated;
pub use script_copy::{CopyFilter, ScriptCopy, ScriptCopyResult, ScriptDimension, ScriptSource};
pub(crate) use work::check_volume;
pub use work::state_bytes;
const MAX_BYTES: usize = 64 * 1024 * 1024;
fn refuse(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueBlockEntity {
    pub position: [i32; 3],
    /// 不含 TAG_Compound 头的原始载荷。
    pub compound: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueEntity {
    pub position: [f64; 3],
    #[serde(default)]
    pub block_position: Option<[i32; 3]>,
    /// 根实体和完整 Passengers/Riding 树的原始 Compound 载荷。
    pub compound: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiomeVolume {
    pub resolution: u8,
    pub size: [u16; 3],
    pub cells: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFileProof {
    pub path: std::path::PathBuf,
    pub sha256: String,
}
impl SourceFileProof {
    pub fn verify(&self) -> Result<(), WorldError> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(refuse("结构源文件必须是普通文件"));
        }
        use std::io::Read;
        let mut file = fs::File::open(&self.path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        if format!("{:x}", hasher.finalize()) != self.sha256 {
            return Err(refuse("结构源文件在预览后已变化"));
        }
        Ok(())
    }
    pub fn from_bytes(path: &Path, bytes: &[u8]) -> Result<Self, WorldError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(refuse("结构源不是普通文件"));
        }
        Ok(Self {
            path: fs::canonicalize(path)?,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        })
    }
}
impl BiomeVolume {
    /// 将逐方块采样转换为目标四格群系，拒绝混合值和选区外修改。
    pub fn plan_quart(
        &self,
        destination: [i32; 3],
    ) -> Result<std::collections::BTreeMap<[i32; 3], String>, WorldError> {
        self.plan_quart_masked(destination, None)
    }
    pub fn plan_quart_masked(
        &self,
        destination: [i32; 3],
        mask: Option<&[bool]>,
    ) -> Result<std::collections::BTreeMap<[i32; 3], String>, WorldError> {
        if self.resolution != 1 {
            return Err(refuse("当前 biome 预检需要逐方块采样"));
        }
        let volume = self
            .size
            .iter()
            .try_fold(1usize, |n, &v| n.checked_mul(v as usize))
            .ok_or_else(|| refuse("biome 网格尺寸溢出"))?;
        if volume != self.cells.len() || mask.is_some_and(|mask| mask.len() != volume) {
            return Err(refuse("biome 网格或掩码数据数量不匹配"));
        }
        let mut cells: std::collections::BTreeMap<[i32; 3], (String, usize)> =
            std::collections::BTreeMap::new();
        for y in 0..i32::from(self.size[1]) {
            for z in 0..i32::from(self.size[2]) {
                for x in 0..i32::from(self.size[0]) {
                    let xyz = [x, y, z];
                    let mut quart = [0; 3];
                    for axis in 0..3 {
                        quart[axis] = destination[axis]
                            .checked_add(xyz[axis])
                            .ok_or_else(|| refuse("biome 目标坐标溢出"))?
                            .div_euclid(4);
                    }
                    let index = ((y as usize * self.size[2] as usize + z as usize)
                        * self.size[0] as usize)
                        + x as usize;
                    if mask.is_some_and(|mask| !mask[index]) {
                        continue;
                    }
                    let value = &self.cells[index];
                    let entry = cells.entry(quart).or_insert_with(|| (value.clone(), 0));
                    if &entry.0 != value {
                        return Err(refuse(format!(
                            "目标 biome 四格 {:?} 含多个来源值，无法无损表达",
                            quart
                        )));
                    }
                    entry.1 += 1;
                }
            }
        }
        if let Some((quart, _)) = cells.iter().find(|(_, (_, count))| *count != 64) {
            return Err(refuse(format!(
                "目标 biome 四格 {:?} 仅部分覆盖，会影响选区外，请对齐四格边界或取消复制 biome",
                quart
            )));
        }
        Ok(cells
            .into_iter()
            .map(|(key, (value, _))| (key, value))
            .collect())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PasteOptions {
    pub include_biomes: bool,
}
/// 捕获完整 BE 载荷；实体和生物群系由用户的传输选项分别决定。
#[derive(Debug, Clone, Copy)]
pub struct CaptureOptions {
    pub entities: bool,
    pub biomes: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockClipboard {
    pub format: String,
    pub version: u32,
    pub source_root: String,
    pub dimension: String,
    /// 实体载荷实际来自哪个维度；第三方结构未记录时未知，不使用导入目标冒充来源。
    #[serde(default)]
    pub entity_dimension: Option<String>,
    pub data_version: Option<i32>,
    #[serde(default)]
    pub source_release: Option<String>,
    pub numeric: bool,
    pub source_min: [i32; 3],
    #[serde(default)]
    pub source_bound: bool,
    #[serde(default)]
    pub source_file: Option<SourceFileProof>,
    pub size: [u16; 3],
    // Y-major, then Z, then X; no Block Entity NBT or entities.
    pub blocks: Vec<State>,
    /// None 表示每一格均存在；Some 为原版结构中实际列出的格子。
    #[serde(default)]
    pub mask: Option<Vec<bool>>,
    #[serde(default)]
    pub offset: [i32; 3],
    #[serde(default)]
    pub block_entities: Vec<OpaqueBlockEntity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scheduled_ticks: Vec<OpaqueScheduledTick>,
    #[serde(default)]
    pub entities: Vec<OpaqueEntity>,
    /// 捕获时实际启用的画作定义；原生结构随身保存，变换不重新猜测自定义尺寸。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub painting_variants: std::collections::BTreeMap<String, crate::painting::Variant>,
    #[serde(default)]
    pub biomes: Option<BiomeVolume>,
    /// 数字注册表载荷保持独立类型；不得作为命名 biome 或跨世界注册项导出。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_biomes: Option<crate::numeric_biome::Volume>,
    #[serde(default)]
    pub metadata: Option<Vec<u8>>,
}
impl BlockClipboard {
    /// 用户为未记录维度的外部实体载荷补充真实来源；已有来源不能被隐式覆盖。
    pub fn bind_entity_dimension(&mut self, dimension: &str) -> Result<(), WorldError> {
        crate::world_data::registry_id(dimension)?;
        if self
            .entity_dimension
            .as_deref()
            .is_some_and(|id| id != dimension)
        {
            return Err(refuse("结构已经记录了不同的实体来源维度"));
        }
        self.entity_dimension = Some(dimension.into());
        Ok(())
    }
    pub fn validate(
        &self,
        world: &impl crate::structure_context::CodecContext,
        dimension: &str,
    ) -> Result<(), WorldError> {
        self.validate_with_budget(world, dimension, EditWorkBudget::default())
    }
    pub fn validate_with_budget(
        &self,
        world: &impl crate::structure_context::CodecContext,
        dimension: &str,
        budget: EditWorkBudget,
    ) -> Result<(), WorldError> {
        self.check_work(budget)?;
        if let Some(dimension) = &self.entity_dimension {
            crate::world_data::registry_id(dimension)?;
        }
        for (id, variant) in &self.painting_variants {
            crate::world_data::registry_id(id)?;
            variant.clone().validate().map_err(refuse)?;
        }
        if let Some(world) = world
            .target_world()
            .filter(|_| !self.painting_variants.is_empty())
        {
            let catalog = world
                .world_data
                .painting_catalog(
                    self.data_version
                        .ok_or_else(|| refuse("画作定义缺少源 DataVersion"))?,
                )
                .map_err(refuse)?;
            for (id, variant) in &self.painting_variants {
                if catalog.0.get(id) != Some(variant) {
                    return Err(refuse(format!("目标画作定义与结构来源不同：{id}")));
                }
            }
        }
        if !((self.version == 1 && self.format == LEGACY_FORMAT)
            || (self.version == VERSION && self.format == FORMAT))
        {
            return Err(refuse("Unknown native block-only clipboard format/version"));
        }
        if self.version == 1 && self.has_payload() {
            return Err(refuse("旧版方块剪贴板不能包含结构载荷"));
        }
        if dimension.is_empty() {
            return Err(refuse("剪贴板目标维度为空"));
        }
        if self.data_version != world.data_version() {
            return Err(refuse("结构与目标 DataVersion 不同，尚无可靠版本转换"));
        }
        if world.target_world().is_some()
            && self.source_root != world.identity().to_string_lossy()
            && self.data_version.is_none()
        {
            return Err(refuse("跨世界结构缺少可核实的 DataVersion"));
        }
        if self.numeric
            && world.target_world().is_some()
            && self.source_root != world.identity().to_string_lossy()
            && (self.source_release.is_none() || self.source_release.as_deref() != world.release())
        {
            return Err(refuse("跨世界数字方块缺少匹配的来源版本/注册表"));
        }
        if self.numeric
            && world.target_world().is_some()
            && self.source_root != world.identity().to_string_lossy()
        {
            return Err(refuse("跨世界数字方块尚无法验证目标注册表，拒绝猜测 ID"));
        }
        let volume = self
            .size
            .iter()
            .try_fold(1usize, |total, &size| {
                if size == 0 {
                    return None;
                }
                total.checked_mul(size as usize)
            })
            .ok_or_else(|| refuse("Clipboard dimensions invalid"))?;
        check_volume(volume, budget)?;
        if volume != self.blocks.len() {
            return Err(refuse("Clipboard size and state count differ"));
        }
        if self.mask.as_ref().is_some_and(|mask| mask.len() != volume) {
            return Err(refuse("结构 mask 长度不匹配"));
        }
        for state in &self.blocks {
            if world.target_world().is_some()
                && self.source_root != world.identity().to_string_lossy()
                && !state.name.starts_with("minecraft:")
            {
                return Err(refuse("跨世界模组方块尚无法验证目标注册表"));
            }
            if (self.numeric != state.name.starts_with("numeric:"))
                || parse_state(&render_state(state))? != *state
            {
                return Err(refuse(
                    "Clipboard contains invalid, noncanonical or mismatched block states",
                ));
            }
        }
        let mut total_payload = 0usize;
        let mut positions = std::collections::BTreeSet::new();
        for item in &self.block_entities {
            if item
                .position
                .iter()
                .zip(self.size)
                .any(|(&v, n)| v < 0 || v >= i32::from(n))
            {
                return Err(refuse("方块实体超出剪贴板范围"));
            }
            if !positions.insert(item.position) {
                return Err(refuse("方块实体位置重复"));
            }
            let index = ((item.position[1] as usize * self.size[2] as usize
                + item.position[2] as usize)
                * self.size[0] as usize)
                + item.position[0] as usize;
            if self.mask.as_ref().is_some_and(|mask| !mask[index]) {
                return Err(refuse("方块实体位于结构未声明的格子"));
            }
            total_payload = total_payload
                .checked_add(item.compound.len())
                .ok_or_else(|| refuse("结构载荷长度溢出"))?;
            let mut world_position = [0; 3];
            for axis in 0..3 {
                world_position[axis] = self.source_min[axis]
                    .checked_add(item.position[axis])
                    .ok_or_else(|| refuse("方块实体坐标溢出"))?;
            }
            validate_compound(&item.compound, |tag| {
                for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
                    if tag.field(name).and_then(|v| v.i32().ok()) != Some(world_position[axis]) {
                        return Err(refuse("方块实体自身坐标与结构位置不一致"));
                    }
                }
                Ok(())
            })?;
        }
        for item in &self.scheduled_ticks {
            if item
                .position
                .iter()
                .zip(self.size)
                .any(|(&v, n)| v < 0 || v >= i32::from(n))
            {
                return Err(refuse("计划更新超出剪贴板范围"));
            }
            let index = (item.position[1] as usize * usize::from(self.size[2])
                + item.position[2] as usize)
                * usize::from(self.size[0])
                + item.position[0] as usize;
            if self.mask.as_ref().is_some_and(|mask| !mask[index]) {
                return Err(refuse("计划更新位于结构未声明的格子"));
            }
            let mut absolute = [0; 3];
            for axis in 0..3 {
                absolute[axis] = self.source_min[axis]
                    .checked_add(item.position[axis])
                    .ok_or_else(|| refuse("计划更新坐标溢出"))?;
            }
            if crate::scheduled_ticks::compound_position(&item.compound)? != absolute {
                return Err(refuse("计划更新自身坐标与结构位置不一致"));
            }
            total_payload = total_payload
                .checked_add(item.compound.len())
                .ok_or_else(|| refuse("计划更新载荷大小溢出"))?;
        }
        for item in &self.entities {
            if item
                .position
                .iter()
                .zip(self.size)
                .any(|(&v, n)| !v.is_finite() || v < 0.0 || v >= f64::from(n))
            {
                return Err(refuse("实体位置超出剪贴板范围"));
            }
            total_payload = total_payload
                .checked_add(item.compound.len())
                .ok_or_else(|| refuse("结构载荷长度溢出"))?;
            validate_compound(&item.compound, |tag| {
                let pos = tag.field("Pos").ok_or_else(|| refuse("实体载荷缺少 Pos"))?;
                if pos.kind != 9
                    || pos.items.len() != 3
                    || pos.items.iter().any(|v| v.kind != 6 || v.raw.len() != 8)
                {
                    return Err(refuse("实体载荷 Pos 类型无效"));
                }
                for axis in 0..3 {
                    let raw = f64::from_be_bytes(
                        pos.items[axis]
                            .raw
                            .try_into()
                            .map_err(|_| refuse("实体坐标长度无效"))?,
                    );
                    if raw - f64::from(self.source_min[axis]) != item.position[axis] {
                        return Err(refuse("实体自身位置与结构位置不一致"));
                    }
                }
                Ok(())
            })?;
        }
        budget.check(
            total_payload
                .checked_mul(4)
                .ok_or_else(|| refuse("结构载荷工作集溢出"))?,
        )?;
        if let Some(biomes) = &self.numeric_biomes {
            biomes.validate(self.size)?;
            if self.biomes.is_some()
                || (world.target_world().is_some()
                    && self.source_root != world.identity().to_string_lossy())
            {
                return Err(refuse(
                    "数字 biome 不可与命名载荷混用，或在未知注册表的其他世界使用",
                ));
            }
            total_payload = total_payload
                .checked_add(biomes.cells.len() * 4)
                .ok_or_else(|| refuse("数字 biome 载荷大小溢出"))?;
            budget.check(
                total_payload
                    .checked_mul(4)
                    .ok_or_else(|| refuse("结构载荷工作集溢出"))?,
            )?;
        }
        if let Some(biomes) = &self.biomes {
            biomes.validate_samples(self.source_min, self.size)?;
            total_payload = biomes
                .cells
                .iter()
                .try_fold(total_payload, |n, v| n.checked_add(v.len()))
                .ok_or_else(|| refuse("结构载荷长度溢出"))?;
            budget.check(
                total_payload
                    .checked_mul(4)
                    .ok_or_else(|| refuse("结构载荷工作集溢出"))?,
            )?;
        }
        if let Some(metadata) = &self.metadata {
            total_payload = total_payload
                .checked_add(metadata.len())
                .ok_or_else(|| refuse("结构元数据长度溢出"))?;
            budget.check(
                total_payload
                    .checked_mul(4)
                    .ok_or_else(|| refuse("结构载荷工作集溢出"))?,
            )?;
            validate_compound(metadata, |_| Ok(()))?;
        }
        Ok(())
    }
    pub fn has_non_block_payload(&self) -> bool {
        self.offset != [0; 3]
            || !self.painting_variants.is_empty()
            || !self.block_entities.is_empty()
            || !self.scheduled_ticks.is_empty()
            || !self.entities.is_empty()
            || self.biomes.is_some()
            || self.numeric_biomes.is_some()
            || self.metadata.is_some()
    }
    pub fn has_payload(&self) -> bool {
        if !self.painting_variants.is_empty() {
            return true;
        }
        if self.numeric_biomes.is_some() {
            return true;
        }
        self.offset != [0, 0, 0]
            || self.mask.is_some()
            || !self.block_entities.is_empty()
            || !self.scheduled_ticks.is_empty()
            || !self.entities.is_empty()
            || self.biomes.is_some()
            || self.metadata.is_some()
    }
}
fn validate_compound(
    compound: &[u8],
    inspect: impl FnOnce(&crate::sections::Tag<'_>) -> Result<(), WorldError>,
) -> Result<(), WorldError> {
    if compound.len() > MAX_BYTES {
        return Err(refuse("结构 Compound 超出内存预算"));
    }
    let mut root = vec![10, 0, 0];
    root.extend_from_slice(compound);
    let tag = crate::sections::parse_root(&root)
        .map_err(|e| refuse(format!("结构 Compound 无效：{e}")))?;
    inspect(&tag)
}

/// Orthogonal X/Z transforms. Only orientation rules we can prove are
/// supported; unfamiliar modded BlockState properties are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HorizontalTransform {
    Identity,
    Rotate90,
    Rotate180,
    Rotate270,
    MirrorX,
    MirrorZ,
}
fn direction(value: &str, operation: HorizontalTransform) -> Option<&'static str> {
    let names = ["north", "east", "south", "west"];
    let index = names.iter().position(|name| *name == value)?;
    let next = match operation {
        HorizontalTransform::Identity => index,
        HorizontalTransform::Rotate90 => (index + 1) % 4,
        HorizontalTransform::Rotate180 => (index + 2) % 4,
        HorizontalTransform::Rotate270 => (index + 3) % 4,
        HorizontalTransform::MirrorX => [0, 3, 2, 1][index],
        HorizontalTransform::MirrorZ => [2, 1, 0, 3][index],
    };
    Some(names[next])
}
fn rail_shape(value: &str, operation: HorizontalTransform) -> Option<String> {
    if let Some(up) = value.strip_prefix("ascending_") {
        return Some(format!("ascending_{}", direction(up, operation)?));
    }
    let (a, b) = value.split_once('_')?;
    let a = direction(a, operation)?;
    let b = direction(b, operation)?;
    let pair = [a, b];
    for (name, dirs) in [
        ("north_south", ["north", "south"]),
        ("east_west", ["east", "west"]),
        ("south_east", ["south", "east"]),
        ("south_west", ["south", "west"]),
        ("north_west", ["north", "west"]),
        ("north_east", ["north", "east"]),
    ] {
        if pair.contains(&dirs[0]) && pair.contains(&dirs[1]) {
            return Some(name.into());
        }
    }
    None
}
pub(crate) fn reorient(state: &State, operation: HorizontalTransform) -> Result<State, WorldError> {
    if operation == HorizontalTransform::Identity {
        return Ok(state.clone());
    }
    if state.name.starts_with("numeric:") {
        return Err(refuse(
            "Cannot rotate/mirror numeric metadata without version-specific ID mappings",
        ));
    }
    if !state.name.starts_with("minecraft:") {
        return Err(refuse("模组方块方向规则未知，不能猜测旋转"));
    }
    let mirrored = matches!(
        operation,
        HorizontalTransform::MirrorX | HorizontalTransform::MirrorZ
    );
    let mut next = state.clone();
    for (key, value) in &mut next.properties {
        match key.as_str() {
            "north" | "east" | "south" | "west" => {
                *key = direction(key, operation)
                    .ok_or_else(|| refuse("方向键无效"))?
                    .into();
            }
            "facing" => {
                let directions = ["north", "east", "south", "west"];
                if let Some(index) = directions.iter().position(|s| *s == value.as_str()) {
                    let next_index = match operation {
                        HorizontalTransform::Rotate90 => (index + 1) % 4,
                        HorizontalTransform::Rotate180 => (index + 2) % 4,
                        HorizontalTransform::Rotate270 => (index + 3) % 4,
                        HorizontalTransform::MirrorX => [0, 3, 2, 1][index],
                        HorizontalTransform::MirrorZ => [2, 1, 0, 3][index],
                        HorizontalTransform::Identity => index,
                    };
                    *value = directions[next_index].into();
                } else if value.as_str() != "up" && value.as_str() != "down" {
                    return Err(refuse(
                        "Unrecognized facing orientation in pasted block state",
                    ));
                }
            }
            "axis" => {
                if !matches!(value.as_str(), "x" | "y" | "z") {
                    return Err(refuse("Unrecognized axis orientation"));
                }
                if matches!(
                    operation,
                    HorizontalTransform::Rotate90 | HorizontalTransform::Rotate270
                ) {
                    if value.as_str() == "x" {
                        *value = "z".into();
                    } else if value.as_str() == "z" {
                        *value = "x".into();
                    }
                }
            }
            "rotation" => {
                let parsed = value
                    .parse::<u8>()
                    .map_err(|_| refuse("Invalid 0..15 rotation BlockState"))?;
                if parsed > 15 {
                    return Err(refuse("BlockState rotation exceeds 15"));
                }
                let n = match operation {
                    HorizontalTransform::Rotate90 => (parsed + 4) % 16,
                    HorizontalTransform::Rotate180 => (parsed + 8) % 16,
                    HorizontalTransform::Rotate270 => (parsed + 12) % 16,
                    HorizontalTransform::MirrorX => (16 - parsed) % 16,
                    HorizontalTransform::MirrorZ => (8 + 16 - parsed) % 16,
                    HorizontalTransform::Identity => parsed,
                };
                *value = n.to_string();
            }
            "shape" if state.name.ends_with("rail") => {
                *value = rail_shape(value, operation).ok_or_else(|| refuse("未知铁轨形状"))?;
            }
            "shape" if state.name.ends_with("_stairs") => {
                if !matches!(
                    value.as_str(),
                    "straight" | "inner_left" | "inner_right" | "outer_left" | "outer_right"
                ) {
                    return Err(refuse("未知楼梯形状"));
                }
                if mirrored {
                    *value = match value.as_str() {
                        "inner_left" => "inner_right",
                        "inner_right" => "inner_left",
                        "outer_left" => "outer_right",
                        "outer_right" => "outer_left",
                        _ => "straight",
                    }
                    .into();
                }
            }
            "orientation" if state.name == "minecraft:jigsaw" => {
                let (first, second) = value
                    .split_once('_')
                    .ok_or_else(|| refuse("拼图方块方向无效"))?;
                let map = |v: &str| -> Result<String, WorldError> {
                    if matches!(v, "up" | "down") {
                        Ok(v.into())
                    } else {
                        Ok(direction(v, operation)
                            .ok_or_else(|| refuse("未知拼图方向"))?
                            .into())
                    }
                };
                *value = format!("{}_{}", map(first)?, map(second)?);
            }
            "hinge" if state.name.ends_with("_door") && mirrored => {
                *value = match value.as_str() {
                    "left" => "right",
                    "right" => "left",
                    _ => return Err(refuse("未知门铰链")),
                }
                .into();
            }
            "type" if state.name.ends_with("chest") && mirrored => {
                *value = match value.as_str() {
                    "left" => "right",
                    "right" => "left",
                    "single" => "single",
                    _ => return Err(refuse("未知箱子连接类型")),
                }
                .into();
            }
            "waterlogged" | "lit" | "powered" | "half" | "type" | "age" | "level" | "open"
            | "hinge" | "distance" | "persistent" | "snowy" | "occupied" | "extended"
            | "triggered" | "conditional" | "delay" | "locked" | "power" | "stage" | "moisture"
            | "hatch" | "eggs" | "bites" | "layers" | "pickles" | "candles" | "charges"
            | "honey_level" | "note" | "instrument" | "face" | "attached" | "disarmed"
            | "in_wall" | "short" | "unstable" | "drag" | "enabled" | "signal_fire" | "berries"
            | "bloom" | "sculk_sensor_phase" | "thickness" | "vertical_direction" | "tilt"
            | "mode" | "part" | "variant" | "seamless" | "decayable" | "check_decay" => {}
            _ => {
                return Err(refuse(format!(
                    "Cannot prove safe rotation for unknown state property '{key}'"
                )))
            }
        }
    }
    next.properties.sort_unstable();
    // Validate the transformed literal rather than inventing a new schema.
    let parsed = parse_state(&render_state(&next))?;
    if parsed != next {
        return Err(refuse("Rotated state did not roundtrip canonically"));
    }
    Ok(next)
}
fn reorient_numeric(
    state: &State,
    operation: HorizontalTransform,
    data_version: Option<i32>,
    source_release: Option<&str>,
) -> Result<State, WorldError> {
    reorient_numeric_with(state, data_version, source_release, |state| {
        reorient(state, operation)
    })
}
pub(crate) fn reorient_numeric_with(
    state: &State,
    data_version: Option<i32>,
    source_release: Option<&str>,
    transform: impl FnOnce(&State) -> Result<State, WorldError>,
) -> Result<State, WorldError> {
    if let (Some(1343), Some(release)) = (data_version, source_release) {
        if release != "1.12.2" {
            return Err(refuse("来源版本名称与 DataVersion 冲突"));
        }
    }
    if let (Some(number), Some(release)) = (data_version, source_release) {
        let expected = match release {
            "1.9.4" => Some(184),
            "1.10.2" => Some(512),
            "1.11.2" => Some(922),
            "1.12.2" => Some(1343),
            _ => None,
        };
        if expected != Some(number) {
            return Err(refuse("来源版本名称与 DataVersion 未核实或冲突"));
        }
    }
    let version = if data_version == Some(1343) {
        "1.12"
    } else {
        let release = source_release.ok_or_else(|| refuse("数字方块缺少已核实的来源版本"))?;
        let mut parts = release.split('.');
        if parts.next() != Some("1") {
            return Err(refuse("未知数字方块来源版本"));
        }
        match parts.next() {
            Some("8") => "1.8",
            Some("9") => "1.9",
            Some("10") => "1.10",
            Some("11") => "1.11",
            Some("12") => "1.12",
            _ => return Err(refuse("未知数字方块来源版本")),
        }
    };
    let id = state
        .name
        .strip_prefix("numeric:")
        .and_then(|v| v.parse::<u16>().ok())
        .ok_or_else(|| refuse("数字方块 ID 无效"))?;
    let data = state
        .properties
        .iter()
        .find(|(name, _)| name == "data")
        .and_then(|(_, v)| v.parse::<u8>().ok())
        .ok_or_else(|| refuse("数字方块 metadata 无效"))?;
    if version == "1.12" && matches!(id, 54 | 130 | 146) {
        let facing = crate::legacy_assets::chest_facing_1_12(data)
            .ok_or_else(|| refuse("旧版箱子 metadata 超出 nibble 范围"))?;
        let name = match id {
            54 => "minecraft:chest",
            130 => "minecraft:ender_chest",
            146 => "minecraft:trapped_chest",
            _ => unreachable!(),
        };
        let initial = State {
            name: name.into(),
            properties: vec![("facing".into(), facing.into())],
        };
        let rotated = transform(&initial)?;
        if rotated == initial {
            return Ok(state.clone());
        }
        let value = match rotated.properties.as_slice() {
            [(key, facing)] if rotated.name == initial.name && key == "facing" => {
                crate::legacy_assets::chest_metadata_1_12(facing)
            }
            _ => None,
        }
        .ok_or_else(|| refuse("旧版箱子旋转状态没有对应水平 metadata"))?;
        let mut result = state.clone();
        result.properties = vec![("data".into(), value.to_string())];
        return Ok(result);
    }
    #[derive(Deserialize)]
    struct Mapping {
        entries: Vec<(u16, u8, Option<String>, Option<String>, String, String)>,
    }
    static V18: std::sync::OnceLock<Result<Mapping, String>> = std::sync::OnceLock::new();
    static V19: std::sync::OnceLock<Result<Mapping, String>> = std::sync::OnceLock::new();
    static V110: std::sync::OnceLock<Result<Mapping, String>> = std::sync::OnceLock::new();
    static V111: std::sync::OnceLock<Result<Mapping, String>> = std::sync::OnceLock::new();
    static V112: std::sync::OnceLock<Result<Mapping, String>> = std::sync::OnceLock::new();
    let mapping = match version {
        "1.8" => &V18,
        "1.9" => &V19,
        "1.10" => &V110,
        "1.11" => &V111,
        _ => &V112,
    };
    let entries = &mapping
        .get_or_init(|| {
            serde_json::from_str(crate::legacy_assets::mapping(version).expect("内置旧版资源存在"))
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| refuse(format!("旧版状态映射无效：{e}")))?
        .entries;
    let name = entries
        .iter()
        .find(|(i, d, _, _, _, _)| *i == id && *d == data)
        .map(|(_, _, _, _, _, name)| name)
        .ok_or_else(|| refuse("旧版数字方块不在 1.12.2 状态映射中"))?;
    let initial = parse_state(name)?;
    let rotated = transform(&initial)?;
    if rotated == initial {
        return Ok(state.clone());
    }
    let mut matches = entries.iter().filter(|(i, _, _, _, _, name)| {
        *i == id && parse_state(name).ok().as_ref() == Some(&rotated)
    });
    let value = matches
        .next()
        .ok_or_else(|| refuse("旧版旋转状态没有对应 metadata"))?
        .1;
    if matches.next().is_some() {
        return Err(refuse("旧版旋转状态存在多个 metadata，拒绝猜测"));
    }
    let mut result = state.clone();
    result.properties = vec![("data".into(), value.to_string())];
    Ok(result)
}
impl BlockClipboard {
    pub fn transformed(&self, operation: HorizontalTransform) -> Result<Self, WorldError> {
        let w = self.size[0] as usize;
        let h = self.size[1] as usize;
        let z = self.size[2] as usize;
        if self.blocks.len()
            != w.checked_mul(h)
                .and_then(|n| n.checked_mul(z))
                .ok_or_else(|| refuse("Clipboard volume overflow"))?
        {
            return Err(refuse("Clipboard source state count mismatch"));
        }
        if operation == HorizontalTransform::Identity {
            return Ok(self.clone());
        }
        if self.source_bound {
            return Err(refuse("绑定来源的结构变换需要保留原始来源证明"));
        }
        let quarter = matches!(
            operation,
            HorizontalTransform::Rotate90 | HorizontalTransform::Rotate270
        );
        let out_w = if quarter { z } else { w };
        let out_z = if quarter { w } else { z };
        // Keep the original source provenance. Only dimensions and block
        // order/states rotate; no Block Entity NBT exists in the clipboard.
        let mut blocks = vec![None; self.blocks.len()];
        let mut mask = self.mask.as_ref().map(|_| vec![false; self.blocks.len()]);
        let mut rotated_biomes = self
            .biomes
            .as_ref()
            .map(|_| vec![String::new(); self.blocks.len()]);
        let mut rotated_numeric = self
            .numeric_biomes
            .as_ref()
            .map(|_| vec![0; self.blocks.len()]);
        for y in 0..h {
            for sz in 0..z {
                for sx in 0..w {
                    let (dx, dz) = match operation {
                        HorizontalTransform::Identity => (sx, sz),
                        HorizontalTransform::Rotate90 => (z - 1 - sz, sx),
                        HorizontalTransform::Rotate180 => (w - 1 - sx, z - 1 - sz),
                        HorizontalTransform::Rotate270 => (sz, w - 1 - sx),
                        HorizontalTransform::MirrorX => (w - 1 - sx, sz),
                        HorizontalTransform::MirrorZ => (sx, z - 1 - sz),
                    };
                    let original = (y * z + sz) * w + sx;
                    let destination = (y * out_z + dz) * out_w + dx;
                    let state = &self.blocks[original];
                    if let (Some(input), Some(output)) =
                        (&self.numeric_biomes, &mut rotated_numeric)
                    {
                        output[destination] = *input
                            .cells
                            .get(original)
                            .ok_or_else(|| refuse("数字 biome 变换索引越界"))?;
                    }
                    if let (Some(input), Some(output)) = (&self.mask, &mut mask) {
                        output[destination] = *input
                            .get(original)
                            .ok_or_else(|| refuse("结构 mask 长度不匹配"))?;
                    }
                    if let (Some(biomes), Some(output)) = (&self.biomes, &mut rotated_biomes) {
                        let source_index = if biomes.resolution == 1 {
                            original
                        } else if biomes.resolution == 4 {
                            let qx = (sx + self.source_min[0].rem_euclid(4) as usize) / 4;
                            let qy = (y + self.source_min[1].rem_euclid(4) as usize) / 4;
                            let qz = (sz + self.source_min[2].rem_euclid(4) as usize) / 4;
                            (qy * usize::from(biomes.size[2]) + qz) * usize::from(biomes.size[0])
                                + qx
                        } else {
                            return Err(refuse("未知 biome 采样精度"));
                        };
                        output[destination] = biomes
                            .cells
                            .get(source_index)
                            .ok_or_else(|| refuse("biome 变换索引越界"))?
                            .clone();
                    }
                    blocks[destination] = Some(if state.name.starts_with("numeric:") {
                        reorient_numeric(
                            state,
                            operation,
                            self.data_version,
                            self.source_release.as_deref(),
                        )?
                    } else {
                        reorient(state, operation)?
                    });
                }
            }
        }
        let mut copy = self.clone();
        copy.size = [out_w as u16, h as u16, out_z as u16];
        copy.mask = mask;
        let offset_x = i64::from(self.offset[0]);
        let offset_z = i64::from(self.offset[2]);
        let (next_x, next_z) = match operation {
            HorizontalTransform::Identity => (offset_x, offset_z),
            HorizontalTransform::Rotate90 => (-offset_z - z as i64, offset_x),
            HorizontalTransform::Rotate180 => (-offset_x - w as i64, -offset_z - z as i64),
            HorizontalTransform::Rotate270 => (offset_z, -offset_x - w as i64),
            HorizontalTransform::MirrorX => (-offset_x - w as i64, offset_z),
            HorizontalTransform::MirrorZ => (offset_x, -offset_z - z as i64),
        };
        copy.offset = [
            i32::try_from(next_x).map_err(|_| refuse("变换后 X 偏移溢出"))?,
            self.offset[1],
            i32::try_from(next_z).map_err(|_| refuse("变换后 Z 偏移溢出"))?,
        ];
        if let Some(cells) = rotated_biomes {
            copy.biomes = Some(BiomeVolume {
                resolution: 1,
                size: copy.size,
                cells,
            });
        }
        if let Some(cells) = rotated_numeric {
            copy.numeric_biomes = Some(crate::numeric_biome::Volume {
                size: copy.size,
                cells,
                data_version: self
                    .numeric_biomes
                    .as_ref()
                    .expect("数字源已存在")
                    .data_version,
            });
        }
        for entry in &mut copy.scheduled_ticks {
            let [x, y, z0] = entry.position;
            let (nx, nz) = match operation {
                HorizontalTransform::Identity => (x, z0),
                HorizontalTransform::Rotate90 => (z as i32 - 1 - z0, x),
                HorizontalTransform::Rotate180 => (w as i32 - 1 - x, z as i32 - 1 - z0),
                HorizontalTransform::Rotate270 => (z0, w as i32 - 1 - x),
                HorizontalTransform::MirrorX => (w as i32 - 1 - x, z0),
                HorizontalTransform::MirrorZ => (x, z as i32 - 1 - z0),
            };
            entry.position = [nx, y, nz];
            let mut absolute = [0; 3];
            for axis in 0..3 {
                absolute[axis] = self.source_min[axis]
                    .checked_add(entry.position[axis])
                    .ok_or_else(|| refuse("变换计划更新坐标溢出"))?;
            }
            entry.compound = crate::scheduled_ticks::relocate(&entry.compound, absolute)?;
        }
        for entry in &mut copy.block_entities {
            let [x, y, z0] = entry.position;
            if x < 0 || y < 0 || z0 < 0 || x as usize >= w || y as usize >= h || z0 as usize >= z {
                return Err(refuse("BE 变换位置越界"));
            }
            let (nx, nz) = match operation {
                HorizontalTransform::Identity => (x, z0),
                HorizontalTransform::Rotate90 => (z as i32 - 1 - z0, x),
                HorizontalTransform::Rotate180 => (w as i32 - 1 - x, z as i32 - 1 - z0),
                HorizontalTransform::Rotate270 => (z0, w as i32 - 1 - x),
                HorizontalTransform::MirrorX => (w as i32 - 1 - x, z0),
                HorizontalTransform::MirrorZ => (x, z as i32 - 1 - z0),
            };
            let mut raw = vec![10, 0, 0];
            raw.extend_from_slice(&entry.compound);
            let tag = crate::sections::parse_root(&raw).map_err(|e| refuse(e.to_string()))?;
            let id = tag
                .field("id")
                .ok_or_else(|| refuse("BE 变换缺少 ID"))?
                .string()
                .map_err(|e| refuse(e.to_string()))?;
            if !matches!(
                id.as_str(),
                "minecraft:chest"
                    | "minecraft:trapped_chest"
                    | "minecraft:barrel"
                    | "minecraft:furnace"
                    | "minecraft:blast_furnace"
                    | "minecraft:smoker"
                    | "minecraft:hopper"
                    | "minecraft:dropper"
                    | "minecraft:dispenser"
                    | "minecraft:brewing_stand"
                    | "minecraft:enchanting_table"
                    | "minecraft:sign"
                    | "minecraft:hanging_sign"
                    | "minecraft:shulker_box"
                    | "minecraft:bed"
                    | "minecraft:banner"
                    | "minecraft:skull"
                    | "minecraft:lectern"
                    | "minecraft:jukebox"
                    | "minecraft:chiseled_bookshelf"
                    | "minecraft:decorated_pot"
            ) {
                return Err(refuse(format!(
                    "尚未核实方块实体 {id} 的旋转载荷，不能猜测"
                )));
            }
            let absolute = (
                self.source_min[0]
                    .checked_add(nx)
                    .ok_or_else(|| refuse("BE X 变换溢出"))?,
                self.source_min[1]
                    .checked_add(y)
                    .ok_or_else(|| refuse("BE Y 变换溢出"))?,
                self.source_min[2]
                    .checked_add(nz)
                    .ok_or_else(|| refuse("BE Z 变换溢出"))?,
            );
            entry.compound = crate::be_editing::relocate_compound(&entry.compound, absolute)?;
            entry.position = [nx, y, nz];
        }
        for entry in &mut copy.entities {
            let [x, y, z0] = entry.position;
            let (nx, nz) = match operation {
                HorizontalTransform::Identity => (x, z0),
                HorizontalTransform::Rotate90 => (z as f64 - z0, x),
                HorizontalTransform::Rotate180 => (w as f64 - x, z as f64 - z0),
                HorizontalTransform::Rotate270 => (z0, w as f64 - x),
                HorizontalTransform::MirrorX => (w as f64 - x, z0),
                HorizontalTransform::MirrorZ => (x, z as f64 - z0),
            };
            if !nx.is_finite()
                || !y.is_finite()
                || !nz.is_finite()
                || nx < 0.0
                || nz < 0.0
                || nx >= out_w as f64
                || nz >= out_z as f64
            {
                return Err(refuse("实体旋转后位于结构边界外，无法作为盒内实体表示"));
            }
            entry.compound = crate::entity_editing::transform_structure_entity(
                &entry.compound,
                self.source_min,
                self.size,
                operation,
                self.data_version,
                &self.painting_variants,
                &crate::entity_editing::DimensionChange::within(self.entity_dimension.as_deref()),
            )?;
            let mut transformed = vec![10, 0, 0];
            transformed.extend_from_slice(&entry.compound);
            let tag =
                crate::sections::parse_root(&transformed).map_err(|e| refuse(e.to_string()))?;
            let pos = tag.require("Pos", 9).map_err(|e| refuse(e.to_string()))?;
            entry.position = std::array::from_fn(|i| {
                f64::from_be_bytes(pos.items[i].raw.try_into().expect("实体变换已核实 Double"))
                    - f64::from(self.source_min[i])
            });
            if let Some([ax, ay, az]) = entry.block_position {
                if ax < 0 || az < 0 || ax as usize >= w || az as usize >= z {
                    return Err(refuse("实体 blockPos 锚点超出结构"));
                }
                let (tx, tz) = match operation {
                    HorizontalTransform::Identity => (ax, az),
                    HorizontalTransform::Rotate90 => (z as i32 - 1 - az, ax),
                    HorizontalTransform::Rotate180 => (w as i32 - 1 - ax, z as i32 - 1 - az),
                    HorizontalTransform::Rotate270 => (az, w as i32 - 1 - ax),
                    HorizontalTransform::MirrorX => (w as i32 - 1 - ax, az),
                    HorizontalTransform::MirrorZ => (ax, w as i32 - 1 - az),
                };
                entry.block_position = Some(
                    crate::structure_transform::hanging::block_position(
                        &tag,
                        self.source_min,
                        entry.position,
                    )?
                    .unwrap_or([tx, ay, tz]),
                );
            }
        }
        copy.blocks = blocks
            .into_iter()
            .map(|v| v.ok_or_else(|| refuse("Transformation left an unset block cell")))
            .collect::<Result<Vec<_>, _>>()?;
        crate::entity_references::relocate_clipboard(
            &self.entities,
            &mut copy.entities,
            self.data_version,
        )?;
        Ok(copy)
    }
}

#[cfg(test)]
pub(crate) fn new(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    numeric: bool,
    source_min: [i32; 3],
    size: [u16; 3],
    blocks: Vec<State>,
) -> Result<BlockClipboard, WorldError> {
    new_with_budget(
        world,
        dimension,
        numeric,
        source_min,
        size,
        blocks,
        EditWorkBudget::default(),
    )
}
pub(crate) fn new_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    numeric: bool,
    source_min: [i32; 3],
    size: [u16; 3],
    blocks: Vec<State>,
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    let clipboard = BlockClipboard {
        format: FORMAT.into(),
        version: VERSION,
        source_root: world.identity().to_string_lossy().to_string(),
        dimension: dimension.into(),
        entity_dimension: None,
        data_version: world.data_version(),
        source_release: world.release().map(str::to_owned),
        numeric,
        source_min,
        source_bound: false,
        source_file: None,
        size,
        blocks,
        mask: None,
        offset: [0; 3],
        block_entities: Vec::new(),
        scheduled_ticks: Vec::new(),
        entities: Vec::new(),
        painting_variants: Default::default(),
        biomes: None,
        numeric_biomes: None,
        metadata: None,
    };
    clipboard.validate_with_budget(world, dimension, budget)?;
    Ok(clipboard)
}
pub fn save(
    world: &impl crate::structure_context::CodecContext,
    path: &Path,
    clip: &BlockClipboard,
) -> Result<(), WorldError> {
    save_with_budget(world, path, clip, EditWorkBudget::default())
}
pub fn save_with_budget(
    world: &impl crate::structure_context::CodecContext,
    path: &Path,
    clip: &BlockClipboard,
    budget: EditWorkBudget,
) -> Result<(), WorldError> {
    if path.extension().and_then(|v| v.to_str()) != Some("mcedit2clip") {
        return Err(refuse("Clipboard must have .mcedit2clip extension"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if world.exports_into_world(parent)? {
        return Err(refuse(
            "Clipboard cannot be saved inside the original world folder",
        ));
    }
    let bytes = encode_with_budget(world, clip, budget)?;
    crate::structure_exchange::write_new(path, &bytes)
}
pub(crate) fn encode_with_budget(
    world: &impl crate::structure_context::CodecContext,
    clip: &BlockClipboard,
    budget: EditWorkBudget,
) -> Result<Vec<u8>, WorldError> {
    clip.validate_with_budget(world, &clip.dimension, budget)?;
    let bytes = serde_json::to_vec_pretty(clip)
        .map_err(|e| refuse(format!("Clipboard encode failed: {e}")))?;
    if bytes.len() > (budget.bytes() / 4) {
        return Err(refuse("Clipboard JSON exceeds 当前文件预算 limit"));
    }
    budget.check(
        bytes
            .len()
            .checked_mul(4)
            .ok_or_else(|| refuse("结构文件工作集溢出"))?,
    )?;
    Ok(bytes)
}
pub fn load(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
) -> Result<BlockClipboard, WorldError> {
    load_with_budget(world, dimension, path, EditWorkBudget::default())
}
pub fn load_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    path: &Path,
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if path.extension().and_then(|v| v.to_str()) != Some("mcedit2clip") {
        return Err(refuse("Only .mcedit2clip is accepted"));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > (budget.bytes() / 4) as u64
    {
        return Err(refuse(
            "Clipboard must be a regular file under 当前文件预算",
        ));
    }
    let bytes = crate::read_limited_file(path, (budget.bytes() / 4) as u64)?;
    let mut clipboard = decode_with_budget(world, dimension, &bytes, budget)?;
    clipboard.source_file = Some(SourceFileProof::from_bytes(path, &bytes)?);
    Ok(clipboard)
}
pub(crate) fn decode_with_budget(
    world: &impl crate::structure_context::CodecContext,
    dimension: &str,
    bytes: &[u8],
    budget: EditWorkBudget,
) -> Result<BlockClipboard, WorldError> {
    if bytes.len() > (budget.bytes() / 4) {
        return Err(refuse("Clipboard JSON exceeds 当前文件预算"));
    }
    budget.check(
        bytes
            .len()
            .checked_mul(4)
            .ok_or_else(|| refuse("结构文件工作集溢出"))?,
    )?;
    let clipboard: BlockClipboard = serde_json::from_slice(bytes)
        .map_err(|e| refuse(format!("Invalid clipboard JSON: {e}")))?;
    clipboard.validate_with_budget(world, dimension, budget)?;
    Ok(clipboard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_rotation_and_mirror_preserve_cells_and_known_facing() {
        let sample = BlockClipboard {
            format: FORMAT.into(),
            version: VERSION,
            source_root: "/original".into(),
            dimension: "minecraft:overworld".into(),
            entity_dimension: None,
            data_version: Some(4000),
            source_release: None,
            numeric: false,
            source_min: [1, 0, 2],
            source_bound: false,
            source_file: None,
            size: [2, 1, 3],
            offset: [0; 3],
            block_entities: Vec::new(),
            scheduled_ticks: Vec::new(),
            entities: Vec::new(),
            painting_variants: Default::default(),
            biomes: None,
            numeric_biomes: None,
            metadata: None,
            mask: None,
            blocks: vec![
                State {
                    name: "minecraft:oak_stairs".into(),
                    properties: vec![("facing".into(), "north".into())],
                },
                State {
                    name: "minecraft:stone".into(),
                    properties: vec![],
                },
                State {
                    name: "minecraft:stone".into(),
                    properties: vec![],
                },
                State {
                    name: "minecraft:oak_log".into(),
                    properties: vec![("axis".into(), "x".into())],
                },
                State {
                    name: "minecraft:stone".into(),
                    properties: vec![],
                },
                State {
                    name: "minecraft:stone".into(),
                    properties: vec![],
                },
            ],
        };
        let rotated = sample.transformed(HorizontalTransform::Rotate90).unwrap();
        assert_eq!(rotated.size, [3, 1, 2]);
        assert_eq!(rotated.blocks[2].properties[0].1, "east");
        assert_eq!(rotated.blocks[4].properties[0].1, "z");
        let restored = rotated.transformed(HorizontalTransform::Rotate270).unwrap();
        assert_eq!(restored.blocks, sample.blocks);
        assert_eq!(restored.size, sample.size);
        let mirror = sample.transformed(HorizontalTransform::MirrorZ).unwrap();
        let twice = mirror.transformed(HorizontalTransform::MirrorZ).unwrap();
        assert_eq!(twice.blocks, sample.blocks);
    }

    #[test]
    fn unknown_orientation_and_numeric_metadata_transformations_fail_closed() {
        let test = State {
            name: "mod:machine".into(),
            properties: vec![("custom_direction".into(), "north".into())],
        };
        assert!(reorient(&test, HorizontalTransform::Rotate90).is_err());
        let numeric = State {
            name: "numeric:301".into(),
            properties: vec![("data".into(), "3".into())],
        };
        assert!(reorient(&numeric, HorizontalTransform::MirrorX).is_err());
        assert_eq!(
            reorient(&numeric, HorizontalTransform::Identity).unwrap(),
            numeric
        );
    }
}
