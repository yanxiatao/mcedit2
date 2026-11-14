//! Pure-Rust original-world block edit TRANSACTIONS (in-memory only).
//!
//! A prepared batch reads ORIGINAL Java chunk data, then stages effective
//! states into an overlay. No function here writes level.dat, .mca, .mcr,
//! original Alpha .dat, external entity sidecars, or unknown modded NBT.
//! The future persistent writer must separately prove byte preservation,
//! atomic backups, schema safety and all-version coverage before exposure.
use crate::clipboard::{BlockClipboard, HorizontalTransform};
use crate::region_plan::NativeRegionCandidate;
use crate::region_transaction::{self, RegionWriteReceipt};
use crate::{clipboard, patch, region_plan, schematic};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::block_lookup::{parse_xyz, Lookup, ReadOnlyRegionCache};
use crate::sections::{decode_sections, Chunk, InPaletteEdit, Layout, State};
use crate::{WorldError, WorldSummary};
use sha2::{Digest, Sha256};

const MAX_HISTORY: usize = 100;
const MAX_COORD: i32 = 30_000_000;
const MIN_Y: i32 = -2048;
const MAX_Y: i32 = 4095;
/// 查看选框使用半开边界，允许零厚度平面；实际编辑仍要求正体积。
pub const SELECTION_MINIMUM: [i32; 3] = [-MAX_COORD + 1, MIN_Y, -MAX_COORD + 1];
pub const SELECTION_MAXIMUM_EXCLUSIVE: [i32; 3] = [MAX_COORD, MAX_Y + 1, MAX_COORD];

type Position = (i32, i32, i32);
type ChunkPos = (i32, i32);
#[path = "edit_compound.rs"]
mod compound;
pub use compound::{ClonePattern, PreparedTransfer, TransferKind, TransferOptions};
#[path = "biome_brush.rs"]
mod biome_brush;
pub use biome_brush::{BiomeBrush, BiomeTarget};
#[path = "brush_stroke.rs"]
mod brush_stroke;
pub use brush_stroke::{
    brush_bounds, brush_bounds_local, BrushMask, BrushPaint, BrushPath, BrushReplacement,
};
#[path = "brush_selection.rs"]
mod brush_selection;
pub use brush_selection::{BrushSelection, EditWorkBudget, EditWorkProgress};
#[path = "batch_paint.rs"]
mod batch_paint;
#[path = "batch_fill.rs"]
pub mod connected_fill;
#[path = "metadata_edits.rs"]
mod metadata_edits;
#[path = "object_editing.rs"]
pub(crate) mod object_editing;
#[path = "structure_work.rs"]
mod structure_work;
#[path = "edit_ticks.rs"]
mod ticks;
pub use object_editing::{ObjectDraft, ObjectPreview, ObjectTarget};
#[path = "nbt_find.rs"]
mod nbt_find;
pub use nbt_find::{
    NbtBatch, NbtMutation, NbtReplacement, NbtSearchHit, NbtSearchOptions, NbtSearchResults,
};
#[path = "selection_analysis.rs"]
pub mod analysis;
#[path = "block_entity_cell.rs"]
pub mod block_entity_cell;
#[path = "selection_deletion.rs"]
pub mod deletion;
#[path = "entity_ray.rs"]
pub mod entity_ray;
#[path = "selection_extent.rs"]
mod selection_extent;
pub use selection_extent::DimensionExtent;
#[path = "edit_be_checkpoint.rs"]
mod be_checkpoint;
#[path = "edit_chunk_snapshot.rs"]
mod chunk_snapshot;
#[path = "entity_relocation.rs"]
mod entity_relocation;
#[path = "edit_identity.rs"]
mod identity;
pub use identity::IdentityPeer;
#[path = "edit_completion.rs"]
mod completion;
#[path = "edit_display.rs"]
mod display;
#[path = "edit_lifecycle.rs"]
mod lifecycle;
#[path = "block_palette.rs"]
pub mod palette;
#[path = "edit_region_history.rs"]
mod region_history;
#[path = "block_replacement.rs"]
pub mod replacement;
#[path = "edit_savepoint.rs"]
mod savepoint;
#[path = "startup_script.rs"]
pub mod script;
#[path = "snapshot_edit.rs"]
pub mod snapshot;
#[path = "snapshot_order.rs"]
mod snapshot_order;

fn refuse(message: impl ToString) -> WorldError {
    WorldError::Invalid(message.to_string())
}
/// 跳过空气仅决定方块覆盖，不能把生物群系的真实选区裁成残缺四格。
fn structure_air(state: &State) -> bool {
    matches!(
        state.name.as_str(),
        "numeric:0" | "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}
/// 克隆载荷由剪贴板自身携带；移动载荷只用于核对旧内嵌实体所需的目标地形槽。
#[derive(Default)]
struct StructurePaste<'a> {
    skip_air: bool,
    moving_entities: &'a [clipboard::OpaqueEntity],
}

/// 剪贴板只有一个 DataVersion；原始载荷不经转换，必须保留真实来源版本。
fn require_clipboard_payload_version(
    chunk: ChunkPos,
    source: &str,
    actual: Option<i32>,
    expected: Option<i32>,
) -> Result<(), WorldError> {
    if actual != expected {
        return Err(refuse(format!(
            "区块 {chunk:?} 的 {source} 载荷版本不匹配：源 DataVersion={actual:?}，剪贴板 DataVersion={expected:?}；尚不支持原始载荷版本转换"
        )));
    }
    Ok(())
}

/// 每个实际选中载荷的区块只读取一次版本，不能用世界版本填补缺失值。
fn clipboard_payload_version(
    raw: &[u8],
    chunk: ChunkPos,
    source: &str,
) -> Result<Option<i32>, WorldError> {
    let root = crate::sections::parse_root(raw)
        .map_err(|error| refuse(format!("区块 {chunk:?} 的 {source} 载荷无效：{error}")))?;
    let mut fields = root.fields.iter().filter(|(name, _)| name == "DataVersion");
    let version = fields
        .next()
        .map(|(_, value)| value.i32())
        .transpose()
        .map_err(|error| {
            refuse(format!(
                "区块 {chunk:?} 的 {source} DataVersion 无效：{error}"
            ))
        })?;
    if fields.next().is_some() {
        return Err(refuse(format!(
            "区块 {chunk:?} 的 {source} 存在重复 DataVersion"
        )));
    }
    Ok(version)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub minimum: [i32; 3],
    pub maximum: [i32; 3],
}
impl Bounds {
    pub fn new(first: Position, second: Position) -> Result<Self, WorldError> {
        Self::spatial(first, second)
    }
    /// 几何校验不承担任务工作量限制；调用方必须接入编辑或渲染的资源预算。
    pub fn spatial(first: Position, second: Position) -> Result<Self, WorldError> {
        let a = [first.0, first.1, first.2];
        let b = [second.0, second.1, second.2];
        let mut minimum = [0; 3];
        let mut maximum = [0; 3];
        for i in 0..3 {
            minimum[i] = a[i].min(b[i]);
            maximum[i] = a[i].max(b[i]);
        }
        for i in [0, 2] {
            if minimum[i] <= -MAX_COORD || maximum[i] >= MAX_COORD {
                return Err(refuse("Selection exceeds native X/Z coordinate limits"));
            }
        }
        if minimum[1] < MIN_Y || maximum[1] > MAX_Y {
            return Err(refuse("Selection exceeds supported Y limits"));
        }
        let box_ = Self { minimum, maximum };
        box_.volume()?;
        Ok(box_)
    }
    pub fn volume(&self) -> Result<usize, WorldError> {
        let mut count: u64 = 1;
        for i in 0..3 {
            let span = (self.maximum[i] as i64 - self.minimum[i] as i64 + 1) as u64;
            count = count
                .checked_mul(span)
                .ok_or_else(|| refuse("Selection volume overflow"))?;
        }
        usize::try_from(count).map_err(|_| refuse("Selection volume overflows address size"))
    }
}
/// Exact original-coordinate 3D edit masks; no chunks or Sections generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushShape {
    HollowBox,
    Ellipsoid,
    EllipsoidShell,
    Diamond,
    ParabolicDome,
    CylinderY,
    CylinderShellY,
}
#[path = "selection_region.rs"]
mod selection_region;
pub use selection_region::{SelectionRegion, SelectionShape};
pub(crate) fn brush_inside(shape: BrushShape, bounds: Bounds, p: Position) -> bool {
    let min = bounds.minimum;
    let max = bounds.maximum;
    let (x, y, z) = p;
    if shape == BrushShape::HollowBox {
        return x == min[0]
            || x == max[0]
            || y == min[1]
            || y == max[1]
            || z == min[2]
            || z == max[2];
    }
    let norm = |index: usize, point: i32| {
        // 每个坐标表示一个体素中心，尺寸包含两端的完整格子。
        let size = max[index] as f64 - min[index] as f64 + 1.0;
        (2.0 * point as f64 - min[index] as f64 - max[index] as f64) / size
    };
    let inside = |x: i32, y: i32, z: i32| {
        if shape == BrushShape::Diamond {
            return norm(0, x).abs() + norm(1, y).abs() + norm(2, z).abs() <= 1.0;
        }
        if shape == BrushShape::ParabolicDome {
            let height = (max[1] - min[1]) as f64;
            let nx = norm(0, x);
            let nz = norm(2, z);
            let x2 = height * nx * nx;
            let z2 = height * nz * nz;
            return y > min[1] && f64::from(y - min[1]) <= height - (x2 * x2 + z2 * z2).sqrt();
        }
        let r = norm(0, x).powi(2)
            + norm(2, z).powi(2)
            + if matches!(shape, BrushShape::CylinderY | BrushShape::CylinderShellY) {
                0.0
            } else {
                norm(1, y).powi(2)
            };
        if matches!(shape, BrushShape::CylinderY | BrushShape::CylinderShellY) {
            r < 1.0
        } else {
            r <= 1.0
        }
    };
    if !inside(x, y, z) {
        return false;
    }
    if matches!(
        shape,
        BrushShape::Ellipsoid
            | BrushShape::CylinderY
            | BrushShape::Diamond
            | BrushShape::ParabolicDome
    ) {
        return true;
    }
    for (dx, dy, dz) in [
        (1, 0, 0),
        (-1, 0, 0),
        (0, 1, 0),
        (0, -1, 0),
        (0, 0, 1),
        (0, 0, -1),
    ] {
        if shape == BrushShape::CylinderShellY && dy != 0 {
            if (dy > 0 && y == max[1]) || (dy < 0 && y == min[1]) {
                return true;
            }
            continue;
        }
        let (nx, ny, nz) = (x + dx, y + dy, z + dz);
        if nx < min[0]
            || nx > max[0]
            || ny < min[1]
            || ny > max[1]
            || nz < min[2]
            || nz > max[2]
            || !inside(nx, ny, nz)
        {
            return true;
        }
    }
    false
}

/// Two endpoint triples separated by semicolon:
/// "-4, -63, 0; 4, -63, 3", allowing reversed selections.
fn parse_bound_endpoints(input: &str) -> Result<(Position, Position), WorldError> {
    let (first, second) = input
        .split_once(';')
        .ok_or_else(|| refuse("Enter X1,Y1,Z1; X2,Y2,Z2"))?;
    if second.contains(';') {
        return Err(refuse("Selection requires exactly two XYZ triples"));
    }
    Ok((parse_xyz(first)?, parse_xyz(second)?))
}
pub fn parse_bounds(input: &str) -> Result<Bounds, WorldError> {
    let (first, second) = parse_bound_endpoints(input)?;
    Bounds::new(first, second)
}
/// 大型编辑工具的坐标输入；工作量由所选择任务的 EditWorkBudget 约束。
pub fn parse_work_bounds(input: &str) -> Result<Bounds, WorldError> {
    let (first, second) = parse_bound_endpoints(input)?;
    Bounds::spatial(first, second)
}
/// 独立结构的闭区间局部范围；实际文件尺寸由文档核对，不套用世界 Y 范围。
pub fn parse_local_bounds(input: &str) -> Result<Bounds, WorldError> {
    let (first, second) = parse_bound_endpoints(input)?;
    let first = [first.0, first.1, first.2];
    let second = [second.0, second.1, second.2];
    let bounds = Bounds {
        minimum: std::array::from_fn(|axis| first[axis].min(second[axis])),
        maximum: std::array::from_fn(|axis| first[axis].max(second[axis])),
    };
    if (0..3).any(|axis| bounds.minimum[axis] < 0 || bounds.maximum[axis] >= i32::from(u16::MAX)) {
        return Err(refuse("结构局部范围须在 u16 尺寸可表示的格子内"));
    }
    bounds.volume()?;
    Ok(bounds)
}

fn registry_part(part: &str, is_path: bool) -> bool {
    !part.is_empty()
        && part.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || b == b'_'
                || b == b'-'
                || b == b'.'
                || (is_path && b == b'/')
        })
}
fn property_part(part: &str) -> bool {
    !part.is_empty()
        && part.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'_' | b'-' | b'.' | b'/' | b':')
        })
}
/// Explicit original state syntax, e.g. minecraft:oak_log[axis=y] or
/// numeric:301[data=7]. No guessed mapping of legacy modded numeric IDs.
pub fn parse_state(input: &str) -> Result<State, WorldError> {
    let input = input.trim();
    let (identity, props_text) = if let Some((name, suffix)) = input.split_once('[') {
        (
            name,
            Some(
                suffix
                    .strip_suffix(']')
                    .ok_or_else(|| refuse("Block state property list must end with ]"))?,
            ),
        )
    } else {
        (input, None)
    };
    let (namespace, path) = identity
        .split_once(':')
        .ok_or_else(|| refuse("Block state needs explicit namespace:path"))?;
    if !registry_part(namespace, false) || !registry_part(path, true) || path.contains(':') {
        return Err(refuse("Invalid or ambiguous original block identity"));
    }
    let mut properties: Vec<(String, String)> = Vec::new();
    if let Some(values) = props_text {
        if values.is_empty() {
            return Err(refuse("Empty property list is ambiguous"));
        }
        for component in values.split(',') {
            let (key, value) = component
                .split_once('=')
                .ok_or_else(|| refuse("Expected a=value in block state properties"))?;
            if !property_part(key) || !property_part(value) || value.contains('=') {
                return Err(refuse("Invalid block state property name or value"));
            }
            if properties.iter().any(|(existing, _)| existing == key) {
                return Err(refuse("Duplicate block state property name"));
            }
            properties.push((key.to_owned(), value.to_owned()));
        }
    }
    properties.sort_unstable();
    if namespace == "numeric" {
        let number = path
            .parse::<u16>()
            .map_err(|_| refuse("Original numeric block ID must be 0..4095"))?;
        if number > 4095
            || properties.len() != 1
            || properties[0].0 != "data"
            || properties[0].1.parse::<u8>().is_err()
            || properties[0].1.parse::<u8>().unwrap() > 15
        {
            return Err(refuse(
                "Numeric block identity requires numeric:ID[data=0..15]",
            ));
        }
    }
    Ok(State {
        name: identity.to_owned(),
        properties,
    })
}
pub fn render_state(state: &State) -> String {
    if state.properties.is_empty() {
        return state.name.clone();
    }
    let members: Vec<_> = state
        .properties
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    format!("{}[{}]", state.name, members.join(","))
}

#[derive(Debug, Clone)]
struct SourceCheck {
    position: Position,
    original: State,
    effective: State,
}
#[derive(Debug, Clone)]
struct Change {
    position: Position,
    original: State,
    before: State,
    after: State,
}
#[derive(Debug, Clone)]
struct EntityChange {
    chunk: ChunkPos,
    source: String,
    source_sha: String,
    before: Vec<u8>,
    after: Vec<u8>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum EntityProof {
    Delete(crate::entity_editing::Deletion),
    DeleteBatch(crate::entity_editing::DeletionBatch),
    Move(crate::entity_editing::Transfer),
    MoveToMissing(crate::entity_editing::MissingTransfer),
    Clone(crate::entity_editing::CloneEntity),
    Import(crate::entity_editing::ImportEntity),
    Edit(crate::entity_editing::Edit),
    EditBatch(crate::entity_editing::EditBatch),
    RootOrder(crate::root_order::Proof),
    Translate {
        chunk: (i32, i32),
        separate: bool,
        proof: crate::entity_editing::Translation,
    },
}
impl EntityProof {
    pub(crate) fn clone_proof(&self) -> Option<&crate::entity_editing::CloneEntity> {
        match self {
            Self::Clone(proof) => Some(proof),
            Self::Import(proof) => Some(proof.clone_proof()),
            _ => None,
        }
    }
    /// 重放中的 GlobalPos 维度必须属于实际提交维度，不能只证明 NBT 字节一致。
    pub(crate) fn verify_dimension(&self, dimension: &str) -> Result<(), WorldError> {
        if let Self::RootOrder(proof) = self {
            if !proof.is_entity() {
                return Err(refuse("实体批次使用了方块实体顺序证明"));
            }
        }
        let (change, same_source) = match self {
            Self::Translate { proof, .. } => (proof.dimensions(), true),
            Self::Move(proof) => (proof.dimensions(), true),
            Self::MoveToMissing(proof) => (proof.dimensions(), true),
            Self::Clone(proof) => (proof.dimensions(), true),
            Self::Import(proof) => (proof.dimensions(), false),
            _ => return Ok(()),
        };
        if change.target.as_deref() != Some(dimension)
            || (same_source && change.source.as_deref() != Some(dimension))
        {
            return Err(refuse("实体空间变换证明与实际提交维度不同"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct EntityPreview {
    root: PathBuf,
    dimension: String,
    identity_revision: Option<std::sync::Arc<()>>,
    changes: Vec<EntityChange>,
    proof: EntityProof,
}
impl EntityPreview {
    pub fn diagnostics(&self) -> &[String] {
        match &self.proof {
            EntityProof::Clone(proof) => proof.diagnostics(),
            EntityProof::Import(proof) => proof.diagnostics(),
            _ => &[],
        }
    }
}
#[derive(Debug, Clone)]
struct EntityGroup {
    changes: Vec<EntityChange>,
    proof: EntityProof,
}
#[derive(Debug, Clone)]
struct StructureBeChunk {
    chunk: ChunkPos,
    source_sha: String,
    before: Vec<u8>,
    after: Vec<u8>,
}
#[derive(Debug, Clone)]
struct StructureSourceBe {
    dimension: String,
    position: Position,
    terrain_sha: String,
    compound_sha: String,
    block: State,
}
#[derive(Debug, Clone)]
struct StructureSourceEntity {
    chunk: ChunkPos,
    source: String,
    source_sha: String,
    index: usize,
    compound_sha: String,
}
/// 创建目标的真实磁盘身份和可重放的候选基底。空基底不是磁盘源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationSnapshot {
    profile: crate::chunk_creation::ChunkCreationProfile,
    metadata_source: Option<crate::chunk_creation::FrozenSource>,
    chunk: ChunkPos,
    source_region_path: PathBuf,
    source_chunk_sha: Option<String>,
    source_region_sha: Option<String>,
    source_mcc_sha: Option<String>,
    source_versions: std::sync::Arc<Vec<crate::history_source::FileVersion>>,
    section_proofs: Vec<crate::chunk_creation::SectionCreationProof>,
    base: Vec<u8>,
}
impl CreationSnapshot {
    pub fn profile(&self) -> &crate::chunk_creation::ChunkCreationProfile {
        &self.profile
    }
    pub fn chunk(&self) -> ChunkPos {
        self.chunk
    }
    pub fn source_chunk_sha(&self) -> Option<&str> {
        self.source_chunk_sha.as_deref()
    }
    pub fn source_region_sha(&self) -> Option<&str> {
        self.source_region_sha.as_deref()
    }
    pub fn source_region_path(&self) -> &Path {
        &self.source_region_path
    }
    pub fn source_mcc_sha(&self) -> Option<&str> {
        self.source_mcc_sha.as_deref()
    }
    pub fn section_proofs(&self) -> &[crate::chunk_creation::SectionCreationProof] {
        &self.section_proofs
    }
    pub fn base(&self) -> &[u8] {
        &self.base
    }
    fn identity(&self) -> String {
        self.source_chunk_sha.clone().unwrap_or_else(|| {
            format!(
                "absent:{}",
                self.source_region_sha.as_deref().unwrap_or("absent-region")
            )
        })
    }
    fn rebase_payload(
        &self,
        previous: Option<&CreationSnapshot>,
        payload: &[u8],
    ) -> Result<Vec<u8>, WorldError> {
        if self.source_chunk_sha.is_none() {
            return Ok(payload.to_vec());
        }
        let prior = previous.map_or(&[][..], |snapshot| snapshot.section_proofs.as_slice());
        if !self.section_proofs.starts_with(prior) {
            return Err(refuse("创建 section 历史不是先前快照的扩展"));
        }
        let mut bytes = payload.to_vec();
        for proof in &self.section_proofs[prior.len()..] {
            bytes = crate::chunk_creation::insert_section(&bytes, proof)?;
        }
        Ok(bytes)
    }
    fn capture(
        reader: &mut ReadOnlyRegionCache<'_>,
        chunk: ChunkPos,
        profile: crate::chunk_creation::ChunkCreationProfile,
    ) -> Result<Self, WorldError> {
        let source_versions = reader.creation_source_versions(chunk.0, chunk.1)?;
        let reader = reader.snapshot_source(&source_versions)?;
        let metadata_source = reader.freeze_creation_source(&profile)?;
        let source_region_sha = reader.creation_region(chunk.0, chunk.1)?.map(str::to_owned);
        let source_region_path = reader.creation_region_image(chunk.0, chunk.1)?.0;
        let raw = reader.creation_chunk_bytes(chunk.0, chunk.1)?;
        let source_mcc_sha = reader
            .creation_sidecar_sha(chunk.0, chunk.1)?
            .map(str::to_owned);
        let source_chunk_sha = raw.as_ref().map(|bytes| nbt_sha(bytes));
        let base = match raw {
            Some(bytes) => bytes,
            None => crate::chunk_creation::empty_chunk(&profile, chunk)?,
        };
        let decoded = decode_sections(&base).map_err(|error| refuse(error.to_string()))?;
        if decoded.x != Some(chunk.0)
            || decoded.z != Some(chunk.1)
            || decoded.data_version != Some(profile.data_version)
        {
            return Err(refuse("创建目标区块身份或版本不匹配"));
        }
        Ok(Self {
            profile,
            metadata_source,
            chunk,
            source_region_path,
            source_chunk_sha,
            source_region_sha,
            source_mcc_sha,
            source_versions,
            section_proofs: Vec::new(),
            base,
        })
    }
    pub fn verify(&self, world: &WorldSummary, dimension: &str) -> Result<(), WorldError> {
        let mut reader = ReadOnlyRegionCache::new(world, dimension)?;
        self.verify_source_cached(&mut reader)
    }
    fn verify_cached(&self, reader: &mut ReadOnlyRegionCache<'_>) -> Result<(), WorldError> {
        let reader = reader.snapshot_source_declared(
            &self.source_versions,
            &[
                (
                    self.source_region_path.clone(),
                    self.source_region_sha.clone(),
                ),
                (
                    crate::region_external::sidecar_path(
                        &self.source_region_path,
                        self.chunk.0,
                        self.chunk.1,
                    )?,
                    self.source_mcc_sha.clone(),
                ),
            ],
        )?;
        self.verify_source_cached(reader)
    }
    fn verify_source_cached(&self, reader: &mut ReadOnlyRegionCache<'_>) -> Result<(), WorldError> {
        reader.verify_frozen_creation_profile(&self.profile, self.metadata_source.as_ref())?;
        if reader.creation_region_image(self.chunk.0, self.chunk.1)?.0 != self.source_region_path {
            return Err(refuse("创建目标 Region 路径已变化"));
        }
        if reader.creation_region(self.chunk.0, self.chunk.1)? != self.source_region_sha.as_deref()
        {
            return Err(refuse("创建目标 Region 存在性或摘要已变化"));
        }
        let source = reader.creation_chunk_bytes(self.chunk.0, self.chunk.1)?;
        if reader.creation_sidecar_sha(self.chunk.0, self.chunk.1)?
            != self.source_mcc_sha.as_deref()
        {
            return Err(refuse("创建目标 MCC 来源存在性或摘要已变化"));
        }
        if source.as_ref().map(|bytes| nbt_sha(bytes)) != self.source_chunk_sha {
            return Err(refuse("创建目标原区块存在性或摘要已变化"));
        }
        let mut rebuilt = match source {
            Some(bytes) => bytes,
            None => crate::chunk_creation::empty_chunk(&self.profile, self.chunk)?,
        };
        for proof in &self.section_proofs {
            rebuilt = crate::chunk_creation::insert_section(&rebuilt, proof)?;
        }
        if rebuilt != self.base {
            return Err(refuse("创建目标候选基底与来源证明不一致"));
        }
        Ok(())
    }
    fn ensure_section(
        &mut self,
        y: i32,
        profile: &crate::chunk_creation::ChunkCreationProfile,
    ) -> Result<(), WorldError> {
        if (
            profile.data_version,
            profile.min_y,
            profile.height,
            profile.has_skylight,
        ) != (
            self.profile.data_version,
            self.profile.min_y,
            self.profile.height,
            self.profile.has_skylight,
        ) {
            return Err(refuse("新增区段的维度几何与已有创建历史不同"));
        }
        let end = self
            .profile
            .min_y
            .checked_add(i32::try_from(self.profile.height).map_err(|_| refuse("维度高度溢出"))?)
            .ok_or_else(|| refuse("维度高度溢出"))?;
        if y < self.profile.min_y || y >= end {
            return Err(refuse("结构目标超出明确的维度高度"));
        }
        let decoded = decode_sections(&self.base).map_err(|error| refuse(error.to_string()))?;
        if decoded.block(0, y, 0).is_some() {
            return Ok(());
        }
        if self.source_chunk_sha.is_none() {
            return Err(refuse("新建区块的目标 section 缺失"));
        }
        let proof = crate::chunk_creation::SectionCreationProof {
            profile: profile.clone(),
            chunk: self.chunk,
            section_y: i8::try_from(y.div_euclid(16)).map_err(|_| refuse("section Y 越界"))?,
        };
        self.base = crate::chunk_creation::insert_section(&self.base, &proof)?;
        self.section_proofs.push(proof);
        Ok(())
    }
}
fn structure_payload_input(
    reader: &mut ReadOnlyRegionCache<'_>,
    chunk: ChunkPos,
    next: Option<&CreationSnapshot>,
    previous: Option<&CreationSnapshot>,
    staged: Option<&Vec<u8>>,
    staged_identity: Option<&String>,
) -> Result<StructureBeChunk, WorldError> {
    let disk = if next.is_some() {
        reader.creation_chunk_bytes(chunk.0, chunk.1)?
    } else {
        reader.batch_chunk_bytes(chunk.0, chunk.1)?
    };
    let identity = if let Some(snapshot) = next {
        snapshot.identity()
    } else {
        nbt_sha(
            disk.as_deref()
                .ok_or_else(|| refuse("结构目标区块不存在"))?,
        )
    };
    if staged.is_some() && staged_identity != Some(&identity) {
        return Err(refuse("结构目标元数据暂存源已变化"));
    }
    let base = previous
        .map(|snapshot| snapshot.base.clone())
        .or(disk)
        .or_else(|| next.map(|snapshot| snapshot.base.clone()))
        .ok_or_else(|| refuse("结构目标元数据基底不存在"))?;
    let before = staged.cloned().unwrap_or(base);
    let after = if let Some(snapshot) = next {
        snapshot.rebase_payload(previous, &before)?
    } else {
        before.clone()
    };
    Ok(StructureBeChunk {
        chunk,
        source_sha: identity,
        before,
        after,
    })
}
#[derive(Debug, Clone)]
pub struct PreparedStructure {
    work_budget: EditWorkBudget,
    root: PathBuf,
    dimension: String,
    identity_revision: Option<std::sync::Arc<()>>,
    target_sources: BTreeMap<ChunkPos, String>,
    creation_before: BTreeMap<ChunkPos, Option<CreationSnapshot>>,
    creation_after: BTreeMap<ChunkPos, CreationSnapshot>,
    source_block_entities: Vec<StructureSourceBe>,
    source_tick_sha: BTreeMap<ChunkPos, String>,
    source_entities: Vec<StructureSourceEntity>,
    /// 仅绑定到其他世界时保存真实根目录；冻结的外部结构不重新访问来源。
    source_world: Option<PathBuf>,
    source_dimension: String,
    source_blocks: Vec<SourceCheck>,
    source_terrain_sha: BTreeMap<ChunkPos, String>,
    source_creation: BTreeMap<ChunkPos, CreationSnapshot>,
    source_file: Option<clipboard::SourceFileProof>,
    source_biomes: Vec<(Position, String)>,
    source_numeric_biomes: Vec<(Position, i32)>,
    blocks: Vec<Change>,
    be_chunks: Vec<StructureBeChunk>,
    be_proofs: Vec<crate::be_editing::Structure>,
    tick_proofs: Vec<crate::scheduled_ticks::Proof>,
    biome_chunks: Vec<StructureBeChunk>,
    biome_proofs: Vec<crate::biome_editing::Proof>,
    entity_groups: Vec<EntityGroup>,
    /// 最终导入身份供同一次脚本结果中的既有引用绑定；不重新分配第二组身份。
    script_references: crate::entity_editing::ReferenceRemap,
    /// 结构文件的自由元信息保留在操作历史中，不写入世界区块。
    pub source_metadata: Option<Vec<u8>>,
    pub diagnostics: Vec<String>,
    pub overwritten_block_entities: usize,
    pub overwritten_inventory_stacks: usize,
}
impl PreparedStructure {
    /// 是否存在需要发布的方块、附加载荷或新建区块；组合操作可跳过空步骤。
    pub fn has_changes(&self) -> bool {
        !self.blocks.is_empty()
            || !self.be_proofs.is_empty()
            || !self.tick_proofs.is_empty()
            || !self.biome_proofs.is_empty()
            || !self.entity_groups.is_empty()
            || self.creation_after.iter().any(|(chunk, snapshot)| {
                self.creation_before.get(chunk).and_then(Option::as_ref) != Some(snapshot)
            })
    }
}
#[derive(Debug, Clone)]
pub struct BeNbtPreview {
    root: PathBuf,
    dimension: String,
    chunk: ChunkPos,
    source_sha: String,
    before: Vec<u8>,
    after: Vec<u8>,
    proof: BeNbtProof,
    completion: Option<crate::chunk_completion::Change>,
}
#[derive(Debug, Clone)]
enum BeNbtProof {
    Edit(crate::be_editing::Edit),
    Batch(crate::be_editing::EditBatch),
    ChunkMetadata(crate::chunk_metadata::Proof),
    RootOrder(crate::root_order::Proof),
}
impl BeNbtProof {
    fn replay(&self, source: &[u8]) -> Result<Vec<u8>, WorldError> {
        match self {
            Self::Edit(p) => p.replay(source),
            Self::Batch(p) => p.replay(source),
            Self::ChunkMetadata(p) => p.replay(source),
            Self::RootOrder(p) => p.replay_blocks(source),
        }
    }
    fn saved(&self) -> BeProof {
        match self {
            Self::Edit(p) => BeProof::Edit(p.clone()),
            Self::Batch(p) => BeProof::EditBatch(p.clone()),
            Self::ChunkMetadata(p) => BeProof::ChunkMetadata(p.clone()),
            Self::RootOrder(p) => BeProof::RootOrder(p.clone()),
        }
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BeProof {
    Scalar(region_plan::BeReplayStep),
    Edit(crate::be_editing::Edit),
    EditBatch(crate::be_editing::EditBatch),
    Structure(crate::be_editing::Structure),
    /// 共用区块附加载荷的顺序历史，不生成虚构的 BE 坐标。
    ChunkMetadata(crate::chunk_metadata::Proof),
    ScheduledTicks(crate::scheduled_ticks::Proof),
    RootOrder(crate::root_order::Proof),
}
#[derive(Debug, Clone)]
enum EditGroup {
    Deletion(std::sync::Arc<lifecycle::Step>),
    Region(std::sync::Arc<region_history::Step>),
    /// 宿主界面的可撤销步骤；仅记录顺序，不含世界载荷或源证明。
    Interface(crate::history::Entry),
    Blocks(Vec<Change>),
    Entities(EntityGroup),
    BlockEntity(ChunkPos),
    BlockEntityNbt(BeNbtPreview),
    Structure(PreparedStructure),
    Compound(Vec<EditGroup>),
}
impl EditGroup {
    fn history_entry(&self) -> crate::history::Entry {
        let (label, detail) = match self {
            Self::Deletion(step) => return step.history_entry(),
            Self::Region(step) => return step.history_entry(),
            Self::Interface(entry) => return entry.clone(),
            Self::Blocks(changes) => ("方块修改".into(), format!("{} 格", changes.len())),
            Self::Entities(group) => (
                "实体修改".into(),
                format!("{} 个实体区块", group.changes.len()),
            ),
            Self::BlockEntity((x, z)) => ("方块实体属性".into(), format!("区块 {x}, {z}")),
            Self::BlockEntityNbt(preview) => (
                match &preview.proof {
                    BeNbtProof::ChunkMetadata(_) => "区块属性",
                    BeNbtProof::RootOrder(_) => "方块实体顺序",
                    BeNbtProof::Edit(_) | BeNbtProof::Batch(_) => "方块实体 NBT",
                }
                .into(),
                format!("区块 {}, {}", preview.chunk.0, preview.chunk.1),
            ),
            Self::Structure(group) => (
                "完整结构修改".into(),
                format!(
                    "{} 格 · {} 个方块实体区块 · {} 个群系区块 · {} 组实体",
                    group.blocks.len(),
                    group.be_chunks.len(),
                    group.biome_chunks.len(),
                    group.entity_groups.len()
                ),
            ),
            Self::Compound(groups) => (
                format!("组合操作（{} 步）", groups.len()),
                groups
                    .iter()
                    .take(6)
                    .map(|group| group.history_entry().label)
                    .collect::<Vec<_>>()
                    .join("、"),
            ),
        };
        crate::history::Entry { label, detail }
    }
    /// 事务本身声明改变的来源；新增历史类型必须明确其区块影响。
    fn collect_source_chunks(&self, chunks: &mut BTreeSet<ChunkPos>) {
        let mut pending = vec![self];
        while let Some(group) = pending.pop() {
            match group {
                Self::Deletion(step) => chunks.extend(step.chunks().iter().copied()),
                Self::Region(step) => chunks.extend(step.chunks().iter().copied()),
                Self::Interface(_) => {}
                Self::Blocks(changes) => chunks.extend(
                    changes
                        .iter()
                        .map(|c| (c.position.0.div_euclid(16), c.position.2.div_euclid(16))),
                ),
                Self::Entities(group) => chunks.extend(group.changes.iter().map(|c| c.chunk)),
                Self::BlockEntity(chunk) => {
                    chunks.insert(*chunk);
                }
                Self::BlockEntityNbt(preview) => {
                    chunks.insert(preview.chunk);
                }
                Self::Structure(group) => {
                    chunks.extend(
                        group
                            .blocks
                            .iter()
                            .map(|c| (c.position.0.div_euclid(16), c.position.2.div_euclid(16))),
                    );
                    chunks.extend(
                        group
                            .be_chunks
                            .iter()
                            .chain(&group.biome_chunks)
                            .map(|c| c.chunk),
                    );
                    chunks.extend(
                        group
                            .entity_groups
                            .iter()
                            .flat_map(|g| g.changes.iter().map(|c| c.chunk)),
                    );
                    chunks.extend(
                        group
                            .creation_before
                            .keys()
                            .chain(group.creation_after.keys())
                            .copied()
                            .filter(|key| {
                                group.creation_before.get(key).and_then(Option::as_ref)
                                    != group.creation_after.get(key)
                            }),
                    );
                }
                Self::Compound(groups) => pending.extend(groups),
            }
        }
    }
}
#[derive(Debug, Clone)]
pub struct Preview {
    work_budget: EditWorkBudget,
    root: PathBuf,
    dimension: String,
    data_version: Option<i32>,
    changes: Vec<Change>,
    source_checks: Vec<SourceCheck>,
    pub is_copy: bool,
    pub is_move: bool,
    pub is_fill: bool,
    pub examined: usize,
    pub matched: usize,
    pub skipped: usize,
    pub source_chunks: usize,
}
impl Preview {
    pub fn changed(&self) -> usize {
        self.changes.len()
    }
    pub fn summary(&self) -> String {
        format!(
            "{}: {} original cells checked; {} matching filter; \
             {} staged block changes; {} unaffected; {} original chunks. \
             This is an IN-MEMORY transaction; the world is not saved.",
            if self.is_move {
                "3D block-only move"
            } else if self.is_copy {
                "3D block-only clone"
            } else if self.is_fill {
                "Connected 3D fill"
            } else {
                "3D selection"
            },
            self.examined,
            self.matched,
            self.changed(),
            self.skipped,
            self.source_chunks
        )
    }
}

/// Reuses a strictly decoded original chunk across every block in a batch.
/// Never creates absent chunks or Section data.
fn original_chunk<'a>(
    chunks: &'a mut BTreeMap<ChunkPos, Option<Chunk>>,
    world: &WorldSummary,
    dimension: &str,
    origin: Option<&crate::history_source::Archive>,
    position: Position,
) -> Result<&'a Chunk, WorldError> {
    let key = (position.0.div_euclid(16), position.2.div_euclid(16));
    if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
        e.insert(
            ReadOnlyRegionCache::for_source(world, dimension, origin)?.read_chunk(key.0, key.1)?,
        );
    }
    chunks.get(&key).and_then(Option::as_ref).ok_or_else(|| {
        refuse(format!(
            "Original Java chunk {},{} is absent; no block clone or generation allowed",
            key.0, key.1
        ))
    })
}

/// Small original-world X/Z plane, preserving exact block state identities.
#[derive(Debug, Clone)]
pub struct Slice {
    pub bounds: Bounds,
    pub source_chunks: usize,
    pub present: usize,
    pub missing_chunks: usize,
    pub missing_sections: usize,
    pub block_entities: usize,
    pub rows: Vec<Vec<SliceCell>>,
}
#[derive(Debug, Clone)]
pub enum SliceCell {
    MissingChunk,
    MissingSection,
    Present {
        state: State,
        original_block_entity: bool,
        /// 当前格来自尚未保存的新槽或 section。
        staged_creation: bool,
        /// 原格心及六邻格的保存光照；未知通道保持缺失。
        light: crate::saved_light::Samples,
    },
}
impl Slice {
    /// Stable per-slice symbols; @ always marks original Block Entities.
    pub fn text_map(&self) -> String {
        const SYMBOLS: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let mut symbols: BTreeMap<String, char> = BTreeMap::new();
        let mut out = format!(
            "X={}..{}, Y={}, Z={}..{} | chunks={}, known={}, missing chunks={}, missing Sections={}, BE={}\n",
            self.bounds.minimum[0], self.bounds.maximum[0], self.bounds.minimum[1],
            self.bounds.minimum[2], self.bounds.maximum[2], self.source_chunks,
            self.present, self.missing_chunks, self.missing_sections, self.block_entities,
        );
        for (index, row) in self.rows.iter().enumerate() {
            out.push_str(&format!("Z={:>8} |", self.bounds.minimum[2] + index as i32));
            for cell in row {
                let symbol = match cell {
                    SliceCell::MissingChunk => ' ',
                    SliceCell::MissingSection => '?',
                    SliceCell::Present {
                        original_block_entity: true,
                        ..
                    } => '@',
                    SliceCell::Present { state, .. } => {
                        let name = render_state(state);
                        if let Some(letter) = symbols.get(&name) {
                            *letter
                        } else {
                            let letter = SYMBOLS.chars().nth(symbols.len()).unwrap_or('*');
                            symbols.insert(name, letter);
                            letter
                        }
                    }
                };
                out.push(symbol);
            }
            out.push_str("|\n");
        }
        out.push_str(
            "Legend: space=no original chunk; ?=no Section; @=Block Entity; *=other states\n",
        );
        for (state, symbol) in &symbols {
            out.push_str(&format!("{symbol} = {state}\n"));
        }
        out
    }
}

/// Orthogonal vertical inspection plane, Y always runs from highest row
/// to lowest, with exactly one horizontal axis varying. Missing original
/// source cells are NOT generated as air.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAxis {
    AlongX,
    AlongZ,
}
#[derive(Debug, Clone)]
pub struct VerticalSlice {
    pub bounds: Bounds,
    pub axis: VerticalAxis,
    pub source_chunks: usize,
    pub block_entities: usize,
    pub missing_chunks: usize,
    pub missing_sections: usize,
    pub rows: Vec<Vec<SliceCell>>,
}
impl VerticalSlice {
    pub fn coordinate(&self, row: usize, column: usize) -> Option<(i32, i32, i32)> {
        let height = (self.bounds.maximum[1] - self.bounds.minimum[1] + 1) as usize;
        let width = match self.axis {
            VerticalAxis::AlongX => (self.bounds.maximum[0] - self.bounds.minimum[0] + 1) as usize,
            VerticalAxis::AlongZ => (self.bounds.maximum[2] - self.bounds.minimum[2] + 1) as usize,
        };
        if row >= height || column >= width {
            return None;
        }
        let y = self.bounds.maximum[1] - row as i32;
        match self.axis {
            VerticalAxis::AlongX => Some((
                self.bounds.minimum[0] + column as i32,
                y,
                self.bounds.minimum[2],
            )),
            VerticalAxis::AlongZ => Some((
                self.bounds.minimum[0],
                y,
                self.bounds.minimum[2] + column as i32,
            )),
        }
    }
    pub fn text_map(&self) -> String {
        let mut out = format!(
            "Vertical {:?} original plane; BE={}, missing chunks={}, missing Sections={}\n",
            self.axis, self.block_entities, self.missing_chunks, self.missing_sections
        );
        for (line, row) in self.rows.iter().enumerate() {
            out.push_str(&format!("Y={:>5} |", self.bounds.maximum[1] - line as i32));
            for cell in row {
                let glyph = match cell {
                    SliceCell::MissingChunk => ' ',
                    SliceCell::MissingSection => '?',
                    SliceCell::Present {
                        original_block_entity: true,
                        ..
                    } => '@',
                    SliceCell::Present { state, .. } => state.name.chars().last().unwrap_or('*'),
                };
                out.push(glyph);
            }
            out.push_str("|\n");
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct NativeChunkCandidate {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub edited_blocks: usize,
    pub original_nbt_sha256: String,
    pub patched_nbt: Vec<u8>,
}
/// 中间候选明确区分真实磁盘 NBT 摘要与原槽缺失。
struct TerrainChunkCandidate {
    chunk_x: i32,
    chunk_z: i32,
    edited_blocks: usize,
    source_sha: Option<String>,
    patched_nbt: Vec<u8>,
}
impl TerrainChunkCandidate {
    fn into_existing(self) -> Result<NativeChunkCandidate, WorldError> {
        Ok(NativeChunkCandidate {
            chunk_x: self.chunk_x,
            chunk_z: self.chunk_z,
            edited_blocks: self.edited_blocks,
            original_nbt_sha256: self
                .source_sha
                .ok_or_else(|| refuse("已有区块候选缺少真实源摘要"))?,
            patched_nbt: self.patched_nbt,
        })
    }
    fn into_mixed(self) -> crate::chunk_creation::TerrainCandidate {
        match self.source_sha {
            Some(original_nbt_sha256) => {
                crate::chunk_creation::TerrainCandidate::Existing(NativeChunkCandidate {
                    chunk_x: self.chunk_x,
                    chunk_z: self.chunk_z,
                    edited_blocks: self.edited_blocks,
                    original_nbt_sha256,
                    patched_nbt: self.patched_nbt,
                })
            }
            None => crate::chunk_creation::TerrainCandidate::Created {
                chunk: (self.chunk_x, self.chunk_z),
                edited_blocks: self.edited_blocks,
                patched_nbt: self.patched_nbt,
            },
        }
    }
}

/// Original-source-authenticated in-memory NBT change (never a disk write).
#[derive(Debug, Clone)]
pub struct BeScalarPreview {
    root: PathBuf,
    dimension: String,
    chunk: ChunkPos,
    source_sha: String,
    before_sha: String,
    position: Position,
    field: String,
    value: String,
    pub patched_nbt: Vec<u8>,
}
impl BeScalarPreview {
    pub fn sha256(&self) -> String {
        nbt_sha(&self.patched_nbt)
    }
}
#[derive(Debug, Clone)]
struct BeHistory {
    chunk: ChunkPos,
    source_sha: String,
    source_before: std::sync::Arc<[u8]>,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    position: Position,
    field: String,
    value: String,
}
fn nbt_sha(raw: &[u8]) -> String {
    Sha256::digest(raw)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}

/// Strict 3D original block-state transaction overlay. Nothing is saved.
#[derive(Debug, Clone)]
pub struct Session {
    save_work_budget: EditWorkBudget,
    world: WorldSummary,
    dimension: String,
    origin: Option<crate::history_source::Archive>,
    generations: BTreeMap<ChunkPos, std::sync::Arc<lifecycle::Generation>>,
    region_bases: BTreeMap<ChunkPos, region_history::Base>,
    content_hashes: BTreeMap<ChunkPos, [u8; 32]>,
    saved_hashes: std::sync::Arc<BTreeMap<ChunkPos, [u8; 32]>>,
    saved_chunks: std::sync::Arc<BTreeMap<ChunkPos, chunk_snapshot::SavedChunk>>,
    saved_be_steps: std::sync::Arc<Vec<be_checkpoint::ScalarStep>>,
    dirty_chunks: BTreeSet<ChunkPos>,
    pending_block_counts: BTreeMap<ChunkPos, usize>,
    pending_be: BTreeSet<ChunkPos>,
    pending_entity: BTreeSet<ChunkPos>,
    pending_geometry: BTreeSet<ChunkPos>,
    completion: crate::chunk_completion::State,
    revision: std::sync::Arc<()>,
    identity_local_revision: std::sync::Arc<()>,
    identity_peers: Vec<std::sync::Arc<IdentityPeer>>,
    identity_projection: Option<std::sync::Arc<IdentityPeer>>,
    display_source: crate::edit_sources::SourceRevision,
    changed_source_chunks: BTreeSet<ChunkPos>,
    block_chunk_counts: BTreeMap<ChunkPos, usize>,
    recording: Option<compound::Recording>,
    /// 私有连续候选预留原正式会话的一个历史槽；不改变来源证明的可见性。
    script_history_base: Option<usize>,
    creation: BTreeMap<ChunkPos, CreationSnapshot>,
    creation_regions: BTreeSet<ChunkPos>,
    overlay: BTreeMap<Position, State>,
    undo: Vec<EditGroup>,
    redo: Vec<EditGroup>,
    undo_ids: Vec<crate::history::StepId>,
    redo_ids: Vec<crate::history::StepId>,
    entity_overlay: BTreeMap<ChunkPos, Vec<u8>>,
    entity_sources: BTreeMap<ChunkPos, (String, String)>,
    biome_overlay: BTreeMap<ChunkPos, Vec<u8>>,
    biome_source: BTreeMap<ChunkPos, String>,
    be_overlay: BTreeMap<ChunkPos, Vec<u8>>,
    be_source: BTreeMap<ChunkPos, String>,
    be_undo: Vec<BeHistory>,
    be_redo: Vec<BeHistory>,
}
impl Session {
    pub fn set_work_budget(&mut self, budget: EditWorkBudget) {
        self.save_work_budget = budget;
    }
    pub fn work_budget(&self) -> EditWorkBudget {
        self.save_work_budget
    }
    /// 对 level.dat 明确声明但 Region 目录尚未生成的原版维度建立会话。
    /// 仅扩展内存中的世界摘要，目录创建由混合事务负责。
    pub fn new_with_creation_profile(
        mut world: WorldSummary,
        dimension: impl Into<String>,
        profile: &crate::chunk_creation::ChunkCreationProfile,
    ) -> Result<Self, WorldError> {
        let dimension = dimension.into();
        profile.verify_source(&world, &dimension)?;
        if !world.dimensions.iter().any(|entry| entry.name == dimension) {
            let summary =
                crate::chunk_creation::verified_dimension_summary(&world, &dimension, profile)?;
            world.dimensions.push(summary);
        }
        Self::new(world, dimension)
    }
    pub fn new(world: WorldSummary, dimension: impl Into<String>) -> Result<Self, WorldError> {
        let dimension = dimension.into();
        if !world.dimensions.iter().any(|d| d.name == dimension) {
            return Err(refuse(
                "Native edit session dimension is not part of selected world",
            ));
        }
        Ok(Self {
            save_work_budget: EditWorkBudget::default(),
            world,
            dimension,
            origin: None,
            generations: BTreeMap::new(),
            region_bases: BTreeMap::new(),
            content_hashes: BTreeMap::new(),
            saved_hashes: Default::default(),
            saved_chunks: Default::default(),
            saved_be_steps: Default::default(),
            dirty_chunks: BTreeSet::new(),
            pending_block_counts: BTreeMap::new(),
            pending_be: BTreeSet::new(),
            pending_entity: BTreeSet::new(),
            pending_geometry: BTreeSet::new(),
            completion: Default::default(),
            revision: std::sync::Arc::new(()),
            identity_local_revision: std::sync::Arc::new(()),
            identity_peers: Vec::new(),
            identity_projection: None,
            display_source: Default::default(),
            changed_source_chunks: BTreeSet::new(),
            block_chunk_counts: BTreeMap::new(),
            recording: None,
            script_history_base: None,
            creation: BTreeMap::new(),
            creation_regions: BTreeSet::new(),
            overlay: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            undo_ids: Vec::new(),
            redo_ids: Vec::new(),
            entity_overlay: BTreeMap::new(),
            entity_sources: BTreeMap::new(),
            biome_overlay: BTreeMap::new(),
            biome_source: BTreeMap::new(),
            be_overlay: BTreeMap::new(),
            be_source: BTreeMap::new(),
            be_undo: Vec::new(),
            be_redo: Vec::new(),
        })
    }
    pub fn original_world(&self) -> &WorldSummary {
        &self.world
    }
    /// 克隆共享同一身份；实际编辑、界面历史、撤销、重做或放弃会更换身份。
    pub fn revision(&self) -> std::sync::Arc<()> {
        std::sync::Arc::clone(&self.revision)
    }
    /// 显示只捕获区块代次；真实事务身份仍由 revision 提供。
    pub fn display_source(&self) -> crate::edit_sources::SourceRevision {
        self.display_source.clone()
    }
    /// 创建快照保留真实源身份；base 仅用于候选重放和显示。
    pub fn creation_snapshots(&self) -> impl Iterator<Item = &CreationSnapshot> {
        self.creation.values()
    }
    /// 返回逻辑几何基底及原始来源槽的存在性；光照采用显示派生数据，方块暂存由 overlay 合成。
    pub fn visible_chunk_base(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<(Option<Chunk>, bool), WorldError> {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        self.visible_chunk_base_cached(&mut reader, cx, cz)
    }
    /// 使用同一批次的 Region 缓存读取；创建快照仍须核验真实磁盘来源。
    pub fn visible_chunk_base_cached(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        cx: i32,
        cz: i32,
    ) -> Result<(Option<Chunk>, bool), WorldError> {
        let (chunk, present) = self.visible_chunk_base_shared_cached(reader, cx, cz)?;
        Ok((chunk.map(std::sync::Arc::unwrap_or_clone), present))
    }
    /// 同一基底的原始载荷；流式分析在解码前计量，并复用它读取对象列表。
    fn visible_chunk_payload_cached<'a>(
        &'a self,
        reader: &mut ReadOnlyRegionCache<'_>,
        cx: i32,
        cz: i32,
    ) -> Result<(Option<std::borrow::Cow<'a, [u8]>>, bool), WorldError> {
        reader.bind_history_source(self.origin.as_ref())?;
        if let Some(snapshot) = self.creation.get(&(cx, cz)) {
            snapshot.verify_cached(reader)?;
            return Ok((
                Some(std::borrow::Cow::Borrowed(&snapshot.base)),
                snapshot.source_chunk_sha.is_some(),
            ));
        }
        let region = (cx.div_euclid(32), cz.div_euclid(32));
        let has_created_neighbor = self.creation_regions.contains(&region);
        let raw = if has_created_neighbor {
            reader.creation_chunk_bytes(cx, cz)?
        } else {
            reader.read_chunk_bytes(cx, cz)?
        };
        let present = raw.is_some();
        Ok((raw.map(std::borrow::Cow::Owned), present))
    }
    /// 在单次批量读取中共享解码区块；暂存创建缓存按 Session 代次隔离。
    pub fn visible_chunk_base_shared_cached(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        cx: i32,
        cz: i32,
    ) -> Result<(Option<std::sync::Arc<Chunk>>, bool), WorldError> {
        reader.bind_history_source(self.origin.as_ref())?;
        let projected = self.needs_display_projection((cx, cz));
        if projected {
            if let Some((chunk, present)) = reader.get_display_chunk(cx, cz, &self.revision) {
                return Ok((Some(chunk), present));
            }
        }
        let (chunk, present) = if let Some(snapshot) = self.creation.get(&(cx, cz)) {
            snapshot.verify_cached(reader)?;
            let chunk = std::sync::Arc::new(
                decode_sections(&snapshot.base).map_err(|error| refuse(error.to_string()))?,
            );
            (Some(chunk), snapshot.source_chunk_sha.is_some())
        } else {
            let region = (cx.div_euclid(32), cz.div_euclid(32));
            let chunk = if self.creation_regions.contains(&region) {
                reader.read_creation_chunk_shared(cx, cz)?
            } else {
                reader.read_chunk_shared(cx, cz)?
            };
            let present = chunk.is_some();
            (chunk, present)
        };
        let chunk = match chunk {
            Some(chunk) if projected => {
                let current = if self.origin.is_some() {
                    reader.current_chunk_shared(cx, cz)?
                } else {
                    None
                };
                let chunk = self.project_display_chunk((cx, cz), chunk, current)?;
                reader.put_display_chunk(
                    cx,
                    cz,
                    self.revision(),
                    std::sync::Arc::clone(&chunk),
                    present,
                );
                Some(chunk)
            }
            chunk => chunk,
        };
        Ok((chunk, present))
    }
    /// 并行流式批次共享同一 Region 源；创建快照须与该源的全部身份一致。
    pub fn visible_chunk_base_parallel(
        &self,
        shared: &crate::block_lookup::SharedRegionBatch,
        reader: &mut ReadOnlyRegionCache<'_>,
        cx: i32,
        cz: i32,
    ) -> Result<(Option<std::sync::Arc<Chunk>>, bool), WorldError> {
        reader.bind_history_source(self.origin.as_ref())?;
        shared.verify_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let projected = self.needs_display_projection((cx, cz));
        if projected {
            if let Some((chunk, present)) = reader.get_display_chunk(cx, cz, &self.revision) {
                return Ok((Some(chunk), present));
            }
        }
        let (chunk, present) = if let Some(snapshot) = self.creation.get(&(cx, cz)) {
            reader.verify_frozen_creation_profile(
                &snapshot.profile,
                snapshot.metadata_source.as_ref(),
            )?;
            let (path, region_sha, chunk_sha, mcc_sha) = shared.chunk_identity(cx, cz)?;
            if path != snapshot.source_region_path
                || region_sha != snapshot.source_region_sha
                || chunk_sha != snapshot.source_chunk_sha
                || mcc_sha != snapshot.source_mcc_sha
            {
                return Err(refuse("创建快照与并行读取的真实来源身份不一致"));
            }
            let chunk = std::sync::Arc::new(
                decode_sections(&snapshot.base).map_err(|error| refuse(error.to_string()))?,
            );
            (Some(chunk), snapshot.source_chunk_sha.is_some())
        } else {
            let chunk = shared.read_chunk(cx, cz)?;
            let present = chunk.is_some();
            (chunk, present)
        };
        let chunk = match chunk {
            Some(chunk) if projected => {
                let current = if self.origin.is_some() {
                    shared.current_chunk(cx, cz)?
                } else {
                    None
                };
                let chunk = self.project_display_chunk((cx, cz), chunk, current)?;
                reader.put_display_chunk(
                    cx,
                    cz,
                    self.revision(),
                    std::sync::Arc::clone(&chunk),
                    present,
                );
                Some(chunk)
            }
            chunk => chunk,
        };
        Ok((chunk, present))
    }
    /// 指出指定高度是否来自尚未保存的新槽或新 section。
    pub fn creation_origin(&self, cx: i32, cz: i32, y: i32) -> bool {
        self.unsaved_creation((cx, cz), y)
    }
    fn note_change(&mut self) {
        self.completion.prune();
        self.revision = std::sync::Arc::new(());
        self.identity_local_revision = std::sync::Arc::clone(&self.revision);
        self.identity_projection = None;
        let chunks = std::mem::take(&mut self.changed_source_chunks);
        self.update_content_hashes(&chunks);
        let active = chunks
            .into_iter()
            .map(|chunk| {
                let present = self.block_chunk_counts.contains_key(&chunk)
                    || self.generations.contains_key(&chunk)
                    || self.region_bases.contains_key(&chunk)
                    || self.saved_hashes.contains_key(&chunk)
                    || self.creation.contains_key(&chunk)
                    || self.be_overlay.contains_key(&chunk)
                    || self.biome_overlay.contains_key(&chunk)
                    || self.entity_overlay.contains_key(&chunk)
                    || self.completion.has_pending(chunk);
                (chunk, present)
            })
            .collect::<Vec<_>>();
        self.display_source
            .update(active.into_iter(), &self.revision);
    }
    fn rebuild_creation_regions(&mut self) {
        self.creation_regions = self
            .creation
            .keys()
            .map(|&(x, z)| (x.div_euclid(32), z.div_euclid(32)))
            .collect();
    }
    fn terrain_payload_base_cached(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        chunk: ChunkPos,
    ) -> Result<(Vec<u8>, String), WorldError> {
        reader.bind_history_source(self.origin.as_ref())?;
        if let Some(snapshot) = self.creation.get(&chunk) {
            snapshot.verify_cached(reader)?;
            return Ok((snapshot.base.clone(), snapshot.identity()));
        }
        let raw = reader
            .batch_chunk_bytes(chunk.0, chunk.1)?
            .ok_or_else(|| refuse("区块载荷原始源不存在"))?;
        let identity = nbt_sha(&raw);
        Ok((raw, identity))
    }
    fn structure_payload_base(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        group: &PreparedStructure,
        chunk: ChunkPos,
        after: bool,
    ) -> Result<(Vec<u8>, String), WorldError> {
        if let Some(snapshot) = group.creation_after.get(&chunk) {
            snapshot.verify_cached(reader)?;
            let bytes = if after {
                snapshot.base.clone()
            } else if let Some(previous) =
                group.creation_before.get(&chunk).and_then(Option::as_ref)
            {
                previous.base.clone()
            } else {
                reader
                    .creation_chunk_bytes(chunk.0, chunk.1)?
                    .unwrap_or_else(|| snapshot.base.clone())
            };
            return Ok((bytes, snapshot.identity()));
        }
        let raw = self
            .source_chunk_bytes(chunk.0, chunk.1)?
            .ok_or_else(|| refuse("结构目标源区块消失"))?;
        let identity = nbt_sha(&raw);
        Ok((raw, identity))
    }
    /// Reuse an already strictly decoded source chunk instead of reopening
    /// it for each cell in a 3D render. This never exposes the overlay for
    /// direct mutation, and original source BE guards remain authoritative.
    pub(crate) fn is_staged_block(&self, position: Position) -> bool {
        self.overlay.contains_key(&position)
    }
    pub(crate) fn effective_state(&self, position: Position, source: &State) -> State {
        self.effective_state_ref(position, source).clone()
    }
    /// 显示源 NBT 外观时叠加同一 Session 的方块暂存状态；只查内存，不重新读取磁盘。
    pub fn display_state(&self, position: Position, source: &State) -> State {
        self.effective_state(position, source)
    }
    pub(crate) fn display_change(&self, position: Position) -> Option<&State> {
        self.overlay.get(&position)
    }
    /// 按 X 有序范围约束扫描，再筛同一 Z 区块；供只读显示补齐新增外观候选。
    pub(crate) fn display_changes_in_chunk(
        &self,
        cx: i32,
        cz: i32,
    ) -> impl Iterator<Item = (Position, &State)> {
        let start = (i64::from(cx) * 16).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let end = (i64::from(cx) * 16 + 15).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.overlay
            .range((start, i32::MIN, i32::MIN)..=(end, i32::MAX, i32::MAX))
            .filter(move |((x, _, z), _)| x.div_euclid(16) == cx && z.div_euclid(16) == cz)
            .map(|(position, state)| (*position, state))
    }
    /// 只读网格扫描借用调色板，只有暴露方块才复制状态，避免复制整个区块的空气与固体。
    pub(crate) fn effective_state_ref<'a>(
        &'a self,
        position: Position,
        source: &'a State,
    ) -> &'a State {
        self.overlay.get(&position).unwrap_or(source)
    }
    pub(crate) fn has_block_overlay(&self, cx: i32, cz: i32) -> bool {
        self.block_chunk_counts.contains_key(&(cx, cz))
    }
    pub fn dimension(&self) -> &str {
        &self.dimension
    }
    /// Original entities are never inserted into the block-only overlay.
    pub fn inspect_entities(
        &self,
        cx: i32,
        cz: i32,
    ) -> Result<crate::entity_catalog::EntityCatalog, WorldError> {
        if let Some(raw) = self.entity_overlay.get(&(cx, cz)) {
            let (source, original) = self.entity_source((cx, cz))?;
            if self.entity_sources.get(&(cx, cz)) != Some(&(source.clone(), nbt_sha(&original))) {
                return Err(refuse("实体暂存对应的原始源已经变化"));
            }
            crate::entity_catalog::parse(raw, &source, cx, cz)
        } else {
            let (source, raw) = self.entity_source((cx, cz))?;
            crate::entity_catalog::parse(&raw, &source, cx, cz)
        }
    }
    /// 按暂存顺序导出可重放的实体证明，事务仍须验证每个源和全部目标。
    pub fn entity_proofs(&self) -> Result<Vec<EntityProof>, WorldError> {
        if self
            .leaf_groups()
            .any(|group| matches!(group, EditGroup::Deletion(_) | EditGroup::Region(_)))
        {
            return Err(refuse(
                "旧实体证明列表不承载整区块删除或 Region 来源切换，请使用完整混合保存",
            ));
        }
        let mut proofs = Vec::new();
        for group in self.leaf_groups() {
            match group {
                EditGroup::Entities(entity) => proofs.push(entity.proof.clone()),
                EditGroup::Structure(structure) => proofs.extend(
                    structure
                        .entity_groups
                        .iter()
                        .map(|entity| entity.proof.clone()),
                ),
                _ => {}
            }
        }
        Ok(proofs)
    }
    /// 在后台可取消地扫描世界身份；暂存实体先逐块核验原始来源。
    pub fn scan_entity_identities(
        &self,
        memory_budget: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<crate::entity_editing::IdentitySnapshot, WorldError> {
        self.scan_entity_identities_with_creation(&BTreeMap::new(), memory_budget, cancelled)
    }
    /// 磁盘清单保持真实缺失，创建基底只加入当前实体领域的 UUID 工作索引。
    fn scan_entity_identities_with_creation(
        &self,
        extra: &BTreeMap<ChunkPos, CreationSnapshot>,
        memory_budget: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<crate::entity_editing::IdentitySnapshot, WorldError> {
        self.scan_current_identities(extra, memory_budget, cancelled)
    }
    pub fn be_proofs(&self) -> Result<Vec<BeProof>, WorldError> {
        let mut scalar = self.be_undo.iter();
        let mut output = Vec::new();
        let starts = self.history_starts();
        let current =
            |chunk: ChunkPos, index: usize| starts.get(&chunk).is_none_or(|start| index > *start);
        for (index, group) in self.tagged_leaf_groups() {
            match group {
                EditGroup::BlockEntity(_) => {
                    let step = scalar
                        .next()
                        .ok_or_else(|| refuse("方块实体标量历史缺失"))?;
                    if current(step.chunk, index) {
                        output.push(BeProof::Scalar(region_plan::BeReplayStep {
                            xyz: [step.position.0, step.position.1, step.position.2],
                            field: step.field.clone(),
                            value: step.value.clone(),
                        }));
                    }
                }
                EditGroup::BlockEntityNbt(preview) => {
                    if current(preview.chunk, index) {
                        output.push(preview.proof.saved());
                    }
                }
                EditGroup::Structure(group) => {
                    output.extend(
                        group
                            .be_proofs
                            .iter()
                            .cloned()
                            .map(BeProof::Structure)
                            .filter(|proof| current(region_plan::be_proof_chunk(proof), index)),
                    );
                    output.extend(
                        group
                            .tick_proofs
                            .iter()
                            .cloned()
                            .map(BeProof::ScheduledTicks)
                            .filter(|proof| current(region_plan::be_proof_chunk(proof), index)),
                    );
                }
                _ => {}
            }
        }
        if scalar.next().is_some() {
            return Err(refuse("方块实体标量历史没有对应操作组"));
        }
        Ok(output)
    }
    pub fn biome_proofs(&self) -> Vec<crate::biome_editing::Proof> {
        let starts = self.history_starts();
        self.tagged_leaf_groups()
            .filter_map(|(index, group)| match group {
                EditGroup::Structure(group) => Some(group.biome_proofs.iter().cloned().filter({
                    let starts = &starts;
                    move |proof| starts.get(&proof.chunk).is_none_or(|start| index > *start)
                })),
                _ => None,
            })
            .flatten()
            .collect()
    }
    /// 提供实体暂存的真实区块载荷，调用方应将它和同一 Session 代次一起发布。
    pub fn staged_entity_chunk(&self, cx: i32, cz: i32) -> Result<Option<&[u8]>, WorldError> {
        let Some(raw) = self.entity_overlay.get(&(cx, cz)) else {
            return Ok(None);
        };
        let (source, original) = self.entity_source((cx, cz))?;
        if self.entity_sources.get(&(cx, cz)) != Some(&(source, nbt_sha(&original))) {
            return Err(refuse("实体暂存对应的原始源已经变化"));
        }
        Ok(Some(raw))
    }
    pub fn staged_biome_chunk(&self, cx: i32, cz: i32) -> Result<Option<&[u8]>, WorldError> {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        self.staged_biome_chunk_cached(&mut reader, cx, cz)
    }
    pub fn staged_biome_chunk_cached<'a>(
        &'a self,
        reader: &mut ReadOnlyRegionCache<'_>,
        cx: i32,
        cz: i32,
    ) -> Result<Option<&'a [u8]>, WorldError> {
        reader.bind_history_source(self.origin.as_ref())?;
        let Some(staged) = self.biome_overlay.get(&(cx, cz)) else {
            return Ok(None);
        };
        let identity = if let Some(snapshot) = self.creation.get(&(cx, cz)) {
            snapshot.verify_cached(reader)?;
            snapshot.identity()
        } else {
            let raw = reader
                .batch_chunk_bytes(cx, cz)?
                .ok_or_else(|| refuse("biome 原始区块不存在"))?;
            nbt_sha(&raw)
        };
        if self.biome_source.get(&(cx, cz)) != Some(&identity) {
            return Err(refuse("biome 暂存源已变化"));
        }
        Ok(Some(staged))
    }
    pub fn staged_be_chunk(&self, cx: i32, cz: i32) -> Result<Option<&[u8]>, WorldError> {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        self.staged_be_chunk_cached(&mut reader, cx, cz)
    }
    pub fn staged_be_chunk_cached<'a>(
        &'a self,
        reader: &mut ReadOnlyRegionCache<'_>,
        cx: i32,
        cz: i32,
    ) -> Result<Option<&'a [u8]>, WorldError> {
        reader.bind_history_source(self.origin.as_ref())?;
        let Some(staged) = self.be_overlay.get(&(cx, cz)) else {
            return Ok(None);
        };
        let identity = if let Some(snapshot) = self.creation.get(&(cx, cz)) {
            snapshot.verify_cached(reader)?;
            snapshot.identity()
        } else {
            let raw = reader
                .batch_chunk_bytes(cx, cz)?
                .ok_or_else(|| refuse("BE 原始区块不存在"))?;
            nbt_sha(&raw)
        };
        if self.be_source.get(&(cx, cz)) != Some(&identity) {
            return Err(refuse("BE 暂存源已变化"));
        }
        Ok(Some(staged))
    }
    fn entity_source(&self, chunk: ChunkPos) -> Result<(String, Vec<u8>), WorldError> {
        self.entity_source_with_creation(chunk, self.creation.get(&chunk))
    }
    fn entity_source_with_creation(
        &self,
        chunk: ChunkPos,
        creation: Option<&CreationSnapshot>,
    ) -> Result<(String, Vec<u8>), WorldError> {
        match crate::entity_catalog::storage_from(
            &self.world,
            &self.dimension,
            chunk.0,
            chunk.1,
            self.origin.as_ref(),
        )? {
            crate::entity_catalog::Storage::Separate(Some(data)) => Ok(data),
            crate::entity_catalog::Storage::Separate(None) => {
                if let Some(source) = crate::entity_catalog::source_from(
                    &self.world,
                    &self.dimension,
                    chunk.0,
                    chunk.1,
                    self.origin.as_ref(),
                )? {
                    return Ok(source);
                }
                let raw = crate::entity_editing::empty_entity_chunk(
                    chunk,
                    self.world
                        .data_version
                        .ok_or_else(|| refuse("世界缺少 DataVersion"))?,
                )?;
                Ok(("entity-region:missing".into(), raw))
            }
            crate::entity_catalog::Storage::Embedded => {
                if let Some(snapshot) = creation.filter(|snapshot| {
                    snapshot.source_chunk_sha.is_none() && snapshot.profile.data_version == 1343
                }) {
                    snapshot.verify_cached(&mut self.source_reader()?)?;
                    // 领域载荷固定为合同中的完整空槽；方块、光照、BE 和 biome 另行重放。
                    return Ok((
                        "legacy-main-chunk".into(),
                        crate::chunk_creation::empty_chunk(&snapshot.profile, chunk)?,
                    ));
                }
                let terrain = self.source_chunk_bytes(chunk.0, chunk.1)?;
                if terrain.is_none() && matches!(self.world.data_version, Some(3465 | 3955)) {
                    return Ok((
                        "entity-region:missing".into(),
                        crate::entity_editing::empty_entity_chunk(
                            chunk,
                            self.world
                                .data_version
                                .ok_or_else(|| refuse("世界缺少 DataVersion"))?,
                        )?,
                    ));
                }
                let terrain = terrain.ok_or_else(|| refuse("实体目标的地形区块不存在"))?;
                if matches!(self.world.data_version, Some(3465 | 3955)) {
                    let catalog = crate::entity_catalog::parse(
                        &terrain,
                        "legacy-main-chunk",
                        chunk.0,
                        chunk.1,
                    )?;
                    if catalog.entities.is_empty() {
                        return Ok((
                            "entity-region:missing".into(),
                            crate::entity_editing::empty_entity_chunk(
                                chunk,
                                self.world
                                    .data_version
                                    .ok_or_else(|| refuse("世界缺少 DataVersion"))?,
                            )?,
                        ));
                    }
                }
                Ok(("legacy-main-chunk".into(), terrain))
            }
        }
    }
    fn entity_input(&self, chunk: ChunkPos) -> Result<EntityChange, WorldError> {
        self.entity_input_with_creation(chunk, self.creation.get(&chunk))
    }
    fn entity_input_with_creation(
        &self,
        chunk: ChunkPos,
        creation: Option<&CreationSnapshot>,
    ) -> Result<EntityChange, WorldError> {
        let (source, raw) = self.entity_source_with_creation(chunk, creation)?;
        let digest = nbt_sha(&raw);
        if self.entity_overlay.contains_key(&chunk)
            && self.entity_sources.get(&chunk) != Some(&(source.clone(), digest.clone()))
        {
            return Err(refuse("实体暂存对应的原始源已经变化"));
        }
        let before = self
            .entity_overlay
            .get(&chunk)
            .cloned()
            .unwrap_or_else(|| raw.clone());
        Ok(EntityChange {
            chunk,
            source,
            source_sha: digest,
            before,
            after: Vec::new(),
        })
    }
    fn structure_entity_source(
        &self,
        entry: &clipboard::OpaqueEntity,
    ) -> Result<StructureSourceEntity, WorldError> {
        let mut raw = vec![10, 0, 0];
        raw.extend_from_slice(&entry.compound);
        let root = crate::sections::parse_root(&raw).map_err(|e| refuse(e.to_string()))?;
        let pos = root
            .field("Pos")
            .ok_or_else(|| refuse("结构源实体缺少 Pos"))?;
        if pos.kind != 9
            || pos.items.len() != 3
            || pos.items.iter().any(|v| v.kind != 6 || v.raw.len() != 8)
        {
            return Err(refuse("结构源实体 Pos 类型无效"));
        }
        let x = f64::from_be_bytes(pos.items[0].raw.try_into().expect("已校验宽度"));
        let z = f64::from_be_bytes(pos.items[2].raw.try_into().expect("已校验宽度"));
        if !x.is_finite()
            || !z.is_finite()
            || x < i32::MIN as f64 * 16.0
            || x > i32::MAX as f64 * 16.0
            || z < i32::MIN as f64 * 16.0
            || z > i32::MAX as f64 * 16.0
        {
            return Err(refuse("结构源实体坐标越界"));
        }
        let chunk = ((x / 16.0).floor() as i32, (z / 16.0).floor() as i32);
        let source = self.entity_input(chunk)?;
        let catalog =
            crate::entity_catalog::parse(&source.before, &source.source, chunk.0, chunk.1)?;
        let list_path = if source.source.starts_with("entity-region:") {
            "/Entities"
        } else {
            let root =
                crate::sections::parse_root(&source.before).map_err(|e| refuse(e.to_string()))?;
            if root.field("Level").is_some() {
                "/Level/Entities"
            } else {
                "/Entities"
            }
        };
        let mut found = None;
        for entity in catalog
            .entities
            .iter()
            .filter(|entity| entity.path.matches('/').count() == 1)
        {
            let index = entity
                .path
                .split('/')
                .nth(1)
                .and_then(|part| part.parse::<usize>().ok())
                .ok_or_else(|| refuse("结构源实体索引无效"))?;
            if crate::nbt_document::raw_compound_list_item(&source.before, list_path, index)?
                == entry.compound
            {
                if found.replace(index).is_some() {
                    return Err(refuse("结构源实体载荷不唯一"));
                }
            }
        }
        Ok(StructureSourceEntity {
            chunk,
            source: source.source,
            source_sha: source.source_sha,
            index: found.ok_or_else(|| refuse("结构源实体已变化或不存在"))?,
            compound_sha: nbt_sha(&entry.compound),
        })
    }
    fn entity_nbt_path(change: &EntityChange, path: &str) -> Result<String, WorldError> {
        let path = path.trim_start_matches('/');
        let root = crate::sections::parse_root(&change.before)
            .map_err(|error| refuse(error.to_string()))?;
        if change.source.starts_with("entity-region:")
            || root.field("Level").is_none()
            || path.starts_with("Level/")
        {
            Ok(format!("/{path}"))
        } else {
            Ok(format!("/Level/{path}"))
        }
    }
    pub fn prepare_entity_delete(
        &self,
        chunk: ChunkPos,
        path: &str,
    ) -> Result<EntityPreview, WorldError> {
        let mut change = self.entity_input(chunk)?;
        let path = Self::entity_nbt_path(&change, path)?;
        let proof = crate::entity_editing::delete(
            &change.before,
            chunk,
            change.source.starts_with("entity-region:"),
            &path,
        )?;
        change.after = proof.replay(&change.before)?;
        Ok(EntityPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            identity_revision: None,
            changes: vec![change],
            proof: EntityProof::Delete(proof),
        })
    }
    pub fn prepare_entity_edit(
        &self,
        chunk: ChunkPos,
        path: &str,
        operation: crate::nbt_document::Operation,
    ) -> Result<EntityPreview, WorldError> {
        let mut change = self.entity_input(chunk)?;
        let path = Self::entity_nbt_path(&change, path)?;
        let proof = crate::entity_editing::edit(
            &change.before,
            chunk,
            change.source.starts_with("entity-region:"),
            &path,
            operation,
        )?;
        change.after = proof.replay(&change.before)?;
        Ok(EntityPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            identity_revision: None,
            changes: vec![change],
            proof: EntityProof::Edit(proof),
        })
    }
    pub fn prepare_entity_translation(
        &self,
        chunk: ChunkPos,
        path: &str,
        delta: [i32; 3],
    ) -> Result<EntityPreview, WorldError> {
        let mut change = self.entity_input(chunk)?;
        let separate = change.source.starts_with("entity-region:");
        let path = Self::entity_nbt_path(&change, path)?;
        let proof = crate::entity_editing::translate(
            &change.before,
            &path,
            delta,
            crate::entity_editing::DimensionChange::within(Some(&self.dimension)),
        )?;
        change.after = proof.replay(&change.before)?;
        crate::entity_editing::verify_entity_chunk(
            &change.before,
            &change.after,
            &proof,
            chunk,
            separate,
        )?;
        Ok(EntityPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            identity_revision: None,
            changes: vec![change],
            proof: EntityProof::Translate {
                chunk,
                separate,
                proof,
            },
        })
    }
    pub fn prepare_entity_move(
        &self,
        from: ChunkPos,
        to: ChunkPos,
        index: usize,
        delta: [i32; 3],
    ) -> Result<EntityPreview, WorldError> {
        let mut source = self.entity_input(from)?;
        if from == to {
            let root =
                crate::sections::parse_root(&source.before).map_err(|e| refuse(e.to_string()))?;
            let list =
                if !source.source.starts_with("entity-region:") && root.field("Level").is_some() {
                    "/Level/Entities"
                } else {
                    "/Entities"
                };
            return self.prepare_entity_translation(from, &format!("{list}/{index}"), delta);
        }
        let mut target = self.entity_input(to)?;
        if target.source == "entity-region:missing" && !self.entity_overlay.contains_key(&to) {
            let proof = crate::entity_editing::transfer_to_missing(
                &source.before,
                from,
                to,
                source.source.starts_with("entity-region:"),
                index,
                delta,
                self.world
                    .data_version
                    .ok_or_else(|| refuse("世界缺少 DataVersion"))?,
                crate::entity_editing::DimensionChange::within(Some(&self.dimension)),
            )?;
            (source.after, target.after) = proof.replay(&source.before, None)?;
            return Ok(EntityPreview {
                root: self.world.root.clone(),
                dimension: self.dimension.clone(),
                identity_revision: None,
                changes: vec![source, target],
                proof: EntityProof::MoveToMissing(proof),
            });
        }
        let proof = crate::entity_editing::transfer(
            &source.before,
            &target.before,
            from,
            to,
            source.source.starts_with("entity-region:"),
            target.source.starts_with("entity-region:"),
            index,
            delta,
            crate::entity_editing::DimensionChange::within(Some(&self.dimension)),
        )?;
        (source.after, target.after) = proof.replay(&source.before, &target.before)?;
        Ok(EntityPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            identity_revision: None,
            changes: vec![source, target],
            proof: EntityProof::Move(proof),
        })
    }
    pub fn prepare_entity_clone(
        &self,
        from: ChunkPos,
        to: ChunkPos,
        index: usize,
        delta: [i32; 3],
        snapshot: &crate::entity_editing::IdentitySnapshot,
        memory_budget: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<EntityPreview, WorldError> {
        let source = self.entity_input(from)?;
        let mut target = self.entity_input(to)?;
        let missing =
            target.source == "entity-region:missing" && !self.entity_overlay.contains_key(&to);
        let target_bytes = if missing {
            None
        } else {
            Some(target.before.as_slice())
        };
        let proof = crate::entity_editing::clone_entity(
            &source.before,
            target_bytes,
            from,
            to,
            source.source.starts_with("entity-region:"),
            target.source.starts_with("entity-region:"),
            index,
            delta,
            snapshot,
            crate::entity_editing::DimensionChange::within(Some(&self.dimension)),
        )?;
        self.scan_entity_identities(memory_budget, cancelled)?
            .verify_clone(&proof)?;
        target.after = proof.replay(&source.before, target_bytes)?;
        Ok(EntityPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            identity_revision: Some(self.revision()),
            changes: vec![target],
            proof: EntityProof::Clone(proof),
        })
    }
    pub fn apply_entity_preview(&mut self, preview: EntityPreview) -> Result<(), WorldError> {
        if preview.root != self.world.root || preview.dimension != self.dimension {
            return Err(refuse("实体预览属于其他世界或维度"));
        }
        if preview
            .identity_revision
            .as_ref()
            .is_some_and(|revision| !std::sync::Arc::ptr_eq(revision, &self.revision))
        {
            return Err(refuse("实体身份预检来源已改变，请重新预检"));
        }
        self.ensure_history_slot(false)?;
        preview.proof.verify_dimension(&self.dimension)?;
        crate::entity_editing::verify_clone_scopes(
            std::iter::once(&preview.proof).filter_map(EntityProof::clone_proof),
        )?;
        self.check_entity_changes(&preview.changes, false)?;
        if let EntityProof::Clone(proof) = &preview.proof {
            let (from, _) = proof.source();
            let source = self.entity_input(from)?;
            let target = preview
                .changes
                .first()
                .ok_or_else(|| refuse("克隆目标缺失"))?;
            let (_, _, missing) = proof.target();
            let result = proof.replay(
                &source.before,
                if missing { None } else { Some(&target.before) },
            )?;
            if result != target.after {
                return Err(refuse("克隆预览重放不一致"));
            }
        }
        let group = EntityGroup {
            changes: preview.changes,
            proof: preview.proof,
        };
        for change in &group.changes {
            self.entity_overlay
                .insert(change.chunk, change.after.clone());
            self.entity_sources.insert(
                change.chunk,
                (change.source.clone(), change.source_sha.clone()),
            );
        }
        self.record_edit(EditGroup::Entities(group))?;
        self.clear_redo();
        self.note_change();
        Ok(())
    }
    fn check_entity_changes(
        &self,
        changes: &[EntityChange],
        after: bool,
    ) -> Result<(), WorldError> {
        for change in changes {
            let (source, raw) = self.entity_source(change.chunk)?;
            if source != change.source || nbt_sha(&raw) != change.source_sha {
                return Err(refuse("实体源文件已变化"));
            }
            let current = self.entity_overlay.get(&change.chunk).unwrap_or(&raw);
            let expected = if after { &change.after } else { &change.before };
            if current != expected {
                return Err(refuse("实体暂存内容与预览不一致"));
            }
        }
        Ok(())
    }
    fn verify_entity_history(&self) -> Result<(), WorldError> {
        let mut replayed = BTreeMap::new();
        let mut context = Session::new(self.world.clone(), self.dimension.clone())?;
        context.origin = self
            .origin
            .as_ref()
            .map(crate::history_source::Archive::unprojected);
        context.region_bases = self.initial_region_bases();
        if context.origin.is_some() {
            context.bind_generation_source()?;
        }
        let groups = self
            .leaf_groups()
            .flat_map(|group| match group {
                EditGroup::Entities(entity) => std::slice::from_ref(entity),
                EditGroup::Structure(structure) => structure.entity_groups.as_slice(),
                _ => &[],
            })
            .collect::<Vec<_>>();
        crate::entity_editing::verify_clone_scopes(
            groups
                .iter()
                .filter_map(|entity| entity.proof.clone_proof()),
        )?;
        // 按真实因果顺序切换来源，旧证明不能用最终删除后的命名空间校验。
        for group in self.leaf_groups() {
            match group {
                EditGroup::Deletion(step) => {
                    for chunk in step.chunks() {
                        context
                            .generations
                            .insert(*chunk, std::sync::Arc::clone(step.generation()));
                        context.creation.remove(chunk);
                        context.region_bases.remove(chunk);
                        replayed.remove(chunk);
                    }
                    context.bind_generation_source()?;
                }
                EditGroup::Region(step) => step.replay_source(&mut context, &mut replayed)?,
                EditGroup::Structure(structure) => {
                    context.creation.extend(
                        structure
                            .creation_after
                            .iter()
                            .map(|(chunk, snapshot)| (*chunk, snapshot.clone())),
                    );
                    for entity in &structure.entity_groups {
                        context.verify_entity_group(entity, &mut replayed)?;
                    }
                }
                EditGroup::Entities(entity) => {
                    context.verify_entity_group(entity, &mut replayed)?
                }
                _ => {}
            }
        }
        for (chunk, expected) in &self.entity_overlay {
            if replayed.get(chunk) != Some(expected) {
                return Err(refuse("实体暂存与重放历史不匹配"));
            }
        }
        for (chunk, raw) in &replayed {
            if !self.entity_overlay.contains_key(chunk) && context.entity_source(*chunk)?.1 != *raw
            {
                return Err(refuse("实体重放仍有未进入当前暂存的变化"));
            }
        }
        Ok(())
    }
    fn verify_entity_group(
        &self,
        entity: &EntityGroup,
        replayed: &mut BTreeMap<ChunkPos, Vec<u8>>,
    ) -> Result<(), WorldError> {
        entity.proof.verify_dimension(&self.dimension)?;
        for change in &entity.changes {
            let (label, source) = self.entity_source(change.chunk)?;
            if label != change.source || nbt_sha(&source) != change.source_sha {
                return Err(refuse("实体保存证明的源文件已变化"));
            }
            let current = replayed.entry(change.chunk).or_insert(source);
            if current != &change.before {
                return Err(refuse("实体历史输入不连续"));
            }
        }
        let output = match &entity.proof {
            EntityProof::Delete(proof) => vec![proof.replay(&entity.changes[0].before)?],
            EntityProof::DeleteBatch(proof) => vec![proof.replay(&entity.changes[0].before)?],
            EntityProof::Edit(proof) => vec![proof.replay(&entity.changes[0].before)?],
            EntityProof::EditBatch(proof) => vec![proof.replay(&entity.changes[0].before)?],
            EntityProof::RootOrder(proof) => {
                vec![proof.replay_entities(&entity.changes[0].before)?]
            }
            EntityProof::Translate { proof, .. } => {
                vec![proof.replay(&entity.changes[0].before)?]
            }
            EntityProof::Move(proof) => {
                let (source, target) =
                    proof.replay(&entity.changes[0].before, &entity.changes[1].before)?;
                vec![source, target]
            }
            EntityProof::MoveToMissing(proof) => {
                let (source, target) = proof.replay(&entity.changes[0].before, None)?;
                vec![source, target]
            }
            EntityProof::Clone(proof) => {
                let (source_chunk, _) = proof.source();
                let source = if let Some(bytes) = replayed.get(&source_chunk) {
                    bytes.clone()
                } else {
                    self.entity_source(source_chunk)?.1
                };
                let (_, _, missing) = proof.target();
                vec![proof.replay(
                    &source,
                    if missing {
                        None
                    } else {
                        Some(&entity.changes[0].before)
                    },
                )?]
            }
            EntityProof::Import(proof) => {
                let (_, _, missing) = proof.target();
                vec![proof.replay(if missing {
                    None
                } else {
                    Some(&entity.changes[0].before)
                })?]
            }
        };
        if output.len() != entity.changes.len() {
            return Err(refuse("实体历史输出数量不匹配"));
        }
        for (change, bytes) in entity.changes.iter().zip(output) {
            if bytes != change.after {
                return Err(refuse("实体历史不能精确重放"));
            }
            replayed.insert(change.chunk, bytes);
        }
        Ok(())
    }
    fn restore_entity_changes(
        &mut self,
        changes: &[EntityChange],
        after: bool,
    ) -> Result<(), WorldError> {
        let mut next = Vec::with_capacity(changes.len());
        for change in changes {
            let (source, raw) = self.entity_source(change.chunk)?;
            if source != change.source || nbt_sha(&raw) != change.source_sha {
                return Err(refuse("实体撤销或重做时源文件已变化"));
            }
            let bytes = if after { &change.after } else { &change.before };
            next.push((
                change.chunk,
                (bytes != &raw).then(|| bytes.clone()),
                (change.source.clone(), change.source_sha.clone()),
            ));
        }
        for (chunk, bytes, identity) in next {
            if let Some(bytes) = bytes {
                self.entity_overlay.insert(chunk, bytes);
                self.entity_sources.insert(chunk, identity);
            } else {
                self.entity_overlay.remove(&chunk);
                self.entity_sources.remove(&chunk);
            }
        }
        Ok(())
    }
    pub fn export_original_entities(
        &self,
        cx: i32,
        cz: i32,
        path: &Path,
    ) -> Result<usize, WorldError> {
        crate::entity_catalog::export(&self.world, &self.dimension, cx, cz, path)
    }
    pub fn verify_original_entities(&self, path: &Path) -> Result<usize, WorldError> {
        crate::entity_catalog::verify(&self.world, &self.dimension, path)
    }
    pub fn pending_cells(&self) -> usize {
        self.pending_block_counts.values().sum()
    }
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }
    /// 顺序与 history().entries 相同；不解析文本标签判断操作归属。
    pub fn history_ids(&self) -> impl DoubleEndedIterator<Item = &crate::history::StepId> {
        self.undo_ids.iter().chain(self.redo_ids.iter().rev())
    }
    pub fn history_id_at(&self, index: usize) -> Option<&crate::history::StepId> {
        self.history_ids().nth(index)
    }
    /// 将宿主的界面命令排入同一撤销顺序；不改世界、脏状态或显示来源。
    /// 宿主以返回身份保留实际界面载荷，历史被淘汰时一同释放。
    pub fn record_interface_step(
        &mut self,
        entry: crate::history::Entry,
    ) -> Result<crate::history::StepId, WorldError> {
        if self.recording.is_some() {
            return Err(refuse("界面历史不能插入领域组合操作"));
        }
        self.ensure_history_slot(false)?;
        self.record_edit(EditGroup::Interface(entry))?;
        self.revision = std::sync::Arc::new(());
        Ok(self
            .undo_ids
            .last()
            .expect("新界面步骤具有历史身份")
            .clone())
    }
    /// 界面步骤只移动历史身份，不读取区块，也不触碰暂存载荷。
    fn restore_interface_step(&mut self, redo: bool) -> bool {
        let (from, from_ids, to, to_ids) = if redo {
            (
                &mut self.redo,
                &mut self.redo_ids,
                &mut self.undo,
                &mut self.undo_ids,
            )
        } else {
            (
                &mut self.undo,
                &mut self.undo_ids,
                &mut self.redo,
                &mut self.redo_ids,
            )
        };
        if !matches!(from.last(), Some(EditGroup::Interface(_))) {
            return false;
        }
        to.push(from.pop().expect("已核实界面历史步骤"));
        to_ids.push(from_ids.pop().expect("界面历史具有稳定身份"));
        self.revision = std::sync::Arc::new(());
        true
    }
    fn clear_redo(&mut self) {
        self.redo.clear();
        self.redo_ids.clear();
        self.be_redo.clear();
    }
    /// 私有候选的重放步骤具有临时身份；正式组合完成时合并为一个新操作。
    fn queue_redo(&mut self, groups: impl IntoIterator<Item = EditGroup>) {
        for group in groups {
            self.redo.push(group);
            self.redo_ids.push(crate::history::StepId::new());
        }
    }
    pub fn history(&self) -> crate::history::Timeline {
        crate::history::Timeline {
            revision: self.revision(),
            cursor: self.undo.len(),
            entries: self
                .undo
                .iter()
                .chain(self.redo.iter().rev())
                .map(EditGroup::history_entry)
                .collect(),
        }
    }
    pub fn has_pending_changes(&self) -> bool {
        !self.dirty_chunks.is_empty()
    }
    pub fn pending_be_chunks(&self) -> usize {
        self.pending_be.len()
    }
    /// Reads actual original world NBT only when there is no staged overlay.
    /// Inspect a small original-world 3D voxel surface without writing data.
    pub fn inspect_surface(
        &self,
        x: i32,
        z: i32,
        y_min: i32,
        y_max: i32,
        width: u8,
        depth: u8,
    ) -> Result<crate::surface::SurfaceView, WorldError> {
        crate::surface::inspect_surface(self, x, z, y_min, y_max, width, depth)
    }
    pub fn inspect_exposed_volume(
        &self,
        x: i32,
        z: i32,
        y_min: i32,
        y_max: i32,
        width: u8,
        depth: u8,
    ) -> Result<crate::surface::SurfaceView, WorldError> {
        crate::surface::inspect_exposed_volume(self, x, z, y_min, y_max, width, depth)
    }
    pub fn visible_block(&self, position: Position) -> Result<Lookup, WorldError> {
        let (cx, cz) = (position.0.div_euclid(16), position.2.div_euclid(16));
        let (chunk, _) = self.visible_chunk_base(cx, cz)?;
        let Some(chunk) = chunk else {
            if self.overlay.contains_key(&position) {
                return Err(refuse("暂存方块的原始区块已消失"));
            }
            return Ok(Lookup::NoChunk);
        };
        let Some(source) = chunk.block(
            position.0.rem_euclid(16) as u8,
            position.1,
            position.2.rem_euclid(16) as u8,
        ) else {
            if self.overlay.contains_key(&position) {
                return Err(refuse("暂存方块的原始 section 已消失"));
            }
            return Ok(Lookup::NoSection);
        };
        Ok(Lookup::Present(crate::block_lookup::BlockDetails {
            state: self.effective_state(position, source),
            light: chunk.light.sample(
                position.0.rem_euclid(16) as u8,
                position.1,
                position.2.rem_euclid(16) as u8,
            ),
            light_ready: chunk.light.ready,
            chunk_x: cx,
            chunk_z: cz,
            original_data_version: chunk.data_version,
            staged_creation: self.creation_origin(cx, cz, position.1),
            layout: chunk.layout,
            block_x: position.0,
            block_y: position.1,
            block_z: position.2,
        }))
    }
    /// Inspect an ORIGINAL modded Block Entity, reporting exact field kinds
    /// and a source NBT digest without rewriting any of its contents.
    pub fn inspect_original_block_entity(
        &self,
        position: Position,
    ) -> Result<Option<crate::sections::OriginalBlockEntity>, WorldError> {
        let (cx, cz) = (position.0.div_euclid(16), position.2.div_euclid(16));
        let Some(raw) = self.source_chunk_bytes(cx, cz)? else {
            return Ok(None);
        };
        let decoded =
            decode_sections(&raw).map_err(|e| refuse(format!("Original BE chunk invalid: {e}")))?;
        if decoded.x.is_some_and(|x| x != cx) || decoded.z.is_some_and(|z| z != cz) {
            return Err(refuse("Original Block Entity chunk position mismatch"));
        }
        crate::sections::inspect_original_block_entity(&raw, position)
            .map_err(|e| refuse(format!("Original Block Entity NBT invalid: {e}")))
    }
    /// Inspect an original BE nested field tree, bounded to 256 entries
    /// and 8 Compound ancestors; actual source chunk remains unchanged.
    pub fn inspect_original_be_tree(
        &self,
        position: Position,
    ) -> Result<Vec<crate::sections::OriginalBePath>, WorldError> {
        let (cx, cz) = (position.0.div_euclid(16), position.2.div_euclid(16));
        let raw = self
            .source_chunk_bytes(cx, cz)?
            .ok_or_else(|| refuse("Original BE source chunk is absent"))?;
        let chunk =
            decode_sections(&raw).map_err(|e| refuse(format!("BE source NBT rejected: {e}")))?;
        if chunk.x.is_some_and(|x| x != cx) || chunk.z.is_some_and(|z| z != cz) {
            return Err(refuse(
                "Original BE field tree source chunk position mismatch",
            ));
        }
        crate::sections::inspect_original_be_tree(&raw, position)
            .map_err(|e| refuse(format!("BE field tree parse refused: {e}")))
    }
    /// In-memory preview of one existing signed numeric/String tag inside
    /// an original BE. This is NOT the world's disk-write or Undo path.
    pub fn prepare_original_be_scalar(
        &self,
        position: Position,
        field: &str,
        value: &str,
    ) -> Result<(usize, String), WorldError> {
        use sha2::{Digest, Sha256};
        let cx = position.0.div_euclid(16);
        let cz = position.2.div_euclid(16);
        let raw = self
            .source_chunk_bytes(cx, cz)?
            .ok_or_else(|| refuse("Original Block Entity chunk missing"))?;
        let patched =
            crate::sections::patch_original_block_entity_scalar(&raw, position, field, value)
                .map_err(|e| refuse(format!("BE scalar NBT surgery refused: {e}")))?;
        let hash = Sha256::digest(&patched)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        Ok((patched.len(), hash))
    }
    /// Prepare a bounded scalar replacement against ORIGINAL or already
    /// staged BE NBT. Unknown modded sibling bytes must remain verbatim.
    pub fn prepare_be_edit(
        &self,
        position: Position,
        field: &str,
        value: &str,
    ) -> Result<BeScalarPreview, WorldError> {
        let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (original, source_sha) = self.terrain_payload_base_cached(&mut reader, chunk)?;
        if self
            .be_source
            .get(&chunk)
            .is_some_and(|digest| digest != &source_sha)
        {
            return Err(refuse("Original chunk changed since first BE staging"));
        }
        let before = self.be_overlay.get(&chunk).unwrap_or(&original);
        let patched_nbt =
            crate::sections::patch_original_block_entity_scalar(before, position, field, value)
                .map_err(|e| refuse(format!("BE NBT surgery failed: {e}")))?;
        Ok(BeScalarPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            chunk,
            source_sha,
            before_sha: nbt_sha(before),
            position,
            field: field.into(),
            value: value.into(),
            patched_nbt,
        })
    }
    /// Atomic memory-only NBT staging with exact replay and SHA checks.
    pub fn commit_be_edit(&mut self, preview: BeScalarPreview) -> Result<(), WorldError> {
        if preview.root != self.world.root || preview.dimension != self.dimension {
            return Err(refuse("BE edit belongs to another world/dimension"));
        }
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (original, source_sha) =
            self.terrain_payload_base_cached(&mut reader, preview.chunk)?;
        if source_sha != preview.source_sha
            || self
                .be_source
                .get(&preview.chunk)
                .is_some_and(|v| v != &source_sha)
        {
            return Err(refuse("Original BE NBT changed since preview"));
        }
        let current = self.be_overlay.get(&preview.chunk).unwrap_or(&original);
        if nbt_sha(current) != preview.before_sha {
            return Err(refuse("Staged BE changed since preview"));
        }
        let verified = crate::sections::patch_original_block_entity_scalar(
            current,
            preview.position,
            &preview.field,
            &preview.value,
        )
        .map_err(|e| refuse(format!("BE replay refused: {e}")))?;
        if verified != preview.patched_nbt {
            return Err(refuse("BE candidate differs from original-byte replay"));
        }
        // Preserve full deterministic provenance rather than truncating an
        // earlier operation that later changes depend upon.
        self.check_scalar_history_budget(&preview, original.len())?;
        self.ensure_history_slot(false)?;
        let history = BeHistory {
            chunk: preview.chunk,
            source_sha: source_sha.clone(),
            source_before: self
                .be_undo
                .iter()
                .rev()
                .find(|history| {
                    history.chunk == preview.chunk
                        && history.source_before.as_ref() == original.as_slice()
                })
                .map_or_else(
                    || std::sync::Arc::from(original.as_slice()),
                    |history| std::sync::Arc::clone(&history.source_before),
                ),
            before: self.be_overlay.get(&preview.chunk).cloned(),
            after: preview.patched_nbt.clone(),
            position: preview.position,
            field: preview.field.clone(),
            value: preview.value.clone(),
        };
        self.be_overlay.insert(preview.chunk, preview.patched_nbt);
        self.be_source.insert(preview.chunk, source_sha);
        self.be_undo.push(history);
        self.be_redo.clear();
        self.record_edit(EditGroup::BlockEntity(preview.chunk))?;
        self.clear_redo();
        self.note_change();
        Ok(())
    }
    pub fn prepare_be_nbt_batch(
        &self,
        position: Position,
        operations: Vec<crate::nbt_document::Operation>,
        budget: EditWorkBudget,
    ) -> Result<BeNbtPreview, WorldError> {
        budget.check(compound::proof_bytes(&operations)?.saturating_mul(4))?;
        let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (original, source_sha) = self.terrain_payload_base_cached(&mut reader, chunk)?;
        if self.be_overlay.contains_key(&chunk) && self.be_source.get(&chunk) != Some(&source_sha) {
            return Err(refuse("方块实体暂存对应的原始源已变化"));
        }
        let before = self.be_overlay.get(&chunk).cloned().unwrap_or(original);
        budget.check(
            before
                .len()
                .saturating_mul(4)
                .saturating_add(compound::proof_bytes(&operations)?.saturating_mul(4)),
        )?;
        let proof = crate::be_editing::edit_batches(&before, position, vec![operations])?;
        let after = proof.replay(&before)?;
        budget.check(
            before
                .len()
                .saturating_add(after.len())
                .saturating_add(compound::proof_bytes(&proof)?)
                .saturating_mul(4),
        )?;
        Ok(BeNbtPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            chunk,
            source_sha,
            before,
            after,
            proof: BeNbtProof::Batch(proof),
            completion: None,
        })
    }
    fn check_be_history(&self, change: &BeHistory, expect: &[u8]) -> Result<(), WorldError> {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (_, identity) = self.terrain_payload_base_cached(&mut reader, change.chunk)?;
        if identity != change.source_sha
            || self.be_overlay.get(&change.chunk).map(Vec::as_slice) != Some(expect)
        {
            return Err(refuse("BE history refuses changed source or stale overlay"));
        }
        Ok(())
    }
    pub fn prepare_be_nbt_edit(
        &self,
        position: Position,
        operation: crate::nbt_document::Operation,
    ) -> Result<BeNbtPreview, WorldError> {
        let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (original, source_sha) = self.terrain_payload_base_cached(&mut reader, chunk)?;
        if self.be_overlay.contains_key(&chunk) && self.be_source.get(&chunk) != Some(&source_sha) {
            return Err(refuse("方块实体暂存对应的原始源已变化"));
        }
        let before = self.be_overlay.get(&chunk).cloned().unwrap_or(original);
        let proof = crate::be_editing::edit(&before, position, operation)?;
        let after = proof.replay(&before)?;
        Ok(BeNbtPreview {
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            chunk,
            source_sha,
            before,
            after,
            proof: BeNbtProof::Edit(proof),
            completion: None,
        })
    }
    fn check_be_nbt(&self, preview: &BeNbtPreview, after: bool) -> Result<(), WorldError> {
        if let Some(change) = &preview.completion {
            self.completion.check(preview.chunk, change, after)?;
        }
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (original, identity) = self.terrain_payload_base_cached(&mut reader, preview.chunk)?;
        if preview.root != self.world.root
            || preview.dimension != self.dimension
            || identity != preview.source_sha
        {
            return Err(refuse("方块实体源或世界已变化"));
        }
        let expected = if after {
            &preview.after
        } else {
            &preview.before
        };
        if self.be_overlay.get(&preview.chunk).unwrap_or(&original) != expected {
            return Err(refuse("方块实体暂存与预览不一致"));
        }
        Ok(())
    }
    pub fn apply_be_nbt_preview(&mut self, preview: BeNbtPreview) -> Result<(), WorldError> {
        self.ensure_history_slot(false)?;
        self.check_be_nbt(&preview, false)?;
        if preview.proof.replay(&preview.before)? != preview.after {
            return Err(refuse("方块实体编辑证明不匹配"));
        }
        if let Some(change) = &preview.completion {
            self.completion.apply(preview.chunk, change)?;
        }
        self.restore_be_nbt(&preview, true)?;
        self.record_edit(EditGroup::BlockEntityNbt(preview))?;
        self.clear_redo();
        self.note_change();
        Ok(())
    }
    fn restore_be_nbt(&mut self, preview: &BeNbtPreview, after: bool) -> Result<(), WorldError> {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (original, identity) = self.terrain_payload_base_cached(&mut reader, preview.chunk)?;
        if identity != preview.source_sha {
            return Err(refuse("方块实体源已变化"));
        }
        let bytes = if after {
            &preview.after
        } else {
            &preview.before
        };
        if bytes == &original {
            self.be_overlay.remove(&preview.chunk);
            self.be_source.remove(&preview.chunk);
        } else {
            self.be_overlay.insert(preview.chunk, bytes.clone());
            self.be_source
                .insert(preview.chunk, preview.source_sha.clone());
        }
        if let Some(change) = &preview.completion {
            if after && !self.completion.contains(preview.chunk) {
                // 整组脚本 / 快照首次发布也通过历史重放，不借用另一份会话的字段基线。
                let physical = self.physical_completion_source(&mut reader, preview.chunk)?;
                change.verify_physical(&physical)?;
                self.completion.apply(preview.chunk, change)?;
            }
            self.completion.restore(preview.chunk, change, after);
        }
        Ok(())
    }
    fn undo_be_step(&mut self) -> Result<bool, WorldError> {
        let Some(group) = self.be_undo.last() else {
            return Ok(false);
        };
        self.check_be_history(group, &group.after)?;
        let group = self.be_undo.pop().unwrap();
        if let Some(before) = &group.before {
            self.be_overlay.insert(group.chunk, before.clone());
        } else {
            self.be_overlay.remove(&group.chunk);
            self.be_source.remove(&group.chunk);
        }
        self.be_redo.push(group);
        Ok(true)
    }
    fn redo_be_step(&mut self) -> Result<bool, WorldError> {
        let Some(group) = self.be_redo.last() else {
            return Ok(false);
        };
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (_, identity) = self.terrain_payload_base_cached(&mut reader, group.chunk)?;
        if identity != group.source_sha
            || self.be_overlay.get(&group.chunk).cloned() != group.before
        {
            return Err(refuse("BE redo refuses changed source or stale overlay"));
        }
        let group = self.be_redo.pop().unwrap();
        self.be_overlay.insert(group.chunk, group.after.clone());
        self.be_source.insert(group.chunk, group.source_sha.clone());
        self.be_undo.push(group);
        Ok(true)
    }
    pub fn undo_be_edit(&mut self) -> Result<bool, WorldError> {
        if !matches!(
            self.undo.last(),
            Some(EditGroup::BlockEntity(_) | EditGroup::BlockEntityNbt(_))
        ) {
            return Ok(false);
        }
        self.undo()
    }
    pub fn redo_be_edit(&mut self) -> Result<bool, WorldError> {
        if !matches!(
            self.redo.last(),
            Some(EditGroup::BlockEntity(_) | EditGroup::BlockEntityNbt(_))
        ) {
            return Ok(false);
        }
        self.redo()
    }
    pub fn inspect_staged_be(
        &self,
        position: Position,
    ) -> Result<Option<crate::sections::OriginalBlockEntity>, WorldError> {
        let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
        if let Some(raw) = self.be_overlay.get(&chunk) {
            let mut reader = ReadOnlyRegionCache::for_source(
                &self.world,
                &self.dimension,
                self.origin.as_ref(),
            )?;
            let (_, identity) = self.terrain_payload_base_cached(&mut reader, chunk)?;
            if self.be_source.get(&chunk) != Some(&identity) {
                return Err(refuse(
                    "Staged BE inspector refuses changed original source",
                ));
            }
            crate::sections::inspect_original_block_entity(raw, position)
                .map_err(|e| refuse(format!("Staged NBT invalid: {e}")))
        } else {
            self.inspect_original_block_entity(position)
        }
    }
    /// 返回真实暂存区块的完整 NBT 文档及目标 BE 路径和原始源摘要。
    pub fn staged_be_document(
        &self,
        position: Position,
    ) -> Result<(crate::nbt_document::Document, String, String), WorldError> {
        let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (original, sha) = self.terrain_payload_base_cached(&mut reader, chunk)?;
        if self.be_overlay.contains_key(&chunk) && self.be_source.get(&chunk) != Some(&sha) {
            return Err(refuse("方块实体源已变化"));
        }
        let bytes = self.be_overlay.get(&chunk).unwrap_or(&original);
        let path = crate::be_editing::be_path(bytes, position)?;
        Ok((
            crate::nbt_document::Document::new(bytes.clone())?,
            path,
            sha,
        ))
    }
    /// 导出最近保存点与当前标量历史之间的检查点；摘要始终对应当前磁盘。
    pub fn export_be_journal(&self, path: &Path) -> Result<usize, WorldError> {
        self.export_be_checkpoint(path)
    }
    /// 先核对实际磁盘重放，再整体接入已有会话，不清除两侧历史。
    pub fn import_be_journal(&mut self, path: &Path) -> Result<usize, WorldError> {
        self.import_be_checkpoint(path)
    }
    /// Copy the complete original opaque Block Entity byte payload to an
    /// external, immutable .mcedit2be file. Never mutates world or overlay.
    pub fn export_original_be_snapshot(
        &self,
        position: Position,
        path: &Path,
    ) -> Result<usize, WorldError> {
        let snapshot = crate::be_snapshot::capture(&self.world, &self.dimension, position)?;
        let size = snapshot.raw_compound.len();
        crate::be_snapshot::save(&self.world, &self.dimension, &snapshot, path)?;
        Ok(size)
    }
    /// Read and verify an external opaque BE snapshot against current world
    /// source bytes. This is NOT import/paste into any world.
    pub fn verify_original_be_snapshot(
        &self,
        path: &Path,
    ) -> Result<(Position, usize), WorldError> {
        let snap = crate::be_snapshot::verify_file(&self.world, &self.dimension, path)?;
        Ok((
            (snap.xyz[0], snap.xyz[1], snap.xyz[2]),
            snap.raw_compound.len(),
        ))
    }
    /// Preview an untrusted declarative plugin operation using the EXACT
    /// same guarded batch prepare/commit/Undo logic as manual block edits.
    /// Preflight a declarative set of BE scalar surgeries atomically.
    pub fn preview_be_operation_file(&self, path: &Path) -> Result<(usize, usize), WorldError> {
        crate::be_operations::preview_file(self, path)
    }
    /// Stage the whole file or nothing; never write a Minecraft Region.
    pub fn stage_be_operation_file(&mut self, path: &Path) -> Result<usize, WorldError> {
        crate::be_operations::stage_file(self, path)
    }
    /// V2 declarative plugins may comprise several dependent edits, but the
    /// entire sequence is simulated on an isolated cloned Session and
    /// compressed into ONE source-authenticated Undo-able preview.
    pub(crate) fn prepare_plugin_sequence_with_budget(
        &self,
        operations: &[crate::plugins::PluginOperation],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<Preview, WorldError> {
        if operations.is_empty() {
            return Err(refuse("Plugin requires operations"));
        }
        let mut trial = self.clone();
        let mut merged: BTreeMap<Position, Change> = BTreeMap::new();
        let mut merged_bytes = 0usize;
        let (mut examined, mut matched, mut touched) = (0usize, 0usize, 0usize);
        for (step, op) in operations.iter().enumerate() {
            if cancelled() {
                return Err(refuse("声明式插件准备已取消"));
            }
            let preview = crate::plugins::preview_op_with_budget(
                &trial,
                op,
                budget,
                &mut cancelled,
                &mut progress,
            )?;
            if preview.changed() == 0 {
                return Err(refuse(
                    "Declarative plugin step matched no actual original block changes",
                ));
            }
            examined = examined
                .checked_add(preview.examined)
                .ok_or_else(|| refuse("Plugin examined-cell budget overflow"))?;
            matched = matched
                .checked_add(preview.matched)
                .ok_or_else(|| refuse("Plugin matched-cell budget overflow"))?;
            touched = touched
                .checked_add(preview.source_chunks)
                .ok_or_else(|| refuse("Plugin source-chunk counter overflow"))?;
            for (index, item) in preview.changes.iter().enumerate() {
                if index % 4096 == 0 && cancelled() {
                    return Err(refuse("声明式插件合并已取消"));
                }
                let prior = merged.get(&item.position).map_or(0, |change| {
                    compound::change_bytes(std::slice::from_ref(change))
                });
                let change = merged.entry(item.position).or_insert_with(|| item.clone());
                change.after = item.after.clone();
                merged_bytes = merged_bytes
                    .checked_sub(prior)
                    .and_then(|n| {
                        n.checked_add(compound::change_bytes(std::slice::from_ref(change)))
                    })
                    .ok_or_else(|| refuse("插件合并工作集溢出"))?;
                structure_work::check_work(budget, merged_bytes)?;
            }
            trial.advance_plugin_trial_cancellable(preview, &mut cancelled, &mut progress)?;
            progress(EditWorkProgress {
                stage: "完成声明式插件步骤",
                examined,
                selected: merged.len(),
                changed: merged.len(),
                slices: step + 1,
            });
        }
        if cancelled() {
            return Err(refuse("声明式插件候选发布已取消"));
        }
        let changes: Vec<Change> = merged
            .into_values()
            .filter(|c| c.before != c.after)
            .collect();
        if changes.is_empty() {
            return Err(refuse(
                "Declarative plugin finished without any effective changes",
            ));
        }
        Ok(Preview {
            work_budget: budget,
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            data_version: self.world.data_version,
            changes,
            source_checks: Vec::new(),
            is_copy: false,
            is_move: false,
            is_fill: false,
            examined,
            matched,
            skipped: 0,
            source_chunks: touched,
        })
    }
    pub fn prepare_plugin_file(&self, path: &Path) -> Result<Preview, WorldError> {
        self.prepare_plugin_file_with_budget_cancellable(
            path,
            EditWorkBudget::default(),
            || false,
            |_| {},
        )
    }
    pub fn prepare_plugin_file_with_budget_cancellable(
        &self,
        path: &Path,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
        progress: impl FnMut(EditWorkProgress),
    ) -> Result<Preview, WorldError> {
        crate::plugins::prepare_file_with_budget(self, path, budget, cancelled, progress)
    }
    /// 按区块读取真实源或已核验的创建快照，预览本身不写盘。
    /// 没有明确创建快照的缺失区块 / 区段不能编辑。
    pub fn prepare(
        &self,
        bounds: Bounds,
        replacement: &State,
        only_if: Option<&State>,
    ) -> Result<Preview, WorldError> {
        self.prepare_box_cancellable(
            bounds,
            None,
            replacement,
            only_if,
            EditWorkBudget::default(),
            || false,
            |_| {},
        )
    }
    pub fn prepare_brush(
        &self,
        bounds: Bounds,
        shape: BrushShape,
        replacement: &State,
        only_if: Option<&State>,
    ) -> Result<Preview, WorldError> {
        self.prepare_box_cancellable(
            bounds,
            Some(shape),
            replacement,
            only_if,
            EditWorkBudget::default(),
            || false,
            |_| {},
        )
    }
    /// An explicitly bounded read-only horizontal view. Missing source
    /// chunks/Sections are distinguishable from air; no files are modified.
    pub fn inspect_slice(&self, bounds: Bounds) -> Result<Slice, WorldError> {
        if bounds.minimum[1] != bounds.maximum[1] {
            return Err(refuse("X/Z slice requires one fixed Y value"));
        }
        let width = (bounds.maximum[0] - bounds.minimum[0] + 1) as usize;
        let height = (bounds.maximum[2] - bounds.minimum[2] + 1) as usize;
        clipboard::check_volume(bounds.volume()?, EditWorkBudget::default())?;
        let mut chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut be_meta: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut region_reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut rows = Vec::with_capacity(height);
        let (mut present, mut missing_chunks, mut missing_sections, mut block_entities) =
            (0, 0, 0, 0);
        let y = bounds.minimum[1];
        for z in bounds.minimum[2]..=bounds.maximum[2] {
            let mut row = Vec::with_capacity(width);
            for x in bounds.minimum[0]..=bounds.maximum[0] {
                let key = (x.div_euclid(16), z.div_euclid(16));
                if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
                    e.insert(
                        self.visible_chunk_base_cached(&mut region_reader, key.0, key.1)?
                            .0,
                    );
                    let staged = self
                        .staged_be_chunk_cached(&mut region_reader, key.0, key.1)?
                        .map(|v| decode_sections(v).map_err(|e| refuse(e.to_string())))
                        .transpose()?;
                    be_meta.insert(key, staged);
                }
                let cell = match chunks.get(&key).and_then(Option::as_ref) {
                    None => {
                        missing_chunks += 1;
                        SliceCell::MissingChunk
                    }
                    Some(chunk) => {
                        match chunk.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8) {
                            None => {
                                missing_sections += 1;
                                SliceCell::MissingSection
                            }
                            Some(original) => {
                                present += 1;
                                let is_be = be_meta
                                    .get(&key)
                                    .and_then(Option::as_ref)
                                    .unwrap_or(chunk)
                                    .has_block_entity(x, y, z);
                                if is_be {
                                    block_entities += 1;
                                }
                                SliceCell::Present {
                                    state: self.overlay.get(&(x, y, z)).unwrap_or(original).clone(),
                                    original_block_entity: is_be,
                                    staged_creation: self.creation_origin(key.0, key.1, y),
                                    light: Default::default(),
                                }
                            }
                        }
                    }
                };
                row.push(cell);
            }
            rows.push(row);
        }
        let source_chunks = chunks.len();
        Ok(Slice {
            bounds,
            source_chunks,
            present,
            missing_chunks,
            missing_sections,
            block_entities,
            rows,
        })
    }
    /// Inspect an X/Y or Z/Y vertical plane (up to 64x64). Only existing
    /// original chunks/Sections are read. No edits and no world writes.
    pub fn inspect_vertical_slice(&self, bounds: Bounds) -> Result<VerticalSlice, WorldError> {
        let x_static = bounds.minimum[0] == bounds.maximum[0];
        let z_static = bounds.minimum[2] == bounds.maximum[2];
        if !x_static && !z_static {
            return Err(refuse("Vertical slice must keep X or Z fixed"));
        }
        let axis = if z_static {
            VerticalAxis::AlongX
        } else {
            VerticalAxis::AlongZ
        };
        let width = match axis {
            VerticalAxis::AlongX => (bounds.maximum[0] - bounds.minimum[0] + 1) as usize,
            VerticalAxis::AlongZ => (bounds.maximum[2] - bounds.minimum[2] + 1) as usize,
        };
        let height = (bounds.maximum[1] - bounds.minimum[1] + 1) as usize;
        clipboard::check_volume(bounds.volume()?, EditWorkBudget::default())?;
        let mut cache: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut be_meta: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut region_reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let (mut be, mut missing_chunks, mut missing_sections) = (0, 0, 0);
        let mut rows = Vec::with_capacity(height);
        for y in (bounds.minimum[1]..=bounds.maximum[1]).rev() {
            let mut line = Vec::with_capacity(width);
            for offset in 0..width {
                let (x, z) = match axis {
                    VerticalAxis::AlongX => (bounds.minimum[0] + offset as i32, bounds.minimum[2]),
                    VerticalAxis::AlongZ => (bounds.minimum[0], bounds.minimum[2] + offset as i32),
                };
                let key = (x.div_euclid(16), z.div_euclid(16));
                if let std::collections::btree_map::Entry::Vacant(e) = cache.entry(key) {
                    e.insert(
                        self.visible_chunk_base_cached(&mut region_reader, key.0, key.1)?
                            .0,
                    );
                    let staged = self
                        .staged_be_chunk_cached(&mut region_reader, key.0, key.1)?
                        .map(|v| decode_sections(v).map_err(|e| refuse(e.to_string())))
                        .transpose()?;
                    be_meta.insert(key, staged);
                }
                let cell = match cache.get(&key).and_then(Option::as_ref) {
                    None => {
                        missing_chunks += 1;
                        SliceCell::MissingChunk
                    }
                    Some(chunk) => {
                        match chunk.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8) {
                            None => {
                                missing_sections += 1;
                                SliceCell::MissingSection
                            }
                            Some(original) => {
                                let is_be = be_meta
                                    .get(&key)
                                    .and_then(Option::as_ref)
                                    .unwrap_or(chunk)
                                    .has_block_entity(x, y, z);
                                if is_be {
                                    be += 1;
                                }
                                SliceCell::Present {
                                    state: self.overlay.get(&(x, y, z)).unwrap_or(original).clone(),
                                    original_block_entity: is_be,
                                    staged_creation: self.creation_origin(key.0, key.1, y),
                                    light: Default::default(),
                                }
                            }
                        }
                    }
                };
                line.push(cell);
            }
            rows.push(line);
        }
        let source_chunks = cache.len();
        Ok(VerticalSlice {
            bounds,
            axis,
            source_chunks,
            block_entities: be,
            missing_chunks,
            missing_sections,
            rows,
        })
    }
    /// Bounded 6-neighbor flood fill over the *effective* unsaved block
    /// states. All cells are read before preview; no BEs are mutated.
    /// Disconnected equal states are intentionally left untouched.
    pub fn prepare_connected_fill(
        &self,
        bounds: Bounds,
        seed: Position,
        replacement: &State,
    ) -> Result<Preview, WorldError> {
        self.prepare_connected_fill_cancellable(
            bounds,
            seed,
            replacement,
            EditWorkBudget::default(),
            || false,
            |_| {},
        )
    }
    /// Freeze the EFFECTIVE source states (including previous in-memory edits)
    /// and construct a block-only copy. All original BE NBT remains untouched.
    /// Source and destination are fully read and preflighted before returning
    /// a preview; overlap is safe because the source is snapshotted first.
    pub fn prepare_copy(
        &self,
        source: Bounds,
        destination_min: Position,
    ) -> Result<Preview, WorldError> {
        let volume = source.volume()?;
        clipboard::check_volume(volume, EditWorkBudget::default())?;
        let offsets = [
            source.maximum[0] - source.minimum[0],
            source.maximum[1] - source.minimum[1],
            source.maximum[2] - source.minimum[2],
        ];
        let maximum = (
            destination_min
                .0
                .checked_add(offsets[0])
                .ok_or_else(|| refuse("Clone X destination overflow"))?,
            destination_min
                .1
                .checked_add(offsets[1])
                .ok_or_else(|| refuse("Clone Y destination overflow"))?,
            destination_min
                .2
                .checked_add(offsets[2])
                .ok_or_else(|| refuse("Clone Z destination overflow"))?,
        );
        Bounds::new(destination_min, maximum)?;
        let mut chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut snapshots: Vec<(Position, State)> = Vec::with_capacity(volume);
        let mut source_checks = Vec::with_capacity(volume);
        for y in source.minimum[1]..=source.maximum[1] {
            for z in source.minimum[2]..=source.maximum[2] {
                for x in source.minimum[0]..=source.maximum[0] {
                    let position = (x, y, z);
                    let chunk = original_chunk(
                        &mut chunks,
                        &self.world,
                        &self.dimension,
                        self.origin.as_ref(),
                        position,
                    )?;
                    let original = chunk
                        .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                        .ok_or_else(|| refuse("Clone source Section is absent"))?;
                    if chunk.has_block_entity(x, y, z) {
                        return Err(refuse(format!(
                            "Clone source contains original Block Entity at {x},{y},{z}"
                        )));
                    }
                    let effective = self.overlay.get(&position).unwrap_or(original).clone();
                    source_checks.push(SourceCheck {
                        position,
                        original: original.clone(),
                        effective: effective.clone(),
                    });
                    snapshots.push((position, effective));
                }
            }
        }
        let mut changes = Vec::new();
        for (position, replacement) in snapshots {
            let x = destination_min.0 + (position.0 - source.minimum[0]);
            let y = destination_min.1 + (position.1 - source.minimum[1]);
            let z = destination_min.2 + (position.2 - source.minimum[2]);
            let target = (x, y, z);
            let chunk = original_chunk(
                &mut chunks,
                &self.world,
                &self.dimension,
                self.origin.as_ref(),
                target,
            )?;
            let original = chunk
                .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| refuse("Clone destination Section is absent"))?;
            let source_numeric = replacement.name.starts_with("numeric:");
            let target_numeric =
                matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112);
            if source_numeric != target_numeric {
                return Err(refuse(
                    "Clone source and destination use incompatible block ID layouts",
                ));
            }
            let before = self.overlay.get(&target).unwrap_or(original);
            if before != &replacement {
                if chunk.has_block_entity(x, y, z) {
                    return Err(refuse(format!(
                        "Clone destination contains original Block Entity at {x},{y},{z}"
                    )));
                }
                changes.push(Change {
                    position: target,
                    original: original.clone(),
                    before: before.clone(),
                    after: replacement,
                });
            }
        }
        Ok(Preview {
            work_budget: EditWorkBudget::default(),
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            data_version: self.world.data_version,
            changes,
            source_checks,
            is_copy: true,
            is_move: false,
            is_fill: false,
            examined: volume,
            matched: volume,
            skipped: 0,
            source_chunks: chunks.len(),
        })
    }
    /// Atomic block-only MOVE, including overlapping source and destination.
    /// Read the whole original and EFFECTIVE source first, then construct
    /// a frozen final map: source -> actual air, destination -> source state.
    /// Every original Block Entity in either area refuses the entire move.
    pub fn prepare_move(
        &self,
        source: Bounds,
        destination_min: Position,
    ) -> Result<Preview, WorldError> {
        let volume = source.volume()?;
        clipboard::check_volume(volume, EditWorkBudget::default())?;
        let offsets = [
            source.maximum[0] - source.minimum[0],
            source.maximum[1] - source.minimum[1],
            source.maximum[2] - source.minimum[2],
        ];
        let max = (
            destination_min
                .0
                .checked_add(offsets[0])
                .ok_or_else(|| refuse("Move X coordinate overflow"))?,
            destination_min
                .1
                .checked_add(offsets[1])
                .ok_or_else(|| refuse("Move Y coordinate overflow"))?,
            destination_min
                .2
                .checked_add(offsets[2])
                .ok_or_else(|| refuse("Move Z coordinate overflow"))?,
        );
        Bounds::new(destination_min, max)?;
        let mut chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut source_checks = Vec::with_capacity(volume);
        let mut snapshots = Vec::with_capacity(volume);
        let mut final_states: BTreeMap<Position, State> = BTreeMap::new();
        for y in source.minimum[1]..=source.maximum[1] {
            for z in source.minimum[2]..=source.maximum[2] {
                for x in source.minimum[0]..=source.maximum[0] {
                    let pos = (x, y, z);
                    let chunk = original_chunk(
                        &mut chunks,
                        &self.world,
                        &self.dimension,
                        self.origin.as_ref(),
                        pos,
                    )?;
                    if chunk.has_block_entity(x, y, z) {
                        return Err(refuse(
                            "Cannot move original Block Entity without preserving full opaque NBT",
                        ));
                    }
                    let original = chunk
                        .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                        .ok_or_else(|| refuse("Move source missing original Section"))?;
                    let effective = self.overlay.get(&pos).unwrap_or(original).clone();
                    let air =
                        if matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112) {
                            parse_state("numeric:0[data=0]")?
                        } else {
                            parse_state("minecraft:air")?
                        };
                    source_checks.push(SourceCheck {
                        position: pos,
                        original: original.clone(),
                        effective: effective.clone(),
                    });
                    snapshots.push((pos, effective));
                    final_states.insert(pos, air);
                }
            }
        }
        for (position, state) in snapshots {
            let target = (
                destination_min.0 + (position.0 - source.minimum[0]),
                destination_min.1 + (position.1 - source.minimum[1]),
                destination_min.2 + (position.2 - source.minimum[2]),
            );
            let target_chunk = original_chunk(
                &mut chunks,
                &self.world,
                &self.dimension,
                self.origin.as_ref(),
                target,
            )?;
            if target_chunk.has_block_entity(target.0, target.1, target.2) {
                return Err(refuse(
                    "Cannot overwrite original Block Entity with moved blocks",
                ));
            }
            let source_numeric = state.name.starts_with("numeric:");
            let dest_numeric = matches!(
                target_chunk.layout,
                Layout::McRegion128 | Layout::NumericAnvil112
            );
            if source_numeric != dest_numeric {
                return Err(refuse(
                    "Move crosses incompatible numeric/named original chunk layouts",
                ));
            }
            if target_chunk
                .block(
                    target.0.rem_euclid(16) as u8,
                    target.1,
                    target.2.rem_euclid(16) as u8,
                )
                .is_none()
            {
                return Err(refuse("Move destination original Section does not exist"));
            }
            final_states.insert(target, state);
        }
        let mut changes = Vec::new();
        for (position, after) in final_states {
            let chunk = original_chunk(
                &mut chunks,
                &self.world,
                &self.dimension,
                self.origin.as_ref(),
                position,
            )?;
            let original = chunk
                .block(
                    position.0.rem_euclid(16) as u8,
                    position.1,
                    position.2.rem_euclid(16) as u8,
                )
                .ok_or_else(|| refuse("Move affected original Section disappeared"))?;
            let before = self.overlay.get(&position).unwrap_or(original);
            if before != &after {
                changes.push(Change {
                    position,
                    original: original.clone(),
                    before: before.clone(),
                    after,
                });
            }
        }
        Ok(Preview {
            work_budget: EditWorkBudget::default(),
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            data_version: self.world.data_version,
            changes,
            source_checks,
            is_copy: false,
            is_move: true,
            is_fill: false,
            examined: volume,
            matched: volume,
            skipped: 0,
            source_chunks: chunks.len(),
        })
    }
    fn set_effective(&mut self, change: &Change, state: &State) {
        let chunk = (
            change.position.0.div_euclid(16),
            change.position.2.div_euclid(16),
        );
        if state == &change.original {
            if self.overlay.remove(&change.position).is_some() {
                let count = self
                    .block_chunk_counts
                    .get_mut(&chunk)
                    .expect("暂存方块的区块计数存在");
                *count -= 1;
                if *count == 0 {
                    self.block_chunk_counts.remove(&chunk);
                }
            }
        } else {
            if self
                .overlay
                .insert(change.position, state.clone())
                .is_none()
            {
                *self.block_chunk_counts.entry(chunk).or_default() += 1;
            }
        }
    }
    /// Re-check original disk NBT and current staged states for ALL targets
    /// before staging the first change; no partial mutations on refusal.
    pub fn commit(&mut self, prepared: Preview) -> Result<usize, WorldError> {
        self.commit_cancellable(prepared, || false, |_| {})
    }
    /// Attempt a STRICT in-memory original-byte NBT save preview for
    /// already-existing named palette states only. No files are touched.
    pub fn prepare_native_palette_candidates(
        &self,
    ) -> Result<Vec<NativeChunkCandidate>, WorldError> {
        if self
            .completion
            .coordinates()
            .any(|chunk| self.completion.has_pending(chunk))
        {
            return Err(refuse(
                "存在完成字段的显式赋值，请使用完整混合保存，不能只规划方块",
            ));
        }
        if !self.creation.is_empty() {
            return Err(refuse("创建区块或 section 必须通过混合事务保存"));
        }
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        self.prepare_palette_candidates(false, true, &mut reader, None)?
            .into_iter()
            .map(TerrainChunkCandidate::into_existing)
            .collect()
    }
    fn prepare_palette_candidates(
        &self,
        include_creation: bool,
        finalize: bool,
        reader: &mut ReadOnlyRegionCache<'_>,
        only: Option<ChunkPos>,
    ) -> Result<Vec<TerrainChunkCandidate>, WorldError> {
        if !self.entity_overlay.is_empty() {
            return Err(refuse("存在实体暂存，请使用完整混合保存，不能只规划方块"));
        }
        if !self.biome_overlay.is_empty() {
            return Err(refuse("存在 biome 暂存，拒绝只保存方块"));
        }
        if !self.be_overlay.is_empty() {
            return Err(refuse("BE NBT staged: block-only Region writer is deliberately DISABLED; undo or discard BE edits first"));
        }
        if self.overlay.is_empty() && (!include_creation || self.creation.is_empty()) {
            return Err(refuse("No staged block edits for native NBT preview"));
        }
        let authorized_be: BTreeSet<_> = self
            .be_proofs()?
            .into_iter()
            .filter_map(|proof| match proof {
                BeProof::Structure(proof) => Some(proof.position()),
                _ => None,
            })
            .collect();
        let mut grouped: BTreeMap<ChunkPos, Vec<(Position, State)>> = BTreeMap::new();
        for (&pos, target) in &self.overlay {
            if only.is_some_and(|chunk| chunk != (pos.0.div_euclid(16), pos.2.div_euclid(16))) {
                continue;
            }
            grouped
                .entry((pos.0.div_euclid(16), pos.2.div_euclid(16)))
                .or_default()
                .push((pos, target.clone()));
        }
        if include_creation {
            for &chunk in self.creation.keys() {
                if only.is_some_and(|target| target != chunk) {
                    continue;
                }
                grouped.entry(chunk).or_default();
            }
        }
        if include_creation {
            for (chunk, snapshot) in &self.creation {
                if only.is_some_and(|target| target != *chunk) {
                    continue;
                }
                snapshot.verify_cached(reader)?;
            }
        }
        let mut result = Vec::with_capacity(grouped.len());
        for ((cx, cz), positions) in grouped {
            let snapshot = if include_creation {
                self.creation.get(&(cx, cz))
            } else {
                None
            };
            let (raw, source_sha) = if let Some(snapshot) = snapshot {
                snapshot.verify_cached(reader)?;
                (snapshot.base.clone(), snapshot.source_chunk_sha.clone())
            } else {
                let raw = reader
                    .batch_chunk_bytes(cx, cz)?
                    .ok_or_else(|| refuse("Original chunk disappeared before NBT planning"))?;
                let sha = nbt_sha(&raw);
                (raw, Some(sha))
            };
            let chunk =
                decode_sections(&raw).map_err(|e| refuse(format!("Original NBT refused: {e}")))?;
            if chunk.x.is_some_and(|v| v != cx) || chunk.z.is_some_and(|v| v != cz) {
                return Err(refuse(
                    "Original chunk coordinates mismatch before NBT planning",
                ));
            }
            if chunk
                .block_entities
                .iter()
                .any(|&(x, _, z)| x.div_euclid(16) != cx || z.div_euclid(16) != cz)
            {
                return Err(refuse("Original Block Entity ownership is invalid"));
            }
            let mut edits = Vec::with_capacity(positions.len());
            for (pos, replacement) in positions {
                if chunk.has_block_entity(pos.0, pos.1, pos.2) && !authorized_be.contains(&pos) {
                    return Err(refuse("Native NBT preview would replace Block Entity"));
                }
                let original = chunk
                    .block(
                        pos.0.rem_euclid(16) as u8,
                        pos.1,
                        pos.2.rem_euclid(16) as u8,
                    )
                    .ok_or_else(|| refuse("Original NBT preview Section missing"))?;
                edits.push(InPaletteEdit {
                    local_x: pos.0.rem_euclid(16) as u8,
                    y: pos.1,
                    local_z: pos.2.rem_euclid(16) as u8,
                    original: original.clone(),
                    replacement,
                });
            }
            let modified = if edits.is_empty() {
                raw.clone()
            } else {
                match chunk.layout {
                    Layout::Named113 | Layout::Modern118 => {
                        crate::sections::patch_named_with_authorized_be(
                            &raw,
                            &edits,
                            &authorized_be,
                        )
                    }
                    Layout::McRegion128 | Layout::NumericAnvil112 => {
                        crate::sections::patch_numeric_with_authorized_be(
                            &raw,
                            &edits,
                            &authorized_be,
                        )
                    }
                }
                .map_err(|e| {
                    refuse(format!(
                        "Native original-byte preview cannot safely encode chunk {cx},{cz}: {e}"
                    ))
                })?
            };
            let modified = if edits.is_empty() || !finalize {
                modified
            } else {
                crate::chunk_postprocess::invalidate(&modified)?
            };
            result.push(TerrainChunkCandidate {
                chunk_x: cx,
                chunk_z: cz,
                edited_blocks: edits.len(),
                source_sha,
                patched_nbt: modified,
            });
        }
        Ok(result)
    }
    /// Save exactly ONE BE-containing Region using all pre-existing
    /// transactional guarantees: original SHA preflight, append-only
    /// validation, immutable backup, atomic per-file replacement and
    /// durable pending marker. No multi-Region atomic saves.
    pub fn apply_authenticated_be_single_region_save(
        &mut self,
        timestamp: u32,
        confirmation: &str,
    ) -> Result<RegionWriteReceipt, WorldError> {
        if confirmation != "SAVE_BE_NBT" {
            return Err(refuse("Exact SAVE_BE_NBT confirmation required"));
        }
        let mut plans = self.preview_be_region_candidates(timestamp)?;
        if plans.len() != 1 {
            return Err(refuse(
                "BE disk save requires exactly ONE original Region file",
            ));
        }
        let candidate = plans
            .pop()
            .ok_or_else(|| refuse("Missing verified BE Region"))?;
        let receipt = region_transaction::apply_authenticated_be_single_region(
            &self.world,
            &self.dimension,
            &candidate,
            confirmation,
        )?;
        self.discard();
        Ok(receipt)
    }
    /// Preview complete append-only MCA/MCR images for staged block edits.
    /// Never replaces original region data, never writes any file.
    /// BE-aware original Region candidate: STRICT memory-only preflight.
    /// The original single-Region disk writer cannot accept BE candidates.
    fn prepare_terrain_chunk_candidates(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
    ) -> Result<
        (
            Vec<TerrainChunkCandidate>,
            Vec<BeProof>,
            Vec<crate::biome_editing::Proof>,
        ),
        WorldError,
    > {
        self.prepare_terrain_chunk_candidates_in(reader, None, true)
    }
    fn prepare_terrain_chunk_candidates_in(
        &self,
        reader: &mut ReadOnlyRegionCache<'_>,
        only: Option<ChunkPos>,
        finalize: bool,
    ) -> Result<
        (
            Vec<TerrainChunkCandidate>,
            Vec<BeProof>,
            Vec<crate::biome_editing::Proof>,
        ),
        WorldError,
    > {
        if only.is_none()
            && self.be_overlay.is_empty()
            && self.biome_overlay.is_empty()
            && self.creation.is_empty()
        {
            return Err(refuse("没有方块实体或 biome 暂存"));
        }
        let mut blocks_only = self.clone();
        blocks_only.entity_overlay.clear();
        blocks_only.entity_sources.clear();
        blocks_only.be_overlay.clear();
        blocks_only.be_source.clear();
        blocks_only.biome_overlay.clear();
        blocks_only.biome_source.clear();
        let mut candidates = if self.overlay.is_empty() && self.creation.is_empty() {
            Vec::new()
        } else {
            blocks_only.prepare_palette_candidates(true, finalize, reader, only)?
        };
        let mut lookup = BTreeMap::new();
        for (index, item) in candidates.iter().enumerate() {
            lookup.insert((item.chunk_x, item.chunk_z), index);
        }
        let proofs = self
            .be_proofs()?
            .into_iter()
            .filter(|proof| only.is_none_or(|chunk| region_plan::be_proof_chunk(proof) == chunk))
            .collect::<Vec<_>>();
        let biomes = self
            .biome_proofs()
            .into_iter()
            .filter(|proof| only.is_none_or(|chunk| proof.chunk == chunk))
            .collect::<Vec<_>>();
        let mut be_ops: BTreeMap<ChunkPos, Vec<BeProof>> = BTreeMap::new();
        for proof in &proofs {
            be_ops
                .entry(region_plan::be_proof_chunk(proof))
                .or_default()
                .push(proof.clone());
        }
        let mut biome_ops: BTreeMap<ChunkPos, Vec<crate::biome_editing::Proof>> = BTreeMap::new();
        for proof in &biomes {
            biome_ops
                .entry(proof.chunk)
                .or_default()
                .push(proof.clone());
        }
        if be_ops.len()
            != self
                .be_overlay
                .keys()
                .filter(|chunk| only.is_none_or(|target| target == **chunk))
                .count()
            || biome_ops.len()
                != self
                    .biome_overlay
                    .keys()
                    .filter(|chunk| only.is_none_or(|target| target == **chunk))
                    .count()
        {
            return Err(refuse("区块载荷证明与暂存区块不一致"));
        }
        let mut touched: BTreeSet<ChunkPos> = be_ops.keys().copied().collect();
        touched.extend(biome_ops.keys().copied());
        for chunk in touched {
            let snapshot = self.creation.get(&chunk);
            let (original, identity, sha) = if let Some(snapshot) = snapshot {
                snapshot.verify_cached(reader)?;
                (
                    snapshot.base.clone(),
                    snapshot.identity(),
                    snapshot.source_chunk_sha.clone(),
                )
            } else {
                let raw = reader
                    .batch_chunk_bytes(chunk.0, chunk.1)?
                    .ok_or_else(|| refuse("区块载荷源不存在"))?;
                let sha = nbt_sha(&raw);
                (raw, sha.clone(), Some(sha))
            };
            let be = be_ops.get(&chunk).map(Vec::as_slice).unwrap_or(&[]);
            let bio = biome_ops.get(&chunk).map(Vec::as_slice).unwrap_or(&[]);
            if !be.is_empty() {
                if self.be_source.get(&chunk) != Some(&identity) {
                    return Err(refuse("BE 源摘要变化"));
                }
                let staged = self
                    .be_overlay
                    .get(&chunk)
                    .ok_or_else(|| refuse("BE 暂存缺失"))?;
                if region_plan::replay_be_history(&original, be)? != *staged {
                    return Err(refuse("BE 暂存与证明不一致"));
                }
            }
            if !bio.is_empty() {
                if self.biome_source.get(&chunk) != Some(&identity) {
                    return Err(refuse("biome 源摘要变化"));
                }
                let mut checked = original.clone();
                for proof in bio {
                    checked = proof.replay(&checked)?;
                }
                if self.biome_overlay.get(&chunk) != Some(&checked) {
                    return Err(refuse("biome 暂存与证明不一致"));
                }
            }
            let mut output = if let Some(&i) = lookup.get(&chunk) {
                candidates[i].patched_nbt.clone()
            } else {
                original.clone()
            };
            for proof in bio {
                output = proof.replay(&output)?;
            }
            if !be.is_empty() {
                output = region_plan::replay_be_history(&output, be)?;
            }
            if let Some(&i) = lookup.get(&chunk) {
                candidates[i].patched_nbt = output;
            } else {
                lookup.insert(chunk, candidates.len());
                candidates.push(TerrainChunkCandidate {
                    chunk_x: chunk.0,
                    chunk_z: chunk.1,
                    edited_blocks: 0,
                    source_sha: sha,
                    patched_nbt: output,
                });
            }
        }
        Ok((candidates, proofs, biomes))
    }
    pub fn preview_be_region_candidates(
        &self,
        timestamp: u32,
    ) -> Result<Vec<NativeRegionCandidate>, WorldError> {
        if !self.creation.is_empty() {
            return Err(refuse("创建区块或 section 必须通过混合事务保存"));
        }
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?
                .with_work_budget(self.save_work_budget);
        let (candidates, proofs, biomes) = self.prepare_terrain_chunk_candidates(&mut reader)?;
        let candidates = candidates
            .into_iter()
            .map(TerrainChunkCandidate::into_existing)
            .collect::<Result<Vec<_>, _>>()?;
        region_plan::plan_world_regions_with_history_budget(
            &self.world,
            &self.dimension,
            &candidates,
            &[],
            &proofs,
            &biomes,
            &[],
            timestamp,
            self.save_work_budget,
        )
    }
    /// 同一保存入口按派生数据合同完成区块。未重算的旧版方块阶段不会暴露为可写候选。
    pub fn prepare_postprocessed_candidates(
        &self,
        policy: &crate::chunk_postprocess::Policy,
        timestamp: u32,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<crate::mixed_transaction::MixedCandidate>, WorldError> {
        use crate::chunk_postprocess::Policy;
        if cancelled() {
            return Err(refuse("保存候选规划已取消"));
        }
        if self.origin.is_some() || self.completion.has_history() {
            return self.prepare_saved_chunk_candidates(policy, timestamp, cancelled);
        }
        match policy {
            Policy::NativeInvalidation => self.prepare_mixed_region_candidates(timestamp),
            Policy::LegacyRelight {
                profile,
                traits,
                operation_budget,
                memory_budget,
            } => {
                profile.verify_source(&self.world, &self.dimension)?;
                if self.world.data_version != profile.data_version {
                    return Err(refuse("旧数字格式光照合同与实际世界版本不一致"));
                }
                if self.overlay.is_empty() && self.creation.is_empty() {
                    return self.prepare_mixed_region_candidates(timestamp);
                }
                let mut reader = ReadOnlyRegionCache::for_source(
                    &self.world,
                    &self.dimension,
                    self.origin.as_ref(),
                )?
                .with_work_budget(self.save_work_budget);
                let mut block_source = self.clone();
                block_source.biome_overlay.clear();
                block_source.biome_source.clear();
                block_source.be_overlay.clear();
                block_source.be_source.clear();
                block_source.entity_overlay.clear();
                block_source.entity_sources.clear();
                let stages = block_source.prepare_palette_candidates(
                    !self.creation.is_empty(),
                    false,
                    &mut reader,
                    None,
                )?;
                let bios = self.biome_proofs();
                for (chunk, staged) in &self.biome_overlay {
                    self.staged_biome_chunk_cached(&mut reader, chunk.0, chunk.1)?;
                    let mut replay = if let Some(snapshot) = self.creation.get(chunk) {
                        snapshot.verify_cached(&mut reader)?;
                        snapshot.base.clone()
                    } else {
                        reader
                            .batch_chunk_bytes(chunk.0, chunk.1)?
                            .ok_or_else(|| refuse("旧版 biome 来源区块缺失"))?
                    };
                    for proof in bios.iter().filter(|p| p.chunk == *chunk) {
                        replay = proof.replay(&replay)?;
                    }
                    if &replay != staged {
                        return Err(refuse("旧版 biome 暂存与完整证明重放不同"));
                    }
                }
                let chunks = stages
                    .iter()
                    .map(|c| (c.chunk_x, c.chunk_z))
                    .chain(bios.iter().map(|p| p.chunk))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let mut proof = crate::legacy_lighting::prepare_proof(
                    &mut reader,
                    &self.dimension,
                    profile.clone(),
                    traits.clone(),
                    &chunks,
                    *operation_budget,
                    *memory_budget,
                    &mut cancelled,
                )?;
                proof.biome_proofs = bios;
                proof.creations = self
                    .creation
                    .iter()
                    .filter(|(_, snapshot)| snapshot.source_chunk_sha.is_none())
                    .map(|(chunk, snapshot)| {
                        let edited_blocks = stages
                            .iter()
                            .find(|stage| (stage.chunk_x, stage.chunk_z) == *chunk)
                            .ok_or_else(|| refuse("旧版新槽缺少方块阶段"))?
                            .edited_blocks;
                        Ok(crate::legacy_lighting::CreatedSource {
                            profile: snapshot.profile.clone(),
                            chunk: *chunk,
                            edited_blocks,
                        })
                    })
                    .collect::<Result<Vec<_>, WorldError>>()?;
                proof.section_creations = self
                    .creation
                    .values()
                    .flat_map(|snapshot| snapshot.section_proofs.iter().cloned())
                    .collect();
                let blocks: BTreeMap<ChunkPos, Vec<u8>> = stages
                    .into_iter()
                    .map(|c| ((c.chunk_x, c.chunk_z), c.patched_nbt))
                    .collect();
                if !proof.creations.is_empty()
                    || !proof.section_creations.is_empty()
                    || !self.be_overlay.is_empty()
                    || reader.alpha_storage()
                {
                    self.verify_entity_history()?;
                    let shared = crate::shared_chunk_save::prepare_creation_light_with_budget(
                        &self.world,
                        proof,
                        &blocks,
                        self.be_proofs()?,
                        &self.be_overlay,
                        self.entity_proofs()?,
                        timestamp,
                        &mut cancelled,
                        self.save_work_budget,
                    )?;
                    return Ok(vec![crate::mixed_transaction::MixedCandidate::SharedChunk(
                        shared,
                    )]);
                }
                let candidate = crate::legacy_lighting::prepare_regions_cached(
                    &self.world,
                    proof,
                    &blocks,
                    timestamp,
                    &mut reader,
                    &mut cancelled,
                )?;
                if !self.be_overlay.is_empty() || !self.entity_overlay.is_empty() {
                    self.verify_entity_history()?;
                    let shared = crate::shared_chunk_save::prepare_light_with_budget(
                        &self.world,
                        candidate,
                        self.be_proofs()?,
                        &self.be_overlay,
                        self.entity_proofs()?,
                        timestamp,
                        self.save_work_budget,
                    )?;
                    Ok(vec![crate::mixed_transaction::MixedCandidate::SharedChunk(
                        shared,
                    )])
                } else {
                    Ok(vec![crate::mixed_transaction::MixedCandidate::LegacyLight(
                        candidate,
                    )])
                }
            }
        }
    }
    /// 为每个源 Region 选择普通候选或含 MCC 的成对候选。
    pub fn prepare_mixed_region_candidates(
        &self,
        timestamp: u32,
    ) -> Result<Vec<crate::mixed_transaction::MixedCandidate>, WorldError> {
        if self.origin.is_some() || self.completion.has_history() {
            return self.prepare_saved_chunk_candidates(
                &crate::chunk_postprocess::Policy::NativeInvalidation,
                timestamp,
                || false,
            );
        }
        use crate::mixed_transaction::MixedCandidate;
        let mut terrain = self.clone();
        terrain.entity_overlay.clear();
        terrain.entity_sources.clear();
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?
                .with_work_budget(self.save_work_budget);
        let (chunks, proofs, biomes) = if !terrain.creation.is_empty() {
            terrain.prepare_terrain_chunk_candidates(&mut reader)?
        } else if terrain.be_overlay.is_empty()
            && terrain.biome_overlay.is_empty()
            && terrain.overlay.is_empty()
        {
            (Vec::new(), Vec::new(), Vec::new())
        } else if terrain.be_overlay.is_empty() && terrain.biome_overlay.is_empty() {
            (
                terrain.prepare_palette_candidates(false, true, &mut reader, None)?,
                Vec::new(),
                Vec::new(),
            )
        } else {
            terrain.prepare_terrain_chunk_candidates(&mut reader)?
        };
        if !self.creation.is_empty() {
            let snapshots: Vec<_> = self.creation.values().cloned().collect();
            let mut output = crate::chunk_creation::plan_mixed_with_budget(
                &self.world,
                &self.dimension,
                chunks
                    .into_iter()
                    .map(TerrainChunkCandidate::into_mixed)
                    .collect(),
                &snapshots,
                &proofs,
                &biomes,
                timestamp,
                &mut reader,
                self.save_work_budget,
            )?;
            if !self.entity_overlay.is_empty() {
                self.verify_entity_history()?;
                output.push(MixedCandidate::EntityBatch(
                    crate::entity_batch::prepare_with_budget(
                        &self.world,
                        &self.dimension,
                        self.entity_proofs()?,
                        timestamp,
                        self.save_work_budget,
                    )?,
                ));
            }
            return Ok(output);
        }
        let chunks = chunks
            .into_iter()
            .map(TerrainChunkCandidate::into_existing)
            .collect::<Result<Vec<_>, _>>()?;
        let dim = self
            .world
            .dimensions
            .iter()
            .find(|dim| dim.name == self.dimension)
            .ok_or_else(|| refuse("未知世界维度"))?;
        if dim.extension.contains("c.x.z.dat") {
            if !biomes.is_empty() {
                return Err(refuse("Alpha 区块没有可编辑的保存生物群系字段"));
            }
            if !self.entity_overlay.is_empty() {
                self.verify_entity_history()?;
                return Ok(vec![MixedCandidate::SharedChunk(
                    crate::shared_chunk_save::prepare_native_with_budget(
                        &self.world,
                        &self.dimension,
                        chunks,
                        proofs,
                        biomes,
                        self.entity_proofs()?,
                        timestamp,
                        self.save_work_budget,
                    )?,
                )]);
            }
            return chunks
                .iter()
                .map(|chunk| {
                    crate::alpha::prepare_candidate_with_history(
                        &self.world,
                        &self.dimension,
                        chunk,
                        &proofs
                            .iter()
                            .filter(|proof| {
                                region_plan::be_proof_chunk(proof) == (chunk.chunk_x, chunk.chunk_z)
                            })
                            .cloned()
                            .collect::<Vec<_>>(),
                    )
                    .map(MixedCandidate::Alpha)
                })
                .collect();
        }
        if self
            .entity_sources
            .values()
            .any(|(source, _)| !source.starts_with("entity-region:"))
        {
            self.verify_entity_history()?;
            let shared = crate::shared_chunk_save::prepare_native_with_budget(
                &self.world,
                &self.dimension,
                chunks,
                proofs,
                biomes,
                self.entity_proofs()?,
                timestamp,
                self.save_work_budget,
            )?;
            return Ok(vec![MixedCandidate::SharedChunk(shared)]);
        }
        let mut groups: BTreeMap<ChunkPos, Vec<NativeChunkCandidate>> = BTreeMap::new();
        for chunk in chunks {
            groups
                .entry((chunk.chunk_x.div_euclid(32), chunk.chunk_z.div_euclid(32)))
                .or_default()
                .push(chunk);
        }
        if groups.is_empty() && self.entity_overlay.is_empty() {
            return Err(refuse("没有待保存的区块"));
        }
        let mut output = Vec::new();
        for (region, changes) in groups {
            let mca = dim
                .region_dir
                .join(format!("r.{}.{}.mca", region.0, region.1));
            let mcr = dim
                .region_dir
                .join(format!("r.{}.{}.mcr", region.0, region.1));
            let extension = match (mca.exists(), mcr.exists()) {
                (true, false) => "mca",
                (false, true) => "mcr",
                (true, true) => return Err(refuse("同一区域同时存在 MCA 和 MCR")),
                (false, false) => return Err(refuse("源 Region 不存在")),
            };
            let region_proofs: Vec<_> = proofs
                .iter()
                .filter(|proof| {
                    let chunk = region_plan::be_proof_chunk(proof);
                    (chunk.0.div_euclid(32), chunk.1.div_euclid(32)) == region
                })
                .cloned()
                .collect();
            let region_biomes: Vec<_> = biomes
                .iter()
                .filter(|proof| {
                    (proof.chunk.0.div_euclid(32), proof.chunk.1.div_euclid(32)) == region
                })
                .cloned()
                .collect();
            if crate::region_external::requires_external_with_budget(
                &self.world,
                &self.dimension,
                region,
                extension,
                &changes,
                self.save_work_budget,
            )? {
                output.push(MixedCandidate::External(
                    crate::region_external::prepare_history_with_budget(
                        &self.world,
                        &self.dimension,
                        region,
                        extension,
                        &changes,
                        &region_proofs,
                        &region_biomes,
                        &[],
                        timestamp,
                        self.save_work_budget,
                    )?,
                ));
            } else {
                for candidate in region_plan::plan_world_regions_with_history_budget(
                    &self.world,
                    &self.dimension,
                    &changes,
                    &[],
                    &region_proofs,
                    &region_biomes,
                    &[],
                    timestamp,
                    self.save_work_budget,
                )? {
                    output.push(MixedCandidate::Region {
                        dimension: self.dimension.clone(),
                        candidate,
                    });
                }
            }
        }
        if !self.entity_overlay.is_empty() {
            self.verify_entity_history()?;
            output.push(MixedCandidate::EntityBatch(
                crate::entity_batch::prepare_with_budget(
                    &self.world,
                    &self.dimension,
                    self.entity_proofs()?,
                    timestamp,
                    self.save_work_budget,
                )?,
            ));
        }
        Ok(output)
    }
    /// Cross-Region planning is strictly read-only: every source Region
    /// must match its original SHA before an external manifest is created.
    pub fn prepare_region_batch_report(
        &self,
        timestamp: u32,
    ) -> Result<crate::region_report::RegionDryRun, WorldError> {
        crate::region_report::prepare(self, timestamp)
    }
    pub fn export_region_batch_report(
        &self,
        timestamp: u32,
        path: &Path,
    ) -> Result<usize, WorldError> {
        crate::region_report::export(self, timestamp, path)
    }
    pub fn verify_region_batch_report(&self, path: &Path) -> Result<usize, WorldError> {
        crate::region_report::verify(self, path)
    }
    pub fn prepare_native_region_candidates(
        &self,
        timestamp: u32,
    ) -> Result<Vec<NativeRegionCandidate>, WorldError> {
        let candidates = self.prepare_native_palette_candidates()?;
        region_plan::plan_world_regions_with_history_budget(
            &self.world,
            &self.dimension,
            &candidates,
            &[],
            &[],
            &[],
            &[],
            timestamp,
            self.save_work_budget,
        )
    }
    /// Experimental single-Region world write. Refuses any staged edit that
    /// touches a second Region; manual-only library API, not exposed in iced.
    /// A successful atomic replacement clears the overlay and returns the
    /// persistent original backup + pending marker for explicit finalization.
    pub fn apply_single_region_save(
        &mut self,
        timestamp: u32,
    ) -> Result<RegionWriteReceipt, WorldError> {
        let mut images = self.prepare_native_region_candidates(timestamp)?;
        if images.len() != 1 {
            return Err(refuse(
                "Multi-Region edits are not an atomic transaction; export a journal instead",
            ));
        }
        let candidate = images
            .pop()
            .ok_or_else(|| refuse("Missing single Region save candidate"))?;
        let receipt = region_transaction::apply_single_region_with_budget(
            &self.world,
            &self.dimension,
            &candidate,
            self.save_work_budget,
        )?;
        self.discard();
        Ok(receipt)
    }
    /// Capture a block-only snapshot of a source selection. Original BE
    /// coordinates always refuse capture, rather than silently copying a
    /// machine block while losing its inventory or modded NBT.
    pub fn capture_clipboard(&self, bounds: Bounds) -> Result<BlockClipboard, WorldError> {
        self.capture_clipboard_cancellable(bounds, || false)
    }
    pub fn capture_clipboard_cancellable(
        &self,
        bounds: Bounds,
        cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        self.capture_block_states_internal(bounds, false, cancelled)
    }
    /// 将插件的逐格结果作为一个结构操作组预检。未变化格及所有实体均保留。
    /// 变化格上的 BE 只有在已核实兼容时保留，否则以完整删除证明记录。
    /// 按原掩码复核插件快照，不读取或授权形状外的方块。
    pub fn verify_block_snapshot_with_budget_cancellable(
        &self,
        snapshot: &BlockClipboard,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        snapshot.validate_with_budget(&self.world, &self.dimension, budget)?;
        if snapshot.source_root != self.world.root.to_string_lossy()
            || snapshot.dimension != self.dimension
            || snapshot.has_non_block_payload()
        {
            return Err(refuse("插件方块快照必须绑定当前世界选区"));
        }
        let mut maximum = [0; 3];
        for axis in 0..3 {
            maximum[axis] = snapshot.source_min[axis]
                .checked_add(i32::from(snapshot.size[axis]) - 1)
                .ok_or_else(|| refuse("插件选区坐标溢出"))?;
        }
        let bounds = Bounds::spatial(snapshot.source_min.into(), maximum.into())?;
        let current = self.capture_block_states_masked(
            bounds,
            true,
            budget,
            snapshot.mask.as_deref(),
            cancelled,
        )?;
        if current.blocks != snapshot.blocks {
            return Err(refuse("插件运行期间源选区已改变，请重新运行插件"));
        }
        Ok(current)
    }
    pub fn prepare_block_difference_cancellable(
        &self,
        snapshot: &BlockClipboard,
        replacements: &[State],
        cancelled: impl FnMut() -> bool,
    ) -> Result<Option<PreparedStructure>, WorldError> {
        self.prepare_block_difference_with_budget_cancellable(
            snapshot,
            replacements,
            EditWorkBudget::default(),
            cancelled,
        )
    }
    pub fn prepare_block_difference_with_budget_cancellable(
        &self,
        snapshot: &BlockClipboard,
        replacements: &[State],
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Option<PreparedStructure>, WorldError> {
        if snapshot.blocks.len() != replacements.len() {
            return Err(refuse("插件结果改变了选区尺寸"));
        }
        let current =
            self.verify_block_snapshot_with_budget_cancellable(snapshot, budget, &mut cancelled)?;
        let mut maximum = [0; 3];
        for axis in 0..3 {
            maximum[axis] = snapshot.source_min[axis]
                .checked_add(i32::from(snapshot.size[axis]) - 1)
                .ok_or_else(|| refuse("插件选区坐标溢出"))?;
        }
        let bounds = Bounds::spatial(snapshot.source_min.into(), maximum.into())?;
        let current_bytes = current.work_bytes()?;
        let retained = snapshot
            .work_bytes()?
            .checked_add(
                replacements
                    .iter()
                    .map(clipboard::state_bytes)
                    .sum::<usize>(),
            )
            .and_then(|n| n.checked_add(current_bytes))
            .ok_or_else(|| refuse("插件输入输出工作集溢出"))?;
        structure_work::check_work(budget, retained)?;
        let mut mask = Vec::with_capacity(snapshot.blocks.len());
        for (index, (before, after)) in snapshot.blocks.iter().zip(replacements).enumerate() {
            if snapshot.mask.as_ref().is_some_and(|mask| !mask[index]) && before != after {
                return Err(refuse("插件试图改变选区形状外的方块"));
            }
            mask.push(before != after);
        }
        if !mask.iter().any(|changed| *changed) {
            return Ok(None);
        }
        let mut clip = snapshot.clone();
        clip.blocks = replacements.to_vec();
        clip.mask = Some(mask.clone());
        clip.source_bound = false;
        clip.source_file = None;
        clip.block_entities.clear();
        clip.scheduled_ticks.clear();
        clip.entities.clear();
        clip.biomes = None;
        clip.numeric_biomes = None;
        clip.metadata = None;
        let width = usize::from(snapshot.size[0]);
        let depth = usize::from(snapshot.size[2]);
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut payloads: BTreeMap<ChunkPos, Vec<u8>> = BTreeMap::new();
        let mut version_checked = BTreeSet::new();
        let mut diagnostics = Vec::new();
        let mut be_positions = BTreeSet::new();
        let mut visited = 0usize;
        for position in structure_work::positions(bounds) {
            let index = structure_work::index(bounds, position);
            if visited % 4096 == 0 && cancelled() {
                return Err(refuse("插件差异预检已取消"));
            }
            visited += 1;
            if !mask[index] {
                continue;
            }
            let relative = [
                (index % width) as i32,
                (index / (width * depth)) as i32,
                ((index / width) % depth) as i32,
            ];
            let position = (
                snapshot.source_min[0] + relative[0],
                snapshot.source_min[1] + relative[1],
                snapshot.source_min[2] + relative[2],
            );
            let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
            if !payloads.contains_key(&chunk) {
                payloads.clear();
                version_checked.clear();
                let base = self.terrain_payload_base_cached(&mut reader, chunk)?.0;
                let bytes = self
                    .staged_be_chunk_cached(&mut reader, chunk.0, chunk.1)?
                    .map_or(base, |staged| staged.to_vec());
                structure_work::check_work(budget, retained + bytes.len() * 8)?;
                be_positions = decode_sections(&bytes)
                    .map_err(|error| refuse(error.to_string()))?
                    .block_entities;
                payloads.insert(chunk, bytes);
            }
            if !be_positions.contains(&position) {
                continue;
            }
            let raw = payloads
                .get(&chunk)
                .ok_or_else(|| refuse("插件 BE 来源缓存缺失"))?;
            if let Some(compound) = crate::sections::original_block_entity_bytes(raw, position)
                .map_err(|e| refuse(e.to_string()))?
            {
                let mut bytes = vec![10, 0, 0];
                bytes.extend_from_slice(&compound);
                let root =
                    crate::sections::parse_root(&bytes).map_err(|e| refuse(e.to_string()))?;
                let id = root
                    .field("id")
                    .ok_or_else(|| refuse("插件目标 BE 缺少 ID"))?
                    .string()
                    .map_err(|e| refuse(e.to_string()))?;
                if crate::be_registry::compatible(
                    self.world.data_version,
                    &replacements[index].name,
                    &id,
                )? == Some(true)
                {
                    if !version_checked.contains(&chunk) {
                        let source = if self.be_overlay.contains_key(&chunk) {
                            "session:block-entity-overlay"
                        } else if self.creation.contains_key(&chunk) {
                            "session:created-terrain"
                        } else {
                            "terrain-chunk"
                        };
                        require_clipboard_payload_version(
                            chunk,
                            source,
                            clipboard_payload_version(raw, chunk, source)?,
                            clip.data_version,
                        )?;
                        version_checked.insert(chunk);
                    }
                    clip.block_entities.push(clipboard::OpaqueBlockEntity {
                        position: relative,
                        compound,
                    });
                } else {
                    diagnostics.push(format!(
                        "方块 {},{},{} 改变后，原 BE {} 将按删除证明移除",
                        position.0, position.1, position.2, id
                    ));
                }
            }
        }
        let mut prepared = self.prepare_structure_paste_with_budget_cancellable(
            &clip,
            snapshot.source_min.into(),
            clipboard::PasteOptions {
                include_biomes: false,
            },
            None,
            budget,
            cancelled,
        )?;
        prepared.diagnostics.extend(diagnostics);
        Ok(Some(prepared))
    }
    /// 只采样方块状态，允许选区包含 BE；调用者必须用差异预检决定其保留或删除。
    pub fn capture_block_states_cancellable(
        &self,
        bounds: Bounds,
        cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        self.capture_block_states_internal(bounds, true, cancelled)
    }
    fn capture_block_states_internal(
        &self,
        bounds: Bounds,
        allow_be: bool,
        cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        self.capture_block_states_budget_internal(
            bounds,
            allow_be,
            EditWorkBudget::default(),
            cancelled,
        )
    }
    pub fn capture_block_states_with_budget_cancellable(
        &self,
        region: impl Into<SelectionRegion>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        let region = region.into().resolved(&self.world, &self.dimension)?;
        let bounds = region.bounds;
        let mask = region.mask(budget, &mut cancelled)?;
        self.capture_block_states_masked(bounds, true, budget, mask.as_deref(), cancelled)
    }
    fn capture_block_states_budget_internal(
        &self,
        bounds: Bounds,
        allow_be: bool,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        self.capture_block_states_masked(bounds, allow_be, budget, None, cancelled)
    }
    fn capture_block_states_masked(
        &self,
        bounds: Bounds,
        allow_be: bool,
        budget: EditWorkBudget,
        mask: Option<&[bool]>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        if cancelled() {
            return Err(refuse("剪贴板捕获已取消"));
        }
        Bounds::spatial(bounds.minimum.into(), bounds.maximum.into())?;
        let volume = bounds.volume()?;
        clipboard::check_volume(volume, budget)?;
        let mut retained = volume * (std::mem::size_of::<Option<State>>() + 32);
        let mut scratch = 0usize;
        let width = u16::try_from(bounds.maximum[0] - bounds.minimum[0] + 1)
            .map_err(|_| refuse("Clipboard X size overflow"))?;
        let height = u16::try_from(bounds.maximum[1] - bounds.minimum[1] + 1)
            .map_err(|_| refuse("Clipboard Y size overflow"))?;
        let depth = u16::try_from(bounds.maximum[2] - bounds.minimum[2] + 1)
            .map_err(|_| refuse("Clipboard Z size overflow"))?;
        let mut chunks = BTreeMap::new();
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut be_chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut blocks: Vec<Option<State>> = vec![None; volume];
        let mut numeric: Option<bool> = None;
        let mut visited = 0usize;
        for (x, y, z) in structure_work::positions(bounds) {
            let at = structure_work::index(bounds, (x, y, z));
            if visited % 4096 == 0 && cancelled() {
                return Err(refuse("剪贴板捕获已取消"));
            }
            visited += 1;
            if mask.is_some_and(|mask| !mask[at]) {
                continue;
            }
            let pos = (x, y, z);
            let key = (x.div_euclid(16), z.div_euclid(16));
            if !chunks.contains_key(&key) {
                chunks.clear();
                scratch = 0;
                chunks.insert(
                    key,
                    self.visible_chunk_base_cached(&mut reader, key.0, key.1)?.0,
                );
                be_chunks.clear();
                let staged = self
                    .staged_be_chunk_cached(&mut reader, key.0, key.1)?
                    .map(|raw| decode_sections(raw).map_err(|e| refuse(e.to_string())))
                    .transpose()?;
                scratch += chunks[&key].as_ref().map_or(0, structure_work::chunk_bytes)
                    + staged.as_ref().map_or(0, structure_work::chunk_bytes);
                structure_work::check_work(budget, retained + scratch)?;
                be_chunks.insert(key, staged);
            }
            let chunk = chunks
                .get(&key)
                .and_then(Option::as_ref)
                .ok_or_else(|| refuse("剪贴板源区块不存在"))?;
            if !allow_be
                && be_chunks
                    .get(&key)
                    .and_then(Option::as_ref)
                    .unwrap_or(chunk)
                    .has_block_entity(x, y, z)
            {
                return Err(refuse(
                    "Clipboard selection contains original Block Entity; block-only copy refused",
                ));
            }
            let id_numeric = matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112);
            if numeric.is_some_and(|found| found != id_numeric) {
                return Err(refuse(
                    "Clipboard selection mixes numeric and named chunk schemas",
                ));
            }
            numeric = Some(id_numeric);
            let original = chunk
                .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| refuse("Clipboard source Section absent"))?;
            let value = self.overlay.get(&pos).unwrap_or(original);
            retained = retained
                .checked_add(clipboard::state_bytes(value))
                .ok_or_else(|| refuse("结构状态工作集溢出"))?;
            structure_work::check_work(budget, retained + scratch)?;
            blocks[at] = Some(value.clone());
        }
        let numeric = numeric.ok_or_else(|| refuse("选区形状没有可捕获的方块"))?;
        let excluded = parse_state(if numeric {
            "numeric:0[data=0]"
        } else {
            "minecraft:air"
        })?;
        let blocks = blocks
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                if mask.is_some_and(|mask| !mask[index]) {
                    excluded.clone()
                } else {
                    value.expect("选中的源位置已完成读取")
                }
            })
            .collect();
        let mut clip = clipboard::new_with_budget(
            &self.world,
            &self.dimension,
            numeric,
            bounds.minimum,
            [width, height, depth],
            blocks,
            budget,
        )?;
        clip.mask = mask.map(<[bool]>::to_vec);
        clip.validate_with_budget(&self.world, &self.dimension, budget)?;
        Ok(clip)
    }
    /// 捕获现代区块的方块、实体树、方块实体和原始生物群系值。
    pub fn capture_structure(&self, bounds: Bounds) -> Result<BlockClipboard, WorldError> {
        self.capture_structure_cancellable(
            bounds,
            clipboard::CaptureOptions {
                entities: true,
                biomes: true,
            },
            || false,
        )
    }
    pub fn capture_structure_cancellable(
        &self,
        bounds: Bounds,
        options: clipboard::CaptureOptions,
        cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        self.capture_structure_with_budget_cancellable(
            bounds,
            options,
            EditWorkBudget::default(),
            cancelled,
        )
    }
    pub fn capture_structure_with_budget_cancellable(
        &self,
        region: impl Into<SelectionRegion>,
        options: clipboard::CaptureOptions,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        let region = region.into().resolved(&self.world, &self.dimension)?;
        let bounds = region.bounds;
        let mask = region.mask(budget, &mut cancelled)?;
        self.capture_structure_masked(bounds, mask, options, budget, cancelled)
    }
    fn capture_structure_masked(
        &self,
        bounds: Bounds,
        mask: Option<Vec<bool>>,
        options: clipboard::CaptureOptions,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<BlockClipboard, WorldError> {
        Bounds::spatial(bounds.minimum.into(), bounds.maximum.into())?;
        let volume = bounds.volume()?;
        clipboard::check_volume(volume, budget)?;
        let mut retained = volume * (std::mem::size_of::<Option<State>>() + 32);
        let mut scratch = 0usize;
        let size = std::array::from_fn(|axis| {
            u16::try_from(bounds.maximum[axis] - bounds.minimum[axis] + 1).unwrap_or(0)
        });
        if size.contains(&0) {
            return Err(refuse("结构尺寸超出格式范围"));
        }
        let mut selected_chunks = BTreeSet::new();
        let mut chunks = BTreeMap::new();
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let mut payload_bases: BTreeMap<ChunkPos, Vec<u8>> = BTreeMap::new();
        let mut blocks: Vec<Option<State>> = vec![None; volume];
        let mut biome_cells = if options.biomes {
            vec![String::new(); volume]
        } else {
            Vec::new()
        };
        let mut numeric_biome_cells = if options.biomes {
            vec![0; volume]
        } else {
            Vec::new()
        };
        let mut named_biomes = None;
        let mut numeric_biome_version = None;
        let mut staged_biomes: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut staged_be_chunks: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut be_version_checked = BTreeSet::new();
        let mut block_entities = Vec::new();
        let mut numeric = None;
        let mut visited = 0usize;
        for (x, y, z) in structure_work::positions(bounds) {
            let at = structure_work::index(bounds, (x, y, z));
            if visited.is_multiple_of(4096) && cancelled() {
                return Err(refuse("结构源捕获已取消"));
            }
            visited += 1;
            if mask.as_ref().is_some_and(|mask| !mask[at]) {
                continue;
            }
            let pos = (x, y, z);
            let key = (x.div_euclid(16), z.div_euclid(16));
            if selected_chunks.insert(key) {
                retained = retained
                    .checked_add(96)
                    .ok_or_else(|| refuse("选区区块索引溢出"))?;
                structure_work::check_work(budget, retained + scratch)?;
            }
            if !chunks.contains_key(&key) {
                chunks.clear();
                scratch = 0;
                chunks.insert(
                    key,
                    self.visible_chunk_base_cached(&mut reader, key.0, key.1)?.0,
                );
                payload_bases.clear();
                staged_biomes.clear();
                staged_be_chunks.clear();
                be_version_checked.clear();
                let biome = if options.biomes {
                    self.staged_biome_chunk_cached(&mut reader, key.0, key.1)?
                        .map(|raw| decode_sections(raw).map_err(|e| refuse(e.to_string())))
                        .transpose()?
                } else {
                    None
                };
                let be = self
                    .staged_be_chunk_cached(&mut reader, key.0, key.1)?
                    .map(|raw| decode_sections(raw).map_err(|e| refuse(e.to_string())))
                    .transpose()?;
                scratch += chunks[&key].as_ref().map_or(0, structure_work::chunk_bytes)
                    + biome.as_ref().map_or(0, structure_work::chunk_bytes)
                    + be.as_ref().map_or(0, structure_work::chunk_bytes);
                structure_work::check_work(budget, retained + scratch)?;
                staged_biomes.insert(key, biome);
                staged_be_chunks.insert(key, be);
            }
            let chunk = chunks
                .get(&key)
                .and_then(Option::as_ref)
                .ok_or_else(|| refuse("结构源区块不存在"))?;
            let is_numeric = matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112);
            if numeric.is_some_and(|previous| previous != is_numeric) {
                return Err(refuse("结构源混合数字与命名方块布局"));
            }
            numeric = Some(is_numeric);
            let state = chunk
                .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| refuse("结构源 Section 不存在"))?;
            let value = self.overlay.get(&pos).unwrap_or(state);
            retained = retained
                .checked_add(clipboard::state_bytes(value))
                .ok_or_else(|| refuse("结构状态工作集溢出"))?;
            structure_work::check_work(budget, retained + scratch)?;
            blocks[at] = Some(value.clone());
            if options.biomes {
                let biome_chunk = staged_biomes
                    .get(&key)
                    .and_then(Option::as_ref)
                    .unwrap_or(chunk);
                let named = chunk.layout == Layout::Modern118;
                if named_biomes.is_some_and(|old| old != named) {
                    return Err(refuse("结构来源混合数字与命名 biome"));
                }
                named_biomes = Some(named);
                if named {
                    biome_cells[at] = biome_chunk
                        .biome(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                        .ok_or_else(|| refuse("结构源缺少命名 biome"))?
                        .to_owned();
                    retained = retained
                        .checked_add(biome_cells[at].len())
                        .ok_or_else(|| refuse("群系工作集溢出"))?;
                    structure_work::check_work(budget, retained + scratch)?;
                } else {
                    if numeric_biome_version.is_some_and(|v| v != biome_chunk.data_version) {
                        return Err(refuse("结构来源数字 biome 的实际区块版本不一致"));
                    }
                    numeric_biome_version = Some(biome_chunk.data_version);
                    numeric_biome_cells[at] = biome_chunk
                        .numeric_biome(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                        .ok_or_else(|| refuse("结构源缺少已核实布局的数字 biome"))?;
                }
            }
            let key = (x.div_euclid(16), z.div_euclid(16));
            let be_chunk = staged_be_chunks
                .get(&key)
                .and_then(Option::as_ref)
                .unwrap_or(chunk);
            if be_chunk.has_block_entity(x, y, z) {
                if !be_version_checked.contains(&key) {
                    let source = if self.be_overlay.contains_key(&key) {
                        "session:block-entity-overlay"
                    } else if self.creation.contains_key(&key) {
                        "session:created-terrain"
                    } else {
                        "terrain-chunk"
                    };
                    require_clipboard_payload_version(
                        key,
                        source,
                        be_chunk.data_version,
                        self.world.data_version,
                    )?;
                    be_version_checked.insert(key);
                }
                if !payload_bases.contains_key(&key) {
                    let raw = self.terrain_payload_base_cached(&mut reader, key)?.0;
                    scratch = scratch
                        .checked_add(raw.len())
                        .ok_or_else(|| refuse("结构源载荷工作集溢出"))?;
                    structure_work::check_work(budget, retained + scratch)?;
                    payload_bases.insert(key, raw);
                }
                let source = self.be_overlay.get(&key).map(Vec::as_slice).unwrap_or(
                    payload_bases
                        .get(&key)
                        .ok_or_else(|| refuse("结构载荷基底缺失"))?,
                );
                let compound = crate::sections::original_block_entity_bytes(source, pos)
                    .map_err(|e| refuse(e.to_string()))?
                    .ok_or_else(|| refuse("结构方块实体载荷缺失"))?;
                retained = retained
                    .checked_add(
                        compound.len() + std::mem::size_of::<clipboard::OpaqueBlockEntity>(),
                    )
                    .ok_or_else(|| refuse("结构 BE 工作集溢出"))?;
                structure_work::check_work(budget, retained + scratch)?;
                block_entities.push(clipboard::OpaqueBlockEntity {
                    position: [
                        x - bounds.minimum[0],
                        y - bounds.minimum[1],
                        z - bounds.minimum[2],
                    ],
                    compound,
                });
            }
        }
        let numeric = numeric.ok_or_else(|| refuse("选区形状没有可捕获的方块"))?;
        let excluded = parse_state(if numeric {
            "numeric:0[data=0]"
        } else {
            "minecraft:air"
        })?;
        let blocks = blocks
            .into_iter()
            .enumerate()
            .map(|(index, value)| {
                if mask.as_ref().is_some_and(|mask| !mask[index]) {
                    excluded.clone()
                } else {
                    value.expect("选中的源位置已完成读取")
                }
            })
            .collect();
        if named_biomes == Some(true) {
            let sample = biome_cells
                .iter()
                .find(|value| !value.is_empty())
                .expect("命名群系选区含有效来源")
                .clone();
            let omitted = mask
                .as_ref()
                .map_or(0, |mask| mask.iter().filter(|v| !**v).count());
            retained = retained
                .checked_add(
                    omitted
                        .checked_mul(sample.len())
                        .ok_or_else(|| refuse("形状群系工作集溢出"))?,
                )
                .ok_or_else(|| refuse("形状群系工作集溢出"))?;
            structure_work::check_work(budget, retained + scratch)?;
            for (index, value) in biome_cells.iter_mut().enumerate() {
                if mask.as_ref().is_some_and(|mask| !mask[index]) {
                    *value = sample.clone();
                }
            }
        }
        let mut entities = Vec::new();
        if options.entities {
            for cx in bounds.minimum[0].div_euclid(16)..=bounds.maximum[0].div_euclid(16) {
                for cz in bounds.minimum[2].div_euclid(16)..=bounds.maximum[2].div_euclid(16) {
                    if cancelled() {
                        return Err(refuse("结构实体捕获已取消"));
                    }
                    if !selected_chunks.contains(&(cx, cz)) {
                        continue;
                    }
                    let change = self.entity_input((cx, cz))?;
                    structure_work::check_work(budget, retained + change.before.len() * 8)?;
                    let catalog =
                        crate::entity_catalog::parse(&change.before, &change.source, cx, cz)?;
                    let mut version_checked = false;
                    let list_path = if change.source.starts_with("entity-region:") {
                        "/Entities"
                    } else {
                        let root = crate::sections::parse_root(&change.before)
                            .map_err(|e| refuse(e.to_string()))?;
                        if root.field("Level").is_some() {
                            "/Level/Entities"
                        } else {
                            "/Entities"
                        }
                    };
                    for item in catalog
                        .entities
                        .iter()
                        .filter(|item| item.path.matches('/').count() == 1)
                    {
                        let position = item.pos.ok_or_else(|| refuse("结构实体缺少位置"))?;
                        let cell = position.map(|value| value.floor() as i32);
                        if (0..3).all(|axis| {
                            position[axis].is_finite()
                                && position[axis] >= f64::from(bounds.minimum[axis])
                                && position[axis] < f64::from(bounds.maximum[axis]) + 1.
                        }) && mask
                            .as_ref()
                            .is_none_or(|mask| mask[structure_work::index(bounds, cell.into())])
                        {
                            if !version_checked {
                                let source = if self.entity_overlay.contains_key(&(cx, cz)) {
                                    format!("session:entity-overlay/{}", change.source)
                                } else {
                                    change.source.clone()
                                };
                                require_clipboard_payload_version(
                                    (cx, cz),
                                    &source,
                                    clipboard_payload_version(&change.before, (cx, cz), &source)?,
                                    self.world.data_version,
                                )?;
                                version_checked = true;
                            }
                            let index = item
                                .path
                                .split('/')
                                .nth(1)
                                .and_then(|v| v.parse().ok())
                                .ok_or_else(|| refuse("结构实体路径无效"))?;
                            let compound = crate::nbt_document::raw_compound_list_item(
                                &change.before,
                                list_path,
                                index,
                            )?;
                            retained = retained
                                .checked_add(
                                    compound.len() + std::mem::size_of::<clipboard::OpaqueEntity>(),
                                )
                                .ok_or_else(|| refuse("结构实体工作集溢出"))?;
                            structure_work::check_work(budget, retained + change.before.len() * 8)?;
                            entities.push(clipboard::OpaqueEntity {
                                block_position: None,
                                position: std::array::from_fn(|axis| {
                                    position[axis] - f64::from(bounds.minimum[axis])
                                }),
                                compound,
                            });
                        }
                    }
                }
            }
        }
        let mut clip = clipboard::new_with_budget(
            &self.world,
            &self.dimension,
            numeric,
            bounds.minimum,
            size,
            blocks,
            budget,
        )?;
        clip.mask = mask;
        clip.block_entities = block_entities;
        clip.scheduled_ticks = self
            .capture_structure_ticks(bounds, clip.mask.as_deref(), budget, &mut cancelled)?
            .0;
        clip.source_bound = true;
        clip.entity_dimension = self
            .world
            .data_version
            .filter(|v| matches!(v, 3465 | 3955))
            .map(|_| self.dimension.clone());
        clip.entities = entities;
        clip.bind_paintings(&self.world.world_data)?;
        clip.biomes = (named_biomes == Some(true)).then_some(clipboard::BiomeVolume {
            resolution: 1,
            size,
            cells: biome_cells,
        });
        clip.numeric_biomes = if named_biomes == Some(false) {
            Some(crate::numeric_biome::Volume {
                size,
                cells: numeric_biome_cells,
                data_version: numeric_biome_version.expect("已捕获数字 biome 来源版本"),
            })
        } else {
            None
        };
        clip.validate_with_budget(&self.world, &self.dimension, budget)?;
        Ok(clip)
    }
    /// Export a portable same-world snapshot without changing the source
    /// Minecraft world. Existing clipboard files are never overwritten.
    pub fn export_clipboard(&self, bounds: Bounds, path: &Path) -> Result<usize, WorldError> {
        let copied = self.capture_clipboard(bounds)?;
        clipboard::save(&self.world, path, &copied)?;
        Ok(copied.blocks.len())
    }
    /// Create one fully-preflighted, reversible paste Preview.
    /// Pasting never carries Block Entity/entity data and never creates
    /// absent chunks/Sections. Source is a frozen copied block snapshot.
    pub fn prepare_paste_clipboard(
        &self,
        clip: &BlockClipboard,
        destination_min: Position,
    ) -> Result<Preview, WorldError> {
        self.prepare_paste_clipboard_with_budget_cancellable(
            clip,
            destination_min,
            EditWorkBudget::default(),
            || false,
        )
    }
    /// 显式选择是否复制 biome；其余未接入的实体/BE 载荷仍拒绝。
    pub fn prepare_paste_clipboard_with_options(
        &self,
        clip: &BlockClipboard,
        destination: Position,
        options: clipboard::PasteOptions,
    ) -> Result<Preview, WorldError> {
        clip.validate(&self.world, &self.dimension)?;
        let mut selected = clip.clone();
        if let Some(biomes) = &selected.biomes {
            if options.include_biomes {
                biomes.plan_quart_masked(
                    [destination.0, destination.1, destination.2],
                    clip.mask.as_deref(),
                )?;
                return Err(refuse("biome 目标写入和混合证明尚未接入"));
            }
            selected.biomes = None;
        }
        self.prepare_paste_clipboard(&selected, destination)
    }
    /// 预检一组方块与完整方块实体；实体及 biome 写入尚需对应联合证明。
    pub fn prepare_structure_paste(
        &self,
        clip: &BlockClipboard,
        anchor: Position,
        options: clipboard::PasteOptions,
    ) -> Result<PreparedStructure, WorldError> {
        self.prepare_structure_paste_cancellable(clip, anchor, options, || false)
    }
    pub fn prepare_structure_paste_cancellable(
        &self,
        clip: &BlockClipboard,
        anchor: Position,
        options: clipboard::PasteOptions,
        cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedStructure, WorldError> {
        self.prepare_structure_paste_with_creation_cancellable(
            clip, anchor, options, None, cancelled,
        )
    }
    /// 在明确的维度和 biome 合同下预检结构，创建仍仅是内存候选。
    pub fn prepare_structure_paste_with_creation_cancellable(
        &self,
        clip: &BlockClipboard,
        anchor: Position,
        options: clipboard::PasteOptions,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedStructure, WorldError> {
        self.prepare_structure_paste_with_source_cancellable(
            clip,
            clip,
            false,
            anchor,
            options,
            profile,
            None,
            None,
            StructurePaste::default(),
            EditWorkBudget::default(),
            cancelled,
        )
    }
    /// 变换目标几何，同时用未经变换的剪贴板核验真实来源。
    pub fn prepare_transformed_structure_paste_cancellable(
        &self,
        source: &BlockClipboard,
        operation: HorizontalTransform,
        anchor: Position,
        options: clipboard::PasteOptions,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedStructure, WorldError> {
        source.validate(&self.world, &self.dimension)?;
        let mut target = source.clone();
        target.source_bound = false;
        let mut target = target.transformed(operation)?;
        target.source_bound = source.source_bound;
        self.prepare_structure_paste_with_source_cancellable(
            &target,
            source,
            operation != HorizontalTransform::Identity,
            anchor,
            options,
            profile,
            None,
            None,
            StructurePaste::default(),
            EditWorkBudget::default(),
            cancelled,
        )
    }
    /// 同时返回预检与实际重采样结果，显示和提交共享同一份几何 / 载荷。
    /// BE 的源索引来自真实采样，缩放时仍逐项核验原始来源及完整 NBT。
    pub fn prepare_resampled_structure_paste_cancellable(
        &self,
        source: &BlockClipboard,
        transform: crate::structure_transform::Transform,
        anchor: Position,
        options: clipboard::PasteOptions,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        cancelled: impl FnMut() -> bool,
    ) -> Result<(PreparedStructure, BlockClipboard), WorldError> {
        self.prepare_resampled_structure_paste_with_budget_cancellable(
            source,
            transform,
            anchor,
            options,
            profile,
            EditWorkBudget::default(),
            cancelled,
        )
    }
    pub fn prepare_resampled_structure_paste_with_budget_cancellable(
        &self,
        source: &BlockClipboard,
        transform: crate::structure_transform::Transform,
        anchor: Position,
        options: clipboard::PasteOptions,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(PreparedStructure, BlockClipboard), WorldError> {
        source.validate_with_budget(&self.world, &self.dimension, budget)?;
        let (mut target, origins) =
            source.resampled_with_origins_budget_cancellable(transform, budget, &mut cancelled)?;
        target.source_bound = source.source_bound;
        let prepared = self.prepare_structure_paste_with_source_cancellable(
            &target,
            source,
            !transform.is_identity(),
            anchor,
            options,
            profile,
            None,
            Some(&origins),
            StructurePaste::default(),
            budget,
            &mut cancelled,
        )?;
        target.source_bound = false;
        Ok((prepared, target))
    }
    pub fn prepare_structure_paste_with_budget_cancellable(
        &self,
        clip: &BlockClipboard,
        anchor: Position,
        options: clipboard::PasteOptions,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        budget: EditWorkBudget,
        cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedStructure, WorldError> {
        self.prepare_structure_paste_with_source_cancellable(
            clip,
            clip,
            false,
            anchor,
            options,
            profile,
            None,
            None,
            StructurePaste::default(),
            budget,
            cancelled,
        )
    }
    fn prepare_structure_paste_with_source_cancellable(
        &self,
        clip: &BlockClipboard,
        source_clip: &BlockClipboard,
        transformed: bool,
        anchor: Position,
        options: clipboard::PasteOptions,
        profile: Option<crate::chunk_creation::ChunkCreationProfile>,
        frozen_pairs: Option<&BTreeSet<(String, String)>>,
        be_source_indices: Option<&[usize]>,
        intent: StructurePaste<'_>,
        budget: EditWorkBudget,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<PreparedStructure, WorldError> {
        if cancelled() {
            return Err(refuse("结构预检已取消"));
        }
        if let Some(source) = &clip.source_file {
            source.verify()?;
        }
        clip.validate_with_budget(&self.world, &self.dimension, budget)?;
        let source_world = if source_clip.source_bound
            && source_clip.source_root != self.world.root.to_string_lossy()
        {
            let world = crate::inspect_world(&source_clip.source_root)?;
            if world.data_version != source_clip.data_version {
                return Err(refuse("绑定结构的来源世界 DataVersion 已变化"));
            }
            Some(world)
        } else {
            None
        };
        let local_source = source_world.is_none() && source_clip.dimension == self.dimension;
        let source_session = if source_clip.source_bound && !local_source {
            let mut source = Session::new(
                source_world.as_ref().unwrap_or(&self.world).clone(),
                source_clip.dimension.clone(),
            )?;
            if source.world.root == self.world.root {
                source.origin = self.origin.clone();
            }
            Some(source)
        } else {
            None
        };
        let destination = (
            anchor
                .0
                .checked_add(clip.offset[0])
                .ok_or_else(|| refuse("结构 X 偏移溢出"))?,
            anchor
                .1
                .checked_add(clip.offset[1])
                .ok_or_else(|| refuse("结构 Y 偏移溢出"))?,
            anchor
                .2
                .checked_add(clip.offset[2])
                .ok_or_else(|| refuse("结构 Z 偏移溢出"))?,
        );
        let biome_plan = if options.include_biomes {
            clip.biomes
                .as_ref()
                .map(|biomes| {
                    biomes.plan_quart_masked(
                        [destination.0, destination.1, destination.2],
                        clip.mask.as_deref(),
                    )
                })
                .transpose()?
        } else {
            None
        };
        let max = (
            destination
                .0
                .checked_add(i32::from(clip.size[0]) - 1)
                .ok_or_else(|| refuse("结构 X 坐标溢出"))?,
            destination
                .1
                .checked_add(i32::from(clip.size[1]) - 1)
                .ok_or_else(|| refuse("结构 Y 坐标溢出"))?,
            destination
                .2
                .checked_add(i32::from(clip.size[2]) - 1)
                .ok_or_else(|| refuse("结构 Z 坐标溢出"))?,
        );
        Bounds::spatial(destination, max)?;
        if let Some(profile) = &profile {
            profile.verify_source(&self.world, &self.dimension)?;
        }
        let mut work_bytes = clip
            .work_bytes()?
            .checked_add(source_clip.work_bytes()?)
            .ok_or_else(|| refuse("结构双快照工作集溢出"))?;
        structure_work::check_work(budget, work_bytes)?;
        let mut terrain: BTreeMap<ChunkPos, Option<Chunk>> = BTreeMap::new();
        let mut creation_before = BTreeMap::new();
        let mut creation_after: BTreeMap<ChunkPos, CreationSnapshot> = BTreeMap::new();
        let mut creation_reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        let skip_air = intent.skip_air;
        let mut entity_cells = BTreeSet::new();
        if self.world.data_version == Some(1343) {
            for entry in clip.entities.iter().chain(intent.moving_entities) {
                let p = entry.position.map(|value| value.floor() as usize);
                entity_cells.insert(
                    (p[1] * usize::from(clip.size[2]) + p[2]) * usize::from(clip.size[0]) + p[0],
                );
            }
        }
        let mut target_index = 0usize;
        for y in destination.1..=max.1 {
            for z in destination.2..=max.2 {
                for x in destination.0..=max.0 {
                    if target_index % 256 == 0 && cancelled() {
                        return Err(refuse("结构目标预检已取消"));
                    }
                    let selected = !clip.mask.as_ref().is_some_and(|mask| !mask[target_index])
                        && (options.include_biomes
                            || !skip_air
                            || !structure_air(&clip.blocks[target_index]));
                    let entity_cell = entity_cells.contains(&target_index);
                    target_index += 1;
                    if !selected && !entity_cell {
                        continue;
                    }
                    let key = (x.div_euclid(16), z.div_euclid(16));
                    if !terrain.contains_key(&key) {
                        let current = if profile.is_some() && !self.creation.contains_key(&key) {
                            crate::block_lookup::checked_original_chunk(
                                creation_reader.creation_chunk_bytes(key.0, key.1)?,
                                key.0,
                                key.1,
                            )?
                        } else {
                            self.visible_chunk_base_cached(&mut creation_reader, key.0, key.1)?
                                .0
                        };
                        work_bytes = work_bytes
                            .checked_add(current.as_ref().map_or(0, structure_work::chunk_bytes))
                            .ok_or_else(|| refuse("结构目标解码工作集溢出"))?;
                        structure_work::check_work(budget, work_bytes)?;
                        terrain.insert(key, current);
                        if let Some(snapshot) = self.creation.get(&key) {
                            creation_before.insert(key, Some(snapshot.clone()));
                            creation_after.insert(key, snapshot.clone());
                        }
                    }
                    let has_section = terrain
                        .get(&key)
                        .and_then(Option::as_ref)
                        .and_then(|chunk| {
                            chunk.block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                        })
                        .is_some();
                    // 已有地形槽可直接保存内嵌实体；没有方块写入时不补建空 section。
                    let existing_entity_slot =
                        !selected && terrain.get(&key).is_some_and(Option::is_some);
                    if !has_section && !existing_entity_slot {
                        if !creation_after.contains_key(&key) {
                            let contract = profile.clone().ok_or_else(|| refuse("结构目标缺少区块或 section；须明确启用创建并指定维度和 biome 合同"))?;
                            let snapshot = self.capture_creation_in_generation(
                                &mut creation_reader,
                                key,
                                contract,
                            )?;
                            creation_before.insert(key, None);
                            creation_after.insert(key, snapshot);
                        }
                        let snapshot = creation_after
                            .get_mut(&key)
                            .ok_or_else(|| refuse("结构创建快照缺失"))?;
                        let contract = profile
                            .as_ref()
                            .ok_or_else(|| refuse("新增区段须明确指定当前创建合同"))?;
                        creation_reader.verify_creation_profile(contract)?;
                        snapshot.ensure_section(y, contract)?;
                        terrain.insert(
                            key,
                            Some(
                                decode_sections(&snapshot.base)
                                    .map_err(|error| refuse(error.to_string()))?,
                            ),
                        );
                    }
                }
            }
        }
        let mut source_blocks = Vec::new();
        let mut source_terrain_sha = BTreeMap::new();
        let mut source_creation = BTreeMap::new();
        let mut source_biomes = Vec::new();
        let mut source_numeric_biomes = Vec::new();
        let mut foreign_reader = if source_clip.source_bound && !local_source {
            Some(ReadOnlyRegionCache::for_source(
                source_world.as_ref().unwrap_or(&self.world),
                &source_clip.dimension,
                self.origin.as_ref().filter(|origin| {
                    origin.world_root()
                        == source_world.as_ref().unwrap_or(&self.world).root.as_path()
                }),
            )?)
        } else {
            None
        };
        if source_clip.source_bound {
            let mut cache = BTreeMap::new();
            let mut biome_cache: BTreeMap<ChunkPos, Chunk> = BTreeMap::new();
            for (index, expected) in source_clip.blocks.iter().enumerate() {
                if source_clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
                    continue;
                }
                if index % 256 == 0 && cancelled() {
                    return Err(refuse("结构源预检已取消"));
                }
                let width = source_clip.size[0] as usize;
                let depth = source_clip.size[2] as usize;
                let x = source_clip.source_min[0]
                    .checked_add((index % width) as i32)
                    .ok_or_else(|| refuse("结构源 X 溢出"))?;
                let z = source_clip.source_min[2]
                    .checked_add(((index / width) % depth) as i32)
                    .ok_or_else(|| refuse("结构源 Z 溢出"))?;
                let y = source_clip.source_min[1]
                    .checked_add((index / (width * depth)) as i32)
                    .ok_or_else(|| refuse("结构源 Y 溢出"))?;
                let pos = (x, y, z);
                let key = (x.div_euclid(16), z.div_euclid(16));
                let reader = foreign_reader.as_mut().unwrap_or(&mut creation_reader);
                if !cache.contains_key(&key) {
                    let source = if local_source {
                        if let Some(snapshot) = self.creation.get(&key) {
                            source_creation.insert(key, snapshot.clone());
                        }
                        self.visible_chunk_base_cached(reader, key.0, key.1)?.0
                    } else {
                        reader.read_chunk(key.0, key.1)?
                    };
                    cache.insert(key, source);
                }
                let chunk = cache
                    .get(&key)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| refuse("结构源区块不存在"))?;
                let original = chunk
                    .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                    .ok_or_else(|| refuse("结构源 Section 不存在"))?;
                let effective = if local_source {
                    self.overlay.get(&pos).unwrap_or(original)
                } else {
                    original
                };
                if effective != expected {
                    return Err(refuse("结构来源方块已变化"));
                }
                work_bytes = work_bytes
                    .checked_add(
                        compound::state_bytes(original)
                            + compound::state_bytes(effective)
                            + std::mem::size_of::<SourceCheck>(),
                    )
                    .ok_or_else(|| refuse("结构来源证明工作集溢出"))?;
                structure_work::check_work(budget, work_bytes)?;
                source_blocks.push(SourceCheck {
                    position: pos,
                    original: original.clone(),
                    effective: effective.clone(),
                });
                if !source_terrain_sha.contains_key(&key) {
                    let identity = if let Some(snapshot) = source_creation.get(&key) {
                        snapshot.identity()
                    } else {
                        let raw = reader
                            .batch_chunk_bytes(key.0, key.1)?
                            .ok_or_else(|| refuse("结构源区块不存在"))?;
                        nbt_sha(&raw)
                    };
                    source_terrain_sha.insert(key, identity);
                }
                if options.include_biomes {
                    if let Some(biomes) = &source_clip.numeric_biomes {
                        let biome_chunk = if local_source {
                            if let Some(staged) = self.biome_overlay.get(&key) {
                                if !biome_cache.contains_key(&key) {
                                    if self.biome_source.get(&key) != source_terrain_sha.get(&key) {
                                        return Err(refuse("结构来源数字 biome 暂存已过期"));
                                    }
                                    biome_cache.insert(
                                        key,
                                        decode_sections(staged)
                                            .map_err(|e| refuse(e.to_string()))?,
                                    );
                                }
                                &biome_cache[&key]
                            } else {
                                chunk
                            }
                        } else {
                            chunk
                        };
                        let actual = biome_chunk
                            .numeric_biome(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                            .ok_or_else(|| refuse("来源数字 biome 缺失"))?;
                        if biomes.cells.get(index) != Some(&actual) {
                            return Err(refuse("来源数字 biome 已变化"));
                        }
                        source_numeric_biomes.push((pos, actual));
                    }
                    if let Some(biomes) = &source_clip.biomes {
                        if biomes.resolution != 1 {
                            return Err(refuse("绑定来源 biome 预检仅支持逐方块采样"));
                        }
                        let biome_chunk = if local_source {
                            if let Some(staged) = self.biome_overlay.get(&key) {
                                if !biome_cache.contains_key(&key) {
                                    if self.biome_source.get(&key) != source_terrain_sha.get(&key) {
                                        return Err(refuse("结构源 biome 暂存已过期"));
                                    }
                                    biome_cache.insert(
                                        key,
                                        decode_sections(staged)
                                            .map_err(|e| refuse(e.to_string()))?,
                                    );
                                }
                                biome_cache
                                    .get(&key)
                                    .ok_or_else(|| refuse("源 biome 缓存缺失"))?
                            } else {
                                chunk
                            }
                        } else {
                            chunk
                        };
                        let actual = biome_chunk
                            .biome(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                            .ok_or_else(|| refuse("结构源 biome 缺失"))?;
                        if biomes.cells.get(index).map(String::as_str) != Some(actual) {
                            return Err(refuse("结构源 biome 已变化"));
                        }
                        source_biomes.push((pos, actual.to_owned()));
                    }
                }
            }
        }
        let mut incoming = BTreeMap::new();
        let source_tick_sha = if source_clip.source_bound {
            let mut maximum = [0; 3];
            for axis in 0..3 {
                maximum[axis] = source_clip.source_min[axis]
                    .checked_add(i32::from(source_clip.size[axis]) - 1)
                    .ok_or_else(|| refuse("计划更新源范围溢出"))?;
            }
            let (actual, sources) = source_session
                .as_ref()
                .unwrap_or(self)
                .capture_structure_ticks(
                    Bounds {
                        minimum: source_clip.source_min,
                        maximum,
                    },
                    source_clip.mask.as_deref(),
                    budget,
                    &mut cancelled,
                )?;
            if actual != source_clip.scheduled_ticks {
                return Err(refuse("结构计划更新与真实来源不同"));
            }
            sources
        } else {
            BTreeMap::new()
        };
        let mut incoming_ticks = BTreeMap::<ChunkPos, Vec<crate::scheduled_ticks::Tick>>::new();
        for tick in &clip.scheduled_ticks {
            let mut position = [0; 3];
            for axis in 0..3 {
                position[axis] = [destination.0, destination.1, destination.2][axis]
                    .checked_add(tick.position[axis])
                    .ok_or_else(|| refuse("目标计划更新坐标溢出"))?;
            }
            incoming_ticks
                .entry((position[0].div_euclid(16), position[2].div_euclid(16)))
                .or_default()
                .push(crate::scheduled_ticks::Tick {
                    position,
                    kind: tick.kind,
                    compound: crate::scheduled_ticks::relocate(&tick.compound, position)?,
                });
        }
        let mut source_block_entities = Vec::new();
        for (entry_index, entry) in clip.block_entities.iter().enumerate() {
            let index = ((entry.position[1] as usize * clip.size[2] as usize
                + entry.position[2] as usize)
                * clip.size[0] as usize)
                + entry.position[0] as usize;
            let clipboard_block = clip
                .blocks
                .get(index)
                .ok_or_else(|| refuse("结构 BE 方块索引无效"))?;
            if source_clip.source_bound {
                let source_index = match be_source_indices {
                    Some(indices) => *indices
                        .get(entry_index)
                        .ok_or_else(|| refuse("重采样 BE 来源索引缺失"))?,
                    None => entry_index,
                };
                let source_entry = source_clip
                    .block_entities
                    .get(source_index)
                    .ok_or_else(|| refuse("结构源 BE 对应关系缺失"))?;
                let source_index = ((source_entry.position[1] as usize
                    * source_clip.size[2] as usize
                    + source_entry.position[2] as usize)
                    * source_clip.size[0] as usize)
                    + source_entry.position[0] as usize;
                let source_clipboard_block = source_clip
                    .blocks
                    .get(source_index)
                    .ok_or_else(|| refuse("结构源 BE 方块索引无效"))?;
                if source_clipboard_block.name != clipboard_block.name {
                    return Err(refuse("变换后的 BE 所属方块类型与原来源不同"));
                }
                let sx = source_clip.source_min[0]
                    .checked_add(source_entry.position[0])
                    .ok_or_else(|| refuse("源 BE X 溢出"))?;
                let sy = source_clip.source_min[1]
                    .checked_add(source_entry.position[1])
                    .ok_or_else(|| refuse("源 BE Y 溢出"))?;
                let sz = source_clip.source_min[2]
                    .checked_add(source_entry.position[2])
                    .ok_or_else(|| refuse("源 BE Z 溢出"))?;
                let source_chunk = (sx.div_euclid(16), sz.div_euclid(16));
                let reader = foreign_reader.as_mut().unwrap_or(&mut creation_reader);
                let (raw, sha) = if local_source {
                    self.terrain_payload_base_cached(reader, source_chunk)?
                } else {
                    let raw = reader
                        .batch_chunk_bytes(source_chunk.0, source_chunk.1)?
                        .ok_or_else(|| refuse("结构源 BE 区块不存在"))?;
                    let sha = nbt_sha(&raw);
                    (raw, sha)
                };
                let staged = if local_source {
                    self.be_overlay.get(&source_chunk)
                } else {
                    None
                };
                if staged.is_some() && self.be_source.get(&source_chunk) != Some(&sha) {
                    return Err(refuse("结构源 BE 暂存已过期"));
                }
                let actual = crate::sections::original_block_entity_bytes(
                    staged.unwrap_or(&raw),
                    (sx, sy, sz),
                )
                .map_err(|e| refuse(e.to_string()))?
                .ok_or_else(|| refuse("结构源 BE 不存在"))?;
                if actual != source_entry.compound {
                    return Err(refuse("结构 BE 内容与真实源不一致"));
                }
                let decoded = decode_sections(&raw).map_err(|e| refuse(e.to_string()))?;
                let original = decoded
                    .block(sx.rem_euclid(16) as u8, sy, sz.rem_euclid(16) as u8)
                    .ok_or_else(|| refuse("结构源 BE 方块不存在"))?;
                let source_block = if local_source {
                    self.overlay.get(&(sx, sy, sz)).unwrap_or(original)
                } else {
                    original
                };
                if source_clipboard_block != source_block {
                    return Err(refuse("结构源 BE 与源方块状态不匹配"));
                }
                source_block_entities.push(StructureSourceBe {
                    dimension: source_clip.dimension.clone(),
                    position: (sx, sy, sz),
                    terrain_sha: sha,
                    compound_sha: nbt_sha(&actual),
                    block: source_block.clone(),
                });
            } else {
                let mut root = vec![10, 0, 0];
                root.extend_from_slice(&entry.compound);
                let entity =
                    crate::sections::parse_root(&root).map_err(|e| refuse(e.to_string()))?;
                let id = entity
                    .field("id")
                    .ok_or_else(|| refuse("结构 BE 缺少 ID"))?
                    .string()
                    .map_err(|e| refuse(e.to_string()))?;
                // 组合克隆可携带从真实源捕获的配对：缩放后一个源 BE 对应多个目标，
                // 仍由顶层源快照在提交前完整复核，不把未知模组配对猜成原版配对。
                if !frozen_pairs.is_some_and(|pairs| {
                    pairs.contains(&(clipboard_block.name.clone(), id.clone()))
                }) {
                    crate::be_registry::validate(
                        self.world.data_version,
                        &clipboard_block.name,
                        &id,
                    )?;
                }
            }
            let position = (
                destination
                    .0
                    .checked_add(entry.position[0])
                    .ok_or_else(|| refuse("BE X 溢出"))?,
                destination
                    .1
                    .checked_add(entry.position[1])
                    .ok_or_else(|| refuse("BE Y 溢出"))?,
                destination
                    .2
                    .checked_add(entry.position[2])
                    .ok_or_else(|| refuse("BE Z 溢出"))?,
            );
            incoming.insert(
                position,
                crate::be_editing::relocate_compound(&entry.compound, position)?,
            );
        }
        let prior: BTreeSet<_> = self
            .be_proofs()?
            .iter()
            .filter_map(|proof| match proof {
                BeProof::Structure(p) => Some(p.position()),
                _ => None,
            })
            .collect();
        let mut be_chunks: BTreeMap<ChunkPos, StructureBeChunk> = BTreeMap::new();
        let mut tick_positions = BTreeMap::<ChunkPos, BTreeSet<[i32; 3]>>::new();
        let mut tick_proofs = Vec::new();
        let mut existing_be: BTreeMap<ChunkPos, BTreeSet<Position>> = BTreeMap::new();
        let mut proofs = Vec::new();
        let mut blocks = Vec::new();
        let mut overwritten = 0usize;
        let mut stacks = 0usize;
        let mut index = 0usize;
        for y in destination.1..=max.1 {
            for z in destination.2..=max.2 {
                for x in destination.0..=max.0 {
                    if index % 256 == 0 && cancelled() {
                        return Err(refuse("结构预检已取消"));
                    }
                    if clip.mask.as_ref().is_some_and(|mask| !mask[index])
                        || skip_air && structure_air(&clip.blocks[index])
                    {
                        index += 1;
                        continue;
                    }
                    let position = (x, y, z);
                    let key = (x.div_euclid(16), z.div_euclid(16));
                    let chunk = terrain
                        .get(&key)
                        .and_then(Option::as_ref)
                        .ok_or_else(|| refuse("结构目标区块预检缓存缺失"))?;
                    let numeric =
                        matches!(chunk.layout, Layout::McRegion128 | Layout::NumericAnvil112);
                    if numeric != clip.numeric {
                        return Err(refuse("结构方块与目标存储格式不匹配"));
                    }
                    let original = chunk
                        .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                        .ok_or_else(|| refuse("结构目标 Section 不存在"))?;
                    let before = self.overlay.get(&position).unwrap_or(original);
                    let after = clip
                        .blocks
                        .get(index)
                        .ok_or_else(|| refuse("结构方块数量不足"))?;
                    let block_changed = before != after;
                    if block_changed {
                        work_bytes = work_bytes
                            .checked_add(
                                compound::state_bytes(original)
                                    + compound::state_bytes(before)
                                    + compound::state_bytes(after)
                                    + std::mem::size_of::<Change>(),
                            )
                            .ok_or_else(|| refuse("结构方块候选溢出"))?;
                        structure_work::check_work(budget, work_bytes)?;
                        blocks.push(Change {
                            position,
                            original: original.clone(),
                            before: before.clone(),
                            after: after.clone(),
                        });
                    }
                    index += 1;
                    let key = (x.div_euclid(16), z.div_euclid(16));
                    if !be_chunks.contains_key(&key) {
                        let input = structure_payload_input(
                            &mut creation_reader,
                            key,
                            creation_after.get(&key),
                            creation_before.get(&key).and_then(Option::as_ref),
                            self.be_overlay.get(&key),
                            self.be_source.get(&key),
                        )?;
                        work_bytes = work_bytes
                            .checked_add(input.before.len() + input.after.len())
                            .ok_or_else(|| refuse("结构目标载荷工作集溢出"))?;
                        structure_work::check_work(budget, work_bytes)?;
                        let positions = if self.be_overlay.contains_key(&key) {
                            decode_sections(&input.after)
                                .map_err(|error| refuse(error.to_string()))?
                                .block_entities
                        } else {
                            chunk.block_entities.clone()
                        };
                        existing_be.insert(key, positions);
                        be_chunks.insert(key, input);
                    }
                    let group = be_chunks
                        .get_mut(&key)
                        .ok_or_else(|| refuse("结构目标缓存缺失"))?;
                    tick_positions.entry(key).or_default().insert([x, y, z]);
                    work_bytes = work_bytes
                        .checked_add(64)
                        .ok_or_else(|| refuse("计划更新位置索引溢出"))?;
                    structure_work::check_work(budget, work_bytes)?;
                    let old = if existing_be[&key].contains(&position) {
                        crate::sections::original_block_entity_bytes(&group.after, position)
                            .map_err(|error| refuse(error.to_string()))?
                    } else {
                        None
                    };
                    let new = incoming.get(&position).map(Vec::as_slice);
                    if old.as_deref() != new || (block_changed && old.is_some()) {
                        if old.is_some() || new.is_some() {
                            if prior.contains(&position) {
                                return Err(refuse("同一位置已有结构 BE 证明，请先保存或撤销"));
                            }
                            if let Some(bytes) = &old {
                                if old.as_deref() != new {
                                    overwritten += 1;
                                    let mut root = vec![10, 0, 0];
                                    root.extend_from_slice(bytes);
                                    let entity = crate::sections::parse_root(&root)
                                        .map_err(|e| refuse(e.to_string()))?;
                                    if let Some(items) = entity.field("Items") {
                                        if items.kind != 9 {
                                            return Err(refuse("目标库存 Items 不是列表"));
                                        }
                                        stacks = stacks.saturating_add(items.items.len());
                                    }
                                }
                            }
                            let proof = crate::be_editing::structure(&group.after, position, new)?;
                            work_bytes = work_bytes
                                .checked_add(compound::proof_bytes(&proof)?)
                                .ok_or_else(|| refuse("结构方块实体证明工作集溢出"))?;
                            structure_work::check_work(budget, work_bytes)?;
                            group.after = proof.replay(&group.after)?;
                            proofs.push(proof);
                        }
                    }
                }
            }
        }
        for (key, touched) in tick_positions {
            let group = be_chunks.get_mut(&key).expect("选中位置已加载目标载荷");
            if incoming_ticks
                .get(&key)
                .is_some_and(|ticks| ticks.iter().any(|tick| touched.contains(&tick.position)))
            {
                require_clipboard_payload_version(
                    key,
                    "target:scheduled-ticks",
                    clipboard_payload_version(&group.after, key, "target:scheduled-ticks")?,
                    clip.data_version,
                )?;
            }
            if let Some(proof) = crate::scheduled_ticks::Proof::plan(
                &group.after,
                key,
                &touched,
                incoming_ticks.get(&key).map_or(&[], Vec::as_slice),
            )? {
                work_bytes = work_bytes
                    .checked_add(compound::proof_bytes(&proof)?)
                    .ok_or_else(|| refuse("计划更新证明工作集溢出"))?;
                structure_work::check_work(budget, work_bytes)?;
                group.after = proof.replay(&group.after)?;
                tick_proofs.push(proof);
            }
        }
        let target_sources = be_chunks
            .iter()
            .map(|(key, entry)| (*key, entry.source_sha.clone()))
            .collect();
        let proof_chunks: BTreeSet<_> = proofs
            .iter()
            .map(|p| {
                let (x, _, z) = p.position();
                (x.div_euclid(16), z.div_euclid(16))
            })
            .chain(tick_proofs.iter().map(|proof| proof.chunk()))
            .collect();
        let be_chunks = be_chunks
            .into_values()
            .filter(|entry| {
                proof_chunks.contains(&entry.chunk)
                    || (self.be_overlay.contains_key(&entry.chunk) && entry.before != entry.after)
            })
            .collect();
        let mut biome_chunks = BTreeMap::new();
        let mut biome_proofs = Vec::new();
        for (&chunk, snapshot) in &creation_after {
            if let Some(staged) = self.biome_overlay.get(&chunk) {
                let input = structure_payload_input(
                    &mut creation_reader,
                    chunk,
                    Some(snapshot),
                    creation_before.get(&chunk).and_then(Option::as_ref),
                    Some(staged),
                    self.biome_source.get(&chunk),
                )?;
                if input.before != input.after {
                    biome_chunks.insert(chunk, input);
                }
            }
        }
        if let Some(plan) = biome_plan {
            let mut grouped: BTreeMap<ChunkPos, Vec<([i32; 3], String)>> = BTreeMap::new();
            for (quart, value) in plan {
                grouped
                    .entry((quart[0].div_euclid(4), quart[2].div_euclid(4)))
                    .or_default()
                    .push((quart, value));
            }
            for (chunk, cells) in grouped {
                if cancelled() {
                    return Err(refuse("结构预检已取消"));
                }
                let mut input = if let Some(input) = biome_chunks.remove(&chunk) {
                    input
                } else {
                    structure_payload_input(
                        &mut creation_reader,
                        chunk,
                        creation_after.get(&chunk),
                        creation_before.get(&chunk).and_then(Option::as_ref),
                        self.biome_overlay.get(&chunk),
                        self.biome_source.get(&chunk),
                    )?
                };
                let decoded = decode_sections(&input.after).map_err(|e| refuse(e.to_string()))?;
                let mut edits = Vec::new();
                for (quart, replacement) in cells {
                    let y = quart[1]
                        .checked_mul(4)
                        .ok_or_else(|| refuse("biome Y 溢出"))?;
                    let x = (quart[0].rem_euclid(4) * 4) as u8;
                    let z = (quart[2].rem_euclid(4) * 4) as u8;
                    let original = decoded
                        .biome(x, y, z)
                        .ok_or_else(|| refuse("目标 biome 或 Section 缺失"))?
                        .to_owned();
                    if original != replacement {
                        edits.push(crate::biome_editing::Edit {
                            chunk,
                            quart_x: (x / 4),
                            quart_y: quart[1],
                            quart_z: (z / 4),
                            original,
                            replacement,
                        });
                    }
                }
                if !edits.is_empty() {
                    input.after = crate::biome_editing::patch(&input.after, &edits)?;
                    biome_proofs.push(crate::biome_editing::Proof {
                        chunk,
                        edits,
                        numeric: None,
                    });
                }
                if input.before != input.after {
                    biome_chunks.insert(chunk, input);
                }
            }
        }
        let mut entity_groups = Vec::new();
        if options.include_biomes {
            if let Some(volume) = &clip.numeric_biomes {
                for cx in destination.0.div_euclid(16)..=max.0.div_euclid(16) {
                    for cz in destination.2.div_euclid(16)..=max.2.div_euclid(16) {
                        if !terrain.contains_key(&(cx, cz)) {
                            continue;
                        }
                        if cancelled() {
                            return Err(refuse("数字 biome 目标预检已取消"));
                        }
                        let chunk = (cx, cz);
                        let mut input = if let Some(input) = biome_chunks.remove(&chunk) {
                            input
                        } else {
                            structure_payload_input(
                                &mut creation_reader,
                                chunk,
                                creation_after.get(&chunk),
                                creation_before.get(&chunk).and_then(Option::as_ref),
                                self.biome_overlay.get(&chunk),
                                self.biome_source.get(&chunk),
                            )?
                        };
                        let decoded =
                            decode_sections(&input.after).map_err(|e| refuse(e.to_string()))?;
                        let data = decoded.numeric_biomes.as_ref().ok_or_else(|| {
                            refuse("目标区块缺少数字 biome；不能猜测命名注册表转换")
                        })?;
                        let edits = volume.plan_masked(
                            [destination.0, destination.1, destination.2],
                            chunk,
                            data,
                            clip.mask.as_deref(),
                        )?;
                        if !edits.is_empty() {
                            let proof = crate::biome_editing::Proof {
                                chunk,
                                edits: Vec::new(),
                                numeric: Some(crate::numeric_biome::Proof {
                                    storage: data.storage,
                                    edits,
                                }),
                            };
                            input.after = proof.replay(&input.after)?;
                            biome_proofs.push(proof);
                            biome_chunks.insert(chunk, input);
                        }
                    }
                }
            }
        }
        let mut source_entities = Vec::new();
        let mut script_references = crate::entity_editing::ReferenceRemap::default();
        let mut diagnostics = Vec::new();
        if !clip.entities.is_empty() {
            if (transformed || !local_source) && source_clip.source_bound {
                if source_clip.entities.len() != clip.entities.len() {
                    return Err(refuse("变换前后实体数量不匹配"));
                }
                for entry in &source_clip.entities {
                    if cancelled() {
                        return Err(refuse("结构源实体预检已取消"));
                    }
                    source_entities.push(
                        source_session
                            .as_ref()
                            .unwrap_or(self)
                            .structure_entity_source(entry)?,
                    );
                }
            }
            const IDENTITY_BUDGET: usize = 512 * 1024 * 1024;
            let mut identities = self.scan_entity_identities_with_creation(
                &creation_after,
                IDENTITY_BUDGET,
                &mut cancelled,
            )?;
            let mut working: BTreeMap<ChunkPos, Vec<u8>> = BTreeMap::new();
            let delta = [
                destination
                    .0
                    .checked_sub(clip.source_min[0])
                    .ok_or_else(|| refuse("实体 X 位移溢出"))?,
                destination
                    .1
                    .checked_sub(clip.source_min[1])
                    .ok_or_else(|| refuse("实体 Y 位移溢出"))?,
                destination
                    .2
                    .checked_sub(clip.source_min[2])
                    .ok_or_else(|| refuse("实体 Z 位移溢出"))?,
            ];
            let identity_plan = crate::entity_editing::plan_imports(
                &clip.entities,
                clip.data_version,
                delta,
                &identities,
                budget,
                &mut cancelled,
            )?;
            script_references = identity_plan.existing_references(|id| identities.contains(id));
            for (root_index, entry) in clip.entities.iter().enumerate() {
                if cancelled() {
                    return Err(refuse("结构实体预检已取消"));
                }
                if let Some(anchor) = entry.block_position {
                    let derived = entry.position.map(|v| v.floor() as i32);
                    if anchor != derived {
                        return Err(refuse("结构实体的独立 blockPos 锚点尚不能无损粘贴"));
                    }
                }
                let mut entity_root = vec![10, 0, 0];
                entity_root.extend_from_slice(&entry.compound);
                let parsed =
                    crate::sections::parse_root(&entity_root).map_err(|e| refuse(e.to_string()))?;
                let pos = parsed
                    .field("Pos")
                    .ok_or_else(|| refuse("结构实体缺少 Pos"))?;
                if pos.kind != 9
                    || pos.items.len() != 3
                    || pos.items.iter().any(|v| v.kind != 6 || v.raw.len() != 8)
                {
                    return Err(refuse("结构实体 Pos 类型无效"));
                }
                let original: [f64; 3] = std::array::from_fn(|axis| {
                    f64::from_be_bytes(pos.items[axis].raw.try_into().expect("已校验坐标宽度"))
                });
                let from = (
                    (original[0] / 16.0).floor() as i32,
                    (original[2] / 16.0).floor() as i32,
                );
                let to = (
                    ((original[0] + f64::from(delta[0])) / 16.0).floor() as i32,
                    ((original[2] + f64::from(delta[2])) / 16.0).floor() as i32,
                );
                let mut target = self.entity_input_with_creation(
                    to,
                    creation_after.get(&to).or_else(|| self.creation.get(&to)),
                )?;
                if let Some(bytes) = working.get(&to) {
                    target.before = bytes.clone();
                }
                let missing = target.source == "entity-region:missing"
                    && !self.entity_overlay.contains_key(&to)
                    && !working.contains_key(&to);
                let target_bytes = if missing {
                    None
                } else {
                    Some(target.before.as_slice())
                };
                let proof = if source_clip.source_bound && !transformed && local_source {
                    let source = self.entity_input(from)?;
                    let source_bytes = working.get(&from).unwrap_or(&source.before);
                    let label = &source.source;
                    let catalog =
                        crate::entity_catalog::parse(source_bytes, label, from.0, from.1)?;
                    let list_path = if label.starts_with("entity-region:") {
                        "/Entities"
                    } else {
                        let root = crate::sections::parse_root(source_bytes)
                            .map_err(|e| refuse(e.to_string()))?;
                        if root.field("Level").is_some() {
                            "/Level/Entities"
                        } else {
                            "/Entities"
                        }
                    };
                    let mut source_index = None;
                    for entity in catalog
                        .entities
                        .iter()
                        .filter(|entity| entity.path.matches('/').count() == 1)
                    {
                        let index = entity
                            .path
                            .split('/')
                            .nth(1)
                            .and_then(|v| v.parse::<usize>().ok())
                            .ok_or_else(|| refuse("结构实体索引无效"))?;
                        if crate::nbt_document::raw_compound_list_item(
                            source_bytes,
                            list_path,
                            index,
                        )? == entry.compound
                        {
                            if source_index.replace(index).is_some() {
                                return Err(refuse("结构源实体载荷不唯一"));
                            }
                        }
                    }
                    let source_index =
                        source_index.ok_or_else(|| refuse("结构源实体已变化或不存在"))?;
                    let proof = identity_plan.clone_entity(
                        root_index,
                        source_bytes,
                        target_bytes,
                        from,
                        to,
                        source.source.starts_with("entity-region:"),
                        target.source.starts_with("entity-region:"),
                        source_index,
                        delta,
                        &identities,
                        crate::entity_editing::DimensionChange::between(
                            Some(&source_clip.dimension),
                            Some(&self.dimension),
                        ),
                    )?;
                    identities.verify_clone(&proof)?;
                    target.after = proof.replay(source_bytes, target_bytes)?;
                    diagnostics.extend(proof.diagnostics().iter().cloned());
                    EntityProof::Clone(proof)
                } else {
                    let proof = identity_plan.import_entity(
                        root_index,
                        &entry.compound,
                        target_bytes,
                        to,
                        target.source.starts_with("entity-region:"),
                        delta,
                        &identities,
                        crate::entity_editing::DimensionChange::between(
                            if source_clip.source_bound {
                                Some(source_clip.dimension.as_str())
                            } else {
                                clip.entity_dimension.as_deref()
                            },
                            Some(&self.dimension),
                        ),
                    )?;
                    proof.verify_snapshot(&identities)?;
                    target.after = proof.replay(target_bytes)?;
                    diagnostics.extend(proof.diagnostics().iter().cloned());
                    EntityProof::Import(proof)
                };
                identities.replace_chunks(
                    &[crate::entity_editing::IdentityChunkChange {
                        dimension: &self.dimension,
                        chunk: to,
                        separate: target.source.starts_with("entity-region:"),
                        before: target_bytes,
                        after: Some(&target.after),
                    }],
                    IDENTITY_BUDGET,
                )?;
                working.insert(to, target.after.clone());
                entity_groups.push(EntityGroup {
                    changes: vec![target],
                    proof,
                });
            }
        }
        crate::entity_editing::verify_clone_scopes(
            entity_groups
                .iter()
                .filter_map(|entity| entity.proof.clone_proof()),
        )?;
        let prepared = PreparedStructure {
            work_budget: budget,
            root: self.world.root.clone(),
            dimension: self.dimension.clone(),
            identity_revision: (!entity_groups.is_empty()).then(|| self.revision()),
            target_sources,
            creation_before,
            creation_after,
            source_block_entities,
            source_tick_sha,
            source_entities,
            source_world: source_world.as_ref().map(|world| world.root.clone()),
            source_dimension: source_clip.dimension.clone(),
            source_blocks,
            source_terrain_sha,
            source_creation,
            source_file: source_clip.source_file.clone(),
            source_biomes,
            source_numeric_biomes,
            blocks,
            be_chunks,
            be_proofs: proofs,
            tick_proofs,
            biome_chunks: biome_chunks.into_values().collect(),
            biome_proofs,
            entity_groups,
            script_references,
            source_metadata: clip.metadata.clone(),
            diagnostics,
            overwritten_block_entities: overwritten,
            overwritten_inventory_stacks: stacks,
        };
        prepared.check_work(budget)?;
        Ok(prepared)
    }
    fn check_structure_cancellable(
        &self,
        group: &PreparedStructure,
        after: bool,
        cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        let mut reader =
            ReadOnlyRegionCache::for_source(&self.world, &self.dimension, self.origin.as_ref())?;
        self.check_structure_cached(group, after, &mut reader, cancelled)
    }
    fn check_structure_cached(
        &self,
        group: &PreparedStructure,
        after: bool,
        target_reader: &mut ReadOnlyRegionCache<'_>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        if cancelled() {
            return Err(refuse("结构复核已取消"));
        }
        if group.root != self.world.root || group.dimension != self.dimension {
            return Err(refuse("结构预览属于其他世界或维度"));
        }
        crate::entity_editing::verify_clone_scopes(
            group
                .entity_groups
                .iter()
                .filter_map(|entity| entity.proof.clone_proof()),
        )?;
        let source_world = if !after {
            group
                .source_world
                .as_ref()
                .map(crate::inspect_world)
                .transpose()?
        } else {
            None
        };
        if source_world
            .as_ref()
            .is_some_and(|world| world.data_version != self.world.data_version)
        {
            return Err(refuse("绑定结构的来源世界 DataVersion 已变化"));
        }
        let local_source = group.source_world.is_none() && group.source_dimension == self.dimension;
        let mut foreign_reader = if !after && !local_source && !group.source_blocks.is_empty() {
            Some(ReadOnlyRegionCache::for_source(
                source_world.as_ref().unwrap_or(&self.world),
                &group.source_dimension,
                self.origin.as_ref().filter(|origin| {
                    origin.world_root()
                        == source_world.as_ref().unwrap_or(&self.world).root.as_path()
                }),
            )?)
        } else {
            None
        };
        let source_session = if !after && !local_source && !group.source_entities.is_empty() {
            let mut source = Session::new(
                source_world.as_ref().unwrap_or(&self.world).clone(),
                group.source_dimension.clone(),
            )?;
            if source.world.root == self.world.root {
                source.origin = self.origin.clone();
            }
            Some(source)
        } else {
            None
        };
        fn source_bytes(
            foreign: &mut Option<ReadOnlyRegionCache<'_>>,
            target: &mut ReadOnlyRegionCache<'_>,
            chunk: (i32, i32),
        ) -> Result<Option<Vec<u8>>, WorldError> {
            match foreign.as_mut() {
                Some(reader) => reader.batch_chunk_bytes(chunk.0, chunk.1),
                None => target.batch_chunk_bytes(chunk.0, chunk.1),
            }
        }
        for snapshot in group.creation_after.values() {
            if cancelled() {
                return Err(refuse("结构创建来源复核已取消"));
            }
            snapshot.verify_cached(target_reader)?;
        }
        if !after {
            for snapshot in group.source_creation.values() {
                if cancelled() {
                    return Err(refuse("结构创建来源复核已取消"));
                }
                snapshot.verify_cached(target_reader)?;
            }
            if let Some(source) = &group.source_file {
                source.verify()?;
            }
            let mut source_cache = BTreeMap::new();
            for (&chunk, sha) in &group.source_terrain_sha {
                let identity = if let Some(snapshot) = group.source_creation.get(&chunk) {
                    match foreign_reader.as_mut() {
                        Some(reader) => snapshot.verify_cached(reader)?,
                        None => snapshot.verify_cached(target_reader)?,
                    }
                    if self.creation.get(&chunk) != Some(snapshot) {
                        return Err(refuse("结构源创建快照已变化"));
                    }
                    snapshot.identity()
                } else {
                    let raw = source_bytes(&mut foreign_reader, target_reader, chunk)?
                        .ok_or_else(|| refuse("结构源区块消失"))?;
                    nbt_sha(&raw)
                };
                if &identity != sha {
                    return Err(refuse("结构源区块摘要已变化"));
                }
            }
            for (step, check) in group.source_blocks.iter().enumerate() {
                if step % 4096 == 0 && cancelled() {
                    return Err(refuse("结构方块源复核已取消"));
                }
                let (x, y, z) = check.position;
                let key = (x.div_euclid(16), z.div_euclid(16));
                if !source_cache.contains_key(&key) {
                    let chunk = if let Some(snapshot) = group.source_creation.get(&key) {
                        Some(decode_sections(&snapshot.base).map_err(|e| refuse(e.to_string()))?)
                    } else {
                        crate::block_lookup::checked_original_chunk(
                            source_bytes(&mut foreign_reader, target_reader, key)?,
                            key.0,
                            key.1,
                        )?
                    };
                    source_cache.insert(key, chunk);
                }
                let chunk = source_cache
                    .get(&key)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| refuse("结构源区块消失"))?;
                let original = chunk
                    .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                    .ok_or_else(|| refuse("结构源 Section 消失"))?;
                if original != &check.original {
                    return Err(refuse("结构源方块已变化"));
                }
                let effective = if local_source {
                    self.overlay.get(&check.position).unwrap_or(original)
                } else {
                    original
                };
                if effective != &check.effective {
                    return Err(refuse("结构源暂存方块已变化"));
                }
            }
            let mut biome_cache: BTreeMap<ChunkPos, Chunk> = BTreeMap::new();
            enum ExpectedBiome<'a> {
                Named(&'a str),
                Numeric(i32),
            }
            for (position, expected) in group
                .source_biomes
                .iter()
                .map(|(p, v)| (p, ExpectedBiome::Named(v)))
                .chain(
                    group
                        .source_numeric_biomes
                        .iter()
                        .map(|(p, v)| (p, ExpectedBiome::Numeric(*v))),
                )
            {
                let (x, y, z) = *position;
                let key = (x.div_euclid(16), z.div_euclid(16));
                if !biome_cache.contains_key(&key) {
                    let raw = if let Some(snapshot) = group.source_creation.get(&key) {
                        snapshot.base.clone()
                    } else {
                        source_bytes(&mut foreign_reader, target_reader, key)?
                            .ok_or_else(|| refuse("结构源 biome 区块消失"))?
                    };
                    let staged = if local_source {
                        self.biome_overlay.get(&key)
                    } else {
                        None
                    };
                    if staged.is_some()
                        && self.biome_source.get(&key) != group.source_terrain_sha.get(&key)
                    {
                        return Err(refuse("结构源 biome 暂存已过期"));
                    }
                    biome_cache.insert(
                        key,
                        decode_sections(staged.unwrap_or(&raw))
                            .map_err(|e| refuse(e.to_string()))?,
                    );
                }
                let chunk = biome_cache
                    .get(&key)
                    .ok_or_else(|| refuse("结构源 biome 区块消失"))?;
                let matches = match expected {
                    ExpectedBiome::Named(value) => {
                        chunk.biome(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                            == Some(value)
                    }
                    ExpectedBiome::Numeric(value) => {
                        chunk.numeric_biome(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                            == Some(value)
                    }
                };
                if !matches {
                    return Err(refuse("结构源 biome 暂存已变化"));
                }
            }
            for (&key, expected) in &group.source_tick_sha {
                if cancelled() {
                    return Err(refuse("计划更新来源复核已取消"));
                }
                let (raw, identity) = if let Some(snapshot) = group.source_creation.get(&key) {
                    (snapshot.base.clone(), snapshot.identity())
                } else {
                    let raw = source_bytes(&mut foreign_reader, target_reader, key)?
                        .ok_or_else(|| refuse("计划更新源区块消失"))?;
                    let identity = nbt_sha(&raw);
                    (raw, identity)
                };
                let staged = if local_source {
                    self.be_overlay.get(&key)
                } else {
                    None
                };
                if staged.is_some() && self.be_source.get(&key) != Some(&identity) {
                    return Err(refuse("计划更新源暂存已过期"));
                }
                if &crate::scheduled_ticks::fingerprint(staged.unwrap_or(&raw))? != expected {
                    return Err(refuse("结构源计划更新已变化"));
                }
            }
            for check in &group.source_block_entities {
                let (x, y, z) = check.position;
                let key = (x.div_euclid(16), z.div_euclid(16));
                let (raw, identity) = if let Some(snapshot) = group.source_creation.get(&key) {
                    (snapshot.base.clone(), snapshot.identity())
                } else {
                    let raw = source_bytes(&mut foreign_reader, target_reader, key)?
                        .ok_or_else(|| refuse("结构源区块消失"))?;
                    let identity = nbt_sha(&raw);
                    (raw, identity)
                };
                if identity != check.terrain_sha {
                    return Err(refuse("结构源区块摘要已变化"));
                }
                let staged = if local_source && check.dimension == self.dimension {
                    self.be_overlay.get(&key)
                } else {
                    None
                };
                if staged.is_some() && self.be_source.get(&key) != Some(&check.terrain_sha) {
                    return Err(refuse("结构源 BE 暂存已过期"));
                }
                let be = crate::sections::original_block_entity_bytes(
                    staged.unwrap_or(&raw),
                    check.position,
                )
                .map_err(|e| refuse(e.to_string()))?
                .ok_or_else(|| refuse("结构源 BE 消失"))?;
                if nbt_sha(&be) != check.compound_sha {
                    return Err(refuse("结构源 BE 已变化"));
                }
                let decoded = decode_sections(&raw).map_err(|e| refuse(e.to_string()))?;
                let original = decoded
                    .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                    .ok_or_else(|| refuse("结构源方块消失"))?;
                let current = if local_source && check.dimension == self.dimension {
                    self.overlay.get(&check.position).unwrap_or(original)
                } else {
                    original
                };
                if current != &check.block {
                    return Err(refuse("结构源暂存方块已变化"));
                }
            }
            for check in &group.source_entities {
                if cancelled() {
                    return Err(refuse("结构源实体复核已取消"));
                }
                let current = source_session
                    .as_ref()
                    .unwrap_or(self)
                    .entity_input(check.chunk)?;
                if current.source != check.source || current.source_sha != check.source_sha {
                    return Err(refuse("结构源实体文件身份已变化"));
                }
                let list_path = if current.source.starts_with("entity-region:") {
                    "/Entities"
                } else {
                    let root = crate::sections::parse_root(&current.before)
                        .map_err(|e| refuse(e.to_string()))?;
                    if root.field("Level").is_some() {
                        "/Level/Entities"
                    } else {
                        "/Entities"
                    }
                };
                let compound = crate::nbt_document::raw_compound_list_item(
                    &current.before,
                    list_path,
                    check.index,
                )?;
                if nbt_sha(&compound) != check.compound_sha {
                    return Err(refuse("结构源实体载荷已变化"));
                }
            }
        }
        for (&chunk, snapshot) in &group.creation_after {
            if cancelled() {
                return Err(refuse("结构创建来源复核已取消"));
            }
            snapshot.verify_cached(target_reader)?;
            if let Some(previous) = group.creation_before.get(&chunk).and_then(Option::as_ref) {
                previous.verify_cached(target_reader)?;
            }
            let expected = if after {
                Some(snapshot)
            } else {
                group.creation_before.get(&chunk).and_then(Option::as_ref)
            };
            if self.creation.get(&chunk) != expected {
                return Err(refuse("结构创建暂存状态已变化"));
            }
        }
        for (&chunk, sha) in &group.target_sources {
            if cancelled() {
                return Err(refuse("结构复核已取消"));
            }
            if let Some(snapshot) = group.creation_after.get(&chunk) {
                if &snapshot.identity() != sha {
                    return Err(refuse("结构目标创建身份不匹配"));
                }
                continue;
            }
            let raw = target_reader
                .batch_chunk_bytes(chunk.0, chunk.1)?
                .ok_or_else(|| refuse("结构目标源区块消失"))?;
            if &nbt_sha(&raw) != sha {
                return Err(refuse("结构目标源摘要已变化"));
            }
        }
        let mut chunks = BTreeMap::new();
        for (step, change) in group.blocks.iter().enumerate() {
            if step % 4096 == 0 && cancelled() {
                return Err(refuse("结构方块源复核已取消"));
            }
            let (x, y, z) = change.position;
            let key = (x.div_euclid(16), z.div_euclid(16));
            if !chunks.contains_key(&key) {
                let decoded = if let Some(snapshot) = group.creation_after.get(&key) {
                    Some(decode_sections(&snapshot.base).map_err(|e| refuse(e.to_string()))?)
                } else {
                    crate::block_lookup::checked_original_chunk(
                        target_reader.batch_chunk_bytes(key.0, key.1)?,
                        key.0,
                        key.1,
                    )?
                };
                chunks.insert(key, decoded);
            }
            let chunk = chunks
                .get(&key)
                .and_then(Option::as_ref)
                .ok_or_else(|| refuse("结构目标区块消失"))?;
            let original = chunk
                .block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
                .ok_or_else(|| refuse("结构目标 Section 不存在"))?;
            if original != &change.original {
                return Err(refuse("结构目标源方块已变化"));
            }
            let expected = if after { &change.after } else { &change.before };
            if self.overlay.get(&change.position).unwrap_or(original) != expected {
                return Err(refuse("结构目标暂存方块已变化"));
            }
        }
        for change in &group.be_chunks {
            let (raw, identity) =
                self.structure_payload_base(target_reader, group, change.chunk, after)?;
            if identity != change.source_sha {
                return Err(refuse("结构目标源区块摘要已变化"));
            }
            let expected = if after { &change.after } else { &change.before };
            if self.be_overlay.get(&change.chunk).unwrap_or(&raw) != expected {
                return Err(refuse("结构目标 BE 暂存已变化"));
            }
        }
        for change in &group.biome_chunks {
            let (raw, identity) =
                self.structure_payload_base(target_reader, group, change.chunk, after)?;
            if identity != change.source_sha {
                return Err(refuse("结构 biome 源摘要已变化"));
            }
            let expected = if after { &change.after } else { &change.before };
            if self.biome_overlay.get(&change.chunk).unwrap_or(&raw) != expected {
                return Err(refuse("结构 biome 暂存已变化"));
            }
        }
        if after {
            let mut final_changes = BTreeMap::new();
            for entity in &group.entity_groups {
                for change in &entity.changes {
                    final_changes.insert(change.chunk, change.clone());
                }
            }
            self.check_entity_changes(&final_changes.into_values().collect::<Vec<_>>(), true)?;
        } else if !group.entity_groups.is_empty() {
            const IDENTITY_BUDGET: usize = 512 * 1024 * 1024;
            let mut ids = self.scan_entity_identities_with_creation(
                &group.creation_after,
                IDENTITY_BUDGET,
                &mut cancelled,
            )?;
            let mut working: BTreeMap<ChunkPos, Vec<u8>> = BTreeMap::new();
            for entity in &group.entity_groups {
                let (to, separate, missing) = match &entity.proof {
                    EntityProof::Clone(proof) => proof.target(),
                    EntityProof::Import(proof) => proof.target(),
                    _ => return Err(refuse("结构实体证明类型无效")),
                };
                let change = entity
                    .changes
                    .first()
                    .ok_or_else(|| refuse("结构实体目标缺失"))?;
                let target_input = self.entity_input_with_creation(
                    to,
                    group
                        .creation_after
                        .get(&to)
                        .or_else(|| self.creation.get(&to)),
                )?;
                if target_input.source != change.source
                    || target_input.source_sha != change.source_sha
                {
                    return Err(refuse("结构实体目标源已变化"));
                }
                let target = working.get(&to).unwrap_or(&target_input.before);
                if target != &change.before {
                    return Err(refuse("结构实体预览输入已变化"));
                }
                let target_bytes = if missing {
                    None
                } else {
                    Some(target.as_slice())
                };
                entity.proof.verify_dimension(&self.dimension)?;
                // 新操作绑定预检清单；已准入的历史重放原字节，并由下方原子身份更新核对真实冲突。
                let result = match &entity.proof {
                    EntityProof::Clone(proof) => {
                        if group.identity_revision.is_some() {
                            ids.verify_clone(proof)?;
                        }
                        let (from, _) = proof.source();
                        let source_input = self.entity_input(from)?;
                        let source = working.get(&from).unwrap_or(&source_input.before);
                        proof.replay(source, target_bytes)?
                    }
                    EntityProof::Import(proof) => {
                        if group.identity_revision.is_some() {
                            proof.verify_snapshot(&ids)?;
                        }
                        proof.replay(target_bytes)?
                    }
                    _ => return Err(refuse("结构实体证明类型无效")),
                };
                if result != change.after {
                    return Err(refuse("结构实体证明重放不一致"));
                }
                ids.replace_chunks(
                    &[crate::entity_editing::IdentityChunkChange {
                        dimension: &self.dimension,
                        chunk: to,
                        separate,
                        before: target_bytes,
                        after: Some(&result),
                    }],
                    IDENTITY_BUDGET,
                )?;
                working.insert(to, result);
            }
        }
        Ok(())
    }
    /// 完整预检后一次性暂存，作为单个撤销组。
    pub fn commit_structure(&mut self, group: PreparedStructure) -> Result<usize, WorldError> {
        self.commit_structure_cancellable(group, || false)
    }
    pub fn commit_structure_cancellable(
        &mut self,
        group: PreparedStructure,
        cancelled: impl FnMut() -> bool,
    ) -> Result<usize, WorldError> {
        self.commit_structure_with_progress_cancellable(group, cancelled, |_| {})
    }
    pub fn commit_structure_with_progress_cancellable(
        &mut self,
        mut group: PreparedStructure,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<usize, WorldError> {
        if group
            .identity_revision
            .as_ref()
            .is_some_and(|revision| !std::sync::Arc::ptr_eq(revision, &self.revision))
        {
            return Err(refuse("结构实体身份预检来源已改变，请重新预检"));
        }
        self.ensure_history_slot(false)?;
        if !group.has_changes() {
            return Err(refuse("结构粘贴没有实际变化"));
        }
        group.check_work(group.work_budget)?;
        progress(EditWorkProgress {
            stage: "复核完整结构源",
            selected: group.blocks.len(),
            ..Default::default()
        });
        self.check_structure_cancellable(&group, false, &mut cancelled)?;
        if cancelled() {
            return Err(refuse("结构提交已取消"));
        }
        let mut work = self.clone();
        for (&chunk, snapshot) in &group.creation_after {
            if cancelled() {
                return Err(refuse("结构载荷暂存已取消"));
            }
            work.creation.insert(chunk, snapshot.clone());
        }
        work.rebuild_creation_regions();
        for (index, change) in group.blocks.iter().enumerate() {
            if index % 4096 == 0 {
                progress(EditWorkProgress {
                    stage: "构建完整结构暂存",
                    examined: group.blocks.len(),
                    selected: group.blocks.len(),
                    changed: index,
                    slices: index.div_ceil(4096),
                });
                if cancelled() {
                    return Err(refuse("结构方块暂存已取消"));
                }
            }
            work.set_effective(change, &change.after);
        }
        for change in &group.be_chunks {
            if cancelled() {
                return Err(refuse("结构载荷暂存已取消"));
            }
            work.be_overlay.insert(change.chunk, change.after.clone());
            work.be_source
                .insert(change.chunk, change.source_sha.clone());
        }
        for change in &group.biome_chunks {
            if cancelled() {
                return Err(refuse("结构载荷暂存已取消"));
            }
            work.biome_overlay
                .insert(change.chunk, change.after.clone());
            work.biome_source
                .insert(change.chunk, change.source_sha.clone());
        }
        for entity in &group.entity_groups {
            if cancelled() {
                return Err(refuse("结构载荷暂存已取消"));
            }
            for change in &entity.changes {
                work.entity_overlay
                    .insert(change.chunk, change.after.clone());
                work.entity_sources.insert(
                    change.chunk,
                    (change.source.clone(), change.source_sha.clone()),
                );
            }
        }
        let count = group.blocks.len();
        group.identity_revision = None;
        work.record_edit(EditGroup::Structure(group))?;
        work.clear_redo();
        work.note_change();
        if cancelled() {
            return Err(refuse("结构暂存发布已取消"));
        }
        *self = work;
        progress(EditWorkProgress {
            stage: "完整结构暂存完成",
            examined: count,
            selected: count,
            changed: count,
            slices: count.div_ceil(4096),
        });
        Ok(count)
    }
    fn restore_structure(
        &mut self,
        group: &PreparedStructure,
        after: bool,
        target_reader: &mut ReadOnlyRegionCache<'_>,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<(), WorldError> {
        let mut payloads = Vec::with_capacity(group.be_chunks.len());
        let mut biome_payloads = Vec::with_capacity(group.biome_chunks.len());
        let mut entity_changes: BTreeMap<ChunkPos, (EntityChange, EntityChange)> = BTreeMap::new();
        for entity in &group.entity_groups {
            if cancelled() {
                return Err(refuse("结构载荷历史重放已取消"));
            }
            for change in &entity.changes {
                entity_changes
                    .entry(change.chunk)
                    .and_modify(|pair| pair.1 = change.clone())
                    .or_insert_with(|| (change.clone(), change.clone()));
            }
        }
        let mut entity_payloads = Vec::with_capacity(entity_changes.len());
        for (chunk, (first, last)) in entity_changes {
            let (label, raw) = self.entity_source_with_creation(
                chunk,
                group
                    .creation_after
                    .get(&chunk)
                    .or_else(|| self.creation.get(&chunk)),
            )?;
            if label != first.source || nbt_sha(&raw) != first.source_sha {
                return Err(refuse("结构实体源已变化"));
            }
            let bytes = if after { &last.after } else { &first.before };
            entity_payloads.push((
                chunk,
                (after || bytes != &raw).then(|| bytes.clone()),
                (first.source, first.source_sha),
            ));
        }
        for change in &group.be_chunks {
            if cancelled() {
                return Err(refuse("结构载荷历史重放已取消"));
            }
            let (raw, identity) =
                self.structure_payload_base(target_reader, group, change.chunk, after)?;
            if identity != change.source_sha {
                return Err(refuse("结构源摘要已变化"));
            }
            let bytes = if after { &change.after } else { &change.before };
            payloads.push((
                change.chunk,
                (after || bytes != &raw).then(|| bytes.clone()),
                change.source_sha.clone(),
            ));
        }
        for change in &group.biome_chunks {
            if cancelled() {
                return Err(refuse("结构载荷历史重放已取消"));
            }
            let (raw, identity) =
                self.structure_payload_base(target_reader, group, change.chunk, after)?;
            if identity != change.source_sha {
                return Err(refuse("结构 biome 源摘要已变化"));
            }
            let bytes = if after { &change.after } else { &change.before };
            biome_payloads.push((
                change.chunk,
                (after || bytes != &raw).then(|| bytes.clone()),
                change.source_sha.clone(),
            ));
        }
        for (step, change) in group.blocks.iter().enumerate() {
            if step % 4096 == 0 && cancelled() {
                return Err(refuse("结构历史重放已取消"));
            }
            self.set_effective(change, if after { &change.after } else { &change.before });
        }
        for (&chunk, snapshot) in &group.creation_after {
            if after {
                self.creation.insert(chunk, snapshot.clone());
            } else if let Some(previous) =
                group.creation_before.get(&chunk).and_then(Option::as_ref)
            {
                self.creation.insert(chunk, previous.clone());
            } else {
                self.creation.remove(&chunk);
            }
        }
        self.rebuild_creation_regions();
        for (chunk, bytes, sha) in payloads {
            if let Some(bytes) = bytes {
                self.be_overlay.insert(chunk, bytes);
                self.be_source.insert(chunk, sha);
            } else {
                self.be_overlay.remove(&chunk);
                self.be_source.remove(&chunk);
            }
        }
        for (chunk, bytes, sha) in biome_payloads {
            if let Some(bytes) = bytes {
                self.biome_overlay.insert(chunk, bytes);
                self.biome_source.insert(chunk, sha);
            } else {
                self.biome_overlay.remove(&chunk);
                self.biome_source.remove(&chunk);
            }
        }
        for (chunk, bytes, identity) in entity_payloads {
            if let Some(bytes) = bytes {
                self.entity_overlay.insert(chunk, bytes);
                self.entity_sources.insert(chunk, identity);
            } else {
                self.entity_overlay.remove(&chunk);
                self.entity_sources.remove(&chunk);
            }
        }
        Ok(())
    }
    /// Rotation and mirrors are applied BEFORE normal destination checks,
    /// so modified BE targets and missing Sections still fail atomically.
    pub fn prepare_paste_clipboard_file_transformed(
        &self,
        path: &Path,
        destination_min: Position,
        orientation: HorizontalTransform,
    ) -> Result<Preview, WorldError> {
        let source = clipboard::load(&self.world, &self.dimension, path)?;
        let rotated = source.transformed(orientation)?;
        self.prepare_paste_clipboard(&rotated, destination_min)
    }
    pub fn prepare_paste_clipboard_file(
        &self,
        path: &Path,
        destination_min: Position,
    ) -> Result<Preview, WorldError> {
        let clip = clipboard::load(&self.world, &self.dimension, path)?;
        self.prepare_paste_clipboard(&clip, destination_min)
    }
    /// Export exact 1.12.2/MCRegion numeric-ID source blocks to classic
    /// GZip .schematic; source/destination original BE safety still enforced.
    pub fn export_legacy_schematic(
        &self,
        bounds: Bounds,
        path: &Path,
    ) -> Result<usize, WorldError> {
        let clip = self.capture_clipboard(bounds)?;
        schematic::export_legacy(&self.world, &clip, path)?;
        Ok(clip.blocks.len())
    }
    /// Import old numeric MCEdit schematic as a block-only paste preview.
    /// Nonempty entities, TileEntities and ticks are rejected, not lost.
    pub fn prepare_import_legacy_schematic(
        &self,
        path: &Path,
        destination_min: Position,
    ) -> Result<Preview, WorldError> {
        let source = schematic::import_legacy(&self.world, &self.dimension, path)?;
        self.prepare_paste_clipboard(&source, destination_min)
    }
    /// Import Sponge v2 palette-based named blocks; DataVersion must match.
    pub fn prepare_import_sponge_v2(
        &self,
        path: &Path,
        destination_min: Position,
    ) -> Result<Preview, WorldError> {
        let block_clip = schematic::import_sponge_v2(&self.world, &self.dimension, path)?;
        self.prepare_paste_clipboard(&block_clip, destination_min)
    }
    /// Original Sponge v3 block-only paste; preserve full named properties,
    /// reject unknown BE/Entity/Biome payloads and DataVersion drift.
    pub fn prepare_import_sponge_v3(
        &self,
        path: &Path,
        destination_min: Position,
    ) -> Result<Preview, WorldError> {
        let clip = schematic::import_sponge_v3(&self.world, &self.dimension, path)?;
        self.prepare_paste_clipboard(&clip, destination_min)
    }
    pub fn export_sponge_v3(&self, bounds: Bounds, path: &Path) -> Result<usize, WorldError> {
        let clip = self.capture_clipboard(bounds)?;
        schematic::export_sponge_v3(&self.world, &clip, path)?;
        Ok(clip.blocks.len())
    }
    /// Export only BE-free named blocks to Sponge v2; no entity/biome data.
    pub fn export_sponge_v2(&self, bounds: Bounds, path: &Path) -> Result<usize, WorldError> {
        let block_clip = self.capture_clipboard(bounds)?;
        schematic::export_sponge_v2(&self.world, &block_clip, path)?;
        Ok(block_clip.blocks.len())
    }
    /// Write an external original-NBT-verified edit journal, never a world file.
    pub fn export_patch(&self, path: &Path) -> Result<usize, WorldError> {
        let changes = self.pending_patch_blocks()?;
        if changes.is_empty() {
            return Err(refuse("No pending blocks to export"));
        }
        let bytes = patch::encode(&self.world, &self.dimension, &changes)?;
        patch::save(path, &self.world, &bytes)?;
        Ok(changes.len())
    }
    /// Import one fully preflighted original-world journal as a single Undo.
    pub fn import_patch(&mut self, path: &Path) -> Result<usize, WorldError> {
        if self.has_pending_changes() {
            return Err(refuse("Export or discard existing edits before importing"));
        }
        if let Some(origin) = &self.origin {
            origin.verify_current()?;
        }
        let bytes = patch::load(path)?;
        let entries = patch::decode(&self.world, &self.dimension, &bytes)?;
        let mut reader = self.source_reader()?;
        let mut chunks = BTreeMap::new();
        let mut changes = Vec::with_capacity(entries.len());
        for (position, before, after) in entries {
            let chunk = (position.0.div_euclid(16), position.2.div_euclid(16));
            if let std::collections::btree_map::Entry::Vacant(entry) = chunks.entry(chunk) {
                entry.insert(
                    self.visible_chunk_base_cached(&mut reader, chunk.0, chunk.1)?
                        .0,
                );
            }
            let original = chunks[&chunk]
                .as_ref()
                .and_then(|chunk| {
                    chunk.block(
                        position.0.rem_euclid(16) as u8,
                        position.1,
                        position.2.rem_euclid(16) as u8,
                    )
                })
                .ok_or_else(|| refuse("日志导入目标的真实会话基底不存在"))?
                .clone();
            if self.overlay.get(&position).unwrap_or(&original) != &before {
                return Err(refuse("日志磁盘来源与当前会话的已保存内容不一致"));
            }
            changes.push(Change {
                position,
                original,
                before,
                after,
            });
        }
        drop(reader);
        let count = changes.len();
        if count > 0 {
            self.ensure_history_slot(true)?;
            for change in &changes {
                self.set_effective(change, &change.after);
            }
            self.record_edit(EditGroup::Blocks(changes))?;
            if self.undo.len() > MAX_HISTORY {
                self.undo.remove(0);
                self.undo_ids.remove(0);
            }
            self.clear_redo();
            self.note_change();
        }
        Ok(count)
    }
    pub fn undo(&mut self) -> Result<bool, WorldError> {
        self.undo_cancellable(|| false)
    }
    /// 在私有会话中完整重放到目标游标，全部成功后一次发布；错误或取消保留原会话。
    pub fn goto_history_cancellable(
        &mut self,
        target: usize,
        mut cancelled: impl FnMut() -> bool,
        mut progress: impl FnMut(EditWorkProgress),
    ) -> Result<bool, WorldError> {
        if target > self.undo.len() + self.redo.len() {
            return Err(refuse("目标修订不在当前保留历史内"));
        }
        let from = self.undo.len();
        if from == target {
            return Ok(false);
        }
        let steps = from.abs_diff(target);
        let mut work = self.clone();
        for done in 0..steps {
            let changed = if work.undo.len() > target {
                work.undo_with_cancel(&mut cancelled)?
            } else {
                work.redo_with_cancel(&mut cancelled)?
            };
            if !changed {
                return Err(refuse("历史游标与操作记录不一致"));
            }
            progress(EditWorkProgress {
                stage: "切换历史修订",
                examined: done + 1,
                selected: steps,
                ..Default::default()
            });
        }
        if cancelled() {
            return Err(refuse("历史修订发布已取消"));
        }
        *self = work;
        Ok(true)
    }
    pub fn undo_cancellable(
        &mut self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        let mut work = self.clone();
        let result = work.undo_with_cancel(&mut cancelled)?;
        if cancelled() {
            return Err(refuse("撤销发布已取消"));
        }
        if result {
            *self = work;
        }
        Ok(result)
    }
    fn undo_with_cancel(
        &mut self,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        if self.recording.is_some() {
            return Err(refuse("准备组合操作时不能撤销既有历史"));
        }
        if cancelled() {
            return Err(refuse("撤销已取消"));
        }
        if self.restore_interface_step(false) {
            return Ok(true);
        }
        if self.restore_deletion_step(false)? {
            return Ok(true);
        }
        if self.restore_region_step(false)? {
            return Ok(true);
        }
        if matches!(self.undo.last(), Some(EditGroup::Compound(_))) {
            return self.restore_compound(false, cancelled);
        }
        let world = self.world.clone();
        let dimension = self.dimension.clone();
        let mut structure_reader =
            ReadOnlyRegionCache::for_source(&world, &dimension, self.origin.as_ref())?;
        let Some(group) = self.undo.last() else {
            return Ok(false);
        };
        match group {
            EditGroup::Interface(_) => unreachable!("界面历史已独立恢复"),
            EditGroup::Deletion(_) => unreachable!("删除历史已独立恢复"),
            EditGroup::Region(_) => unreachable!("旧 Region 历史已独立恢复"),
            EditGroup::Blocks(changes) => {
                for (index, change) in changes.iter().enumerate() {
                    if index % 4096 == 0 && cancelled() {
                        return Err(refuse("撤销已取消"));
                    }
                    if self
                        .overlay
                        .get(&change.position)
                        .unwrap_or(&change.original)
                        != &change.after
                    {
                        return Err(refuse(
                            "Undo refused: staged block state no longer matches transaction",
                        ));
                    }
                }
            }
            EditGroup::Entities(group) => self.check_entity_changes(&group.changes, true)?,
            EditGroup::BlockEntityNbt(preview) => self.check_be_nbt(preview, true)?,
            EditGroup::Structure(group) => {
                self.check_structure_cached(group, true, &mut structure_reader, &mut *cancelled)?
            }
            EditGroup::BlockEntity(_) => {}
            EditGroup::Compound(_) => return Err(refuse("组合历史不能作为单步骤恢复")),
        }
        if matches!(self.undo.last(), Some(EditGroup::BlockEntity(_))) && !self.undo_be_step()? {
            return Err(refuse("方块实体撤销历史不一致"));
        }
        if let Some(EditGroup::Entities(entity)) = self.undo.last() {
            let changes = entity.changes.clone();
            self.restore_entity_changes(&changes, false)?;
        }
        if let Some(EditGroup::BlockEntityNbt(preview)) = self.undo.last() {
            let preview = preview.clone();
            self.restore_be_nbt(&preview, false)?;
        }
        if let Some(EditGroup::Structure(group)) = self.undo.last() {
            let group = group.clone();
            self.restore_structure(&group, false, &mut structure_reader, &mut *cancelled)?;
        }
        let group = self.undo.pop().expect("已检查历史");
        let identity = self.undo_ids.pop().expect("撤销操作具有稳定身份");
        match &group {
            EditGroup::Interface(_) => unreachable!("界面历史已独立恢复"),
            EditGroup::Deletion(_) => unreachable!("删除历史已独立恢复"),
            EditGroup::Region(_) => unreachable!("旧 Region 历史已独立恢复"),
            EditGroup::Blocks(changes) => {
                for (index, change) in changes.iter().enumerate() {
                    if index % 4096 == 0 && cancelled() {
                        return Err(refuse("撤销已取消"));
                    }
                    self.set_effective(change, &change.before);
                }
            }
            EditGroup::Entities(_) => {}
            EditGroup::BlockEntityNbt(_) => {}
            EditGroup::Structure(_) => {}
            EditGroup::BlockEntity(_) => {}
            EditGroup::Compound(_) => return Err(refuse("组合历史不能作为单步骤恢复")),
        }
        group.collect_source_chunks(&mut self.changed_source_chunks);
        self.redo.push(group);
        self.redo_ids.push(identity);
        self.note_change();
        Ok(true)
    }
    pub fn redo(&mut self) -> Result<bool, WorldError> {
        self.redo_cancellable(|| false)
    }
    pub fn redo_cancellable(
        &mut self,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        let mut work = self.clone();
        let result = work.redo_with_cancel(&mut cancelled)?;
        if cancelled() {
            return Err(refuse("重做发布已取消"));
        }
        if result {
            *self = work;
        }
        Ok(result)
    }
    fn redo_with_cancel(
        &mut self,
        cancelled: &mut dyn FnMut() -> bool,
    ) -> Result<bool, WorldError> {
        if self.recording.is_some() {
            return Err(refuse("准备组合操作时不能重做既有历史"));
        }
        if cancelled() {
            return Err(refuse("重做已取消"));
        }
        if self.restore_interface_step(true) {
            return Ok(true);
        }
        if self.restore_deletion_step(true)? {
            return Ok(true);
        }
        if self.restore_region_step(true)? {
            return Ok(true);
        }
        if matches!(self.redo.last(), Some(EditGroup::Compound(_))) {
            return self.restore_compound(true, cancelled);
        }
        let world = self.world.clone();
        let dimension = self.dimension.clone();
        let mut structure_reader =
            ReadOnlyRegionCache::for_source(&world, &dimension, self.origin.as_ref())?;
        let Some(group) = self.redo.last() else {
            return Ok(false);
        };
        match group {
            EditGroup::Interface(_) => unreachable!("界面历史已独立恢复"),
            EditGroup::Deletion(_) => unreachable!("删除历史已独立恢复"),
            EditGroup::Region(_) => unreachable!("旧 Region 历史已独立恢复"),
            EditGroup::Blocks(changes) => {
                for (index, change) in changes.iter().enumerate() {
                    if index % 4096 == 0 && cancelled() {
                        return Err(refuse("重做已取消"));
                    }
                    if self
                        .overlay
                        .get(&change.position)
                        .unwrap_or(&change.original)
                        != &change.before
                    {
                        return Err(refuse(
                            "Redo refused: staged block state no longer matches transaction",
                        ));
                    }
                }
            }
            EditGroup::Entities(group) => self.check_entity_changes(&group.changes, false)?,
            EditGroup::BlockEntityNbt(preview) => self.check_be_nbt(preview, false)?,
            EditGroup::Structure(group) => {
                self.check_structure_cached(group, false, &mut structure_reader, &mut *cancelled)?
            }
            EditGroup::BlockEntity(_) => {}
            EditGroup::Compound(_) => return Err(refuse("组合历史不能作为单步骤恢复")),
        }
        if matches!(self.redo.last(), Some(EditGroup::BlockEntity(_))) && !self.redo_be_step()? {
            return Err(refuse("方块实体重做历史不一致"));
        }
        if let Some(EditGroup::Entities(entity)) = self.redo.last() {
            let changes = entity.changes.clone();
            self.restore_entity_changes(&changes, true)?;
        }
        if let Some(EditGroup::BlockEntityNbt(preview)) = self.redo.last() {
            let preview = preview.clone();
            self.restore_be_nbt(&preview, true)?;
        }
        if let Some(EditGroup::Structure(group)) = self.redo.last() {
            let group = group.clone();
            self.restore_structure(&group, true, &mut structure_reader, &mut *cancelled)?;
        }
        let group = self.redo.pop().expect("已检查历史");
        match &group {
            EditGroup::Interface(_) => unreachable!("界面历史已独立恢复"),
            EditGroup::Deletion(_) => unreachable!("删除历史已独立恢复"),
            EditGroup::Region(_) => unreachable!("旧 Region 历史已独立恢复"),
            EditGroup::Blocks(changes) => {
                for (index, change) in changes.iter().enumerate() {
                    if index % 4096 == 0 && cancelled() {
                        return Err(refuse("重做已取消"));
                    }
                    self.set_effective(change, &change.after);
                }
            }
            EditGroup::Entities(_) => {}
            EditGroup::BlockEntityNbt(_) => {}
            EditGroup::Structure(_) => {}
            EditGroup::BlockEntity(_) => {}
            EditGroup::Compound(_) => return Err(refuse("组合历史不能作为单步骤恢复")),
        }
        group.collect_source_chunks(&mut self.changed_source_chunks);
        self.undo.push(group);
        self.undo_ids
            .push(self.redo_ids.pop().expect("重做操作具有稳定身份"));
        self.note_change();
        Ok(true)
    }
    /// Explicit discard only; no world files are written.
    pub fn discard(&mut self) {
        self.changed_source_chunks
            .extend(self.display_source.chunks());
        self.recording = None;
        self.origin = None;
        self.generations.clear();
        self.region_bases.clear();
        self.content_hashes.clear();
        self.saved_hashes = Default::default();
        self.saved_chunks = Default::default();
        self.saved_be_steps = Default::default();
        self.dirty_chunks.clear();
        self.pending_block_counts.clear();
        self.pending_be.clear();
        self.pending_entity.clear();
        self.pending_geometry.clear();
        self.completion = Default::default();
        self.creation.clear();
        self.creation_regions.clear();
        self.entity_overlay.clear();
        self.entity_sources.clear();
        self.biome_overlay.clear();
        self.biome_source.clear();
        self.be_overlay.clear();
        self.be_source.clear();
        self.be_undo.clear();
        self.be_redo.clear();
        self.overlay.clear();
        self.block_chunk_counts.clear();
        self.undo.clear();
        self.undo_ids.clear();
        self.clear_redo();
        self.note_change();
    }
}
