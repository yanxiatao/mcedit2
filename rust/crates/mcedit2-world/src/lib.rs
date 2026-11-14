//! Read-only, bounds-checked original Java world, NBT metadata and region index.
//!
//! IMPORTANT: No generic NBT deserialization-and-reserialization, no world
//! writes, no missing-chunk generation, no inferred DataVersion. The original
//! .mca/.mcr and modded chunk payloads are never altered by this crate.

pub mod alpha;
pub mod banner;
pub mod be_editing;
pub mod be_journal;
pub mod be_operations;
pub mod be_registry;
pub mod be_snapshot;
pub mod biome_editing;
pub mod biome_source;
pub mod block_lookup;
pub mod cat_variant;
mod chunk_completion;
pub mod chunk_creation;
pub mod chunk_display;
pub mod chunk_lifecycle;
pub mod chunk_metadata;
pub mod chunk_postprocess;
pub mod chunk_snapshot;
pub mod chunk_surface;
pub mod client_json;
pub mod clipboard;
pub mod command_visuals;
pub mod container_data;
pub mod cursor_info;
pub mod diagnostics;
pub mod edit_sources;
pub mod editing;
pub mod entity_attributes;
pub mod entity_batch;
mod entity_brain;
pub mod entity_catalog;
pub mod entity_display;
pub mod entity_editing;
pub mod entity_light;
pub mod entity_model;
pub mod entity_objects;
pub mod entity_physics;
pub mod entity_projectiles;
mod entity_references;
pub mod entity_sidecar;
mod entity_spatial;
pub mod entity_vehicles;
pub mod entity_water;
pub mod far_view;
pub mod fish_variant;
pub mod fluid_occlusion;
pub mod fluid_surface;
pub mod geometric_line;
pub mod hanging;
pub mod history;
pub mod history_source;
pub mod inspector_data;
pub mod inventory;
pub mod item_catalog;
mod item_fire;
pub mod launch;
pub mod legacy_assets;
pub mod legacy_lighting;
pub mod lifecycle_history;
mod lz4_block;
pub mod map_catalog;
pub mod map_data;
pub mod map_decorations;
pub mod map_edit;
pub mod map_history;
pub mod mixed_transaction;
pub mod native_material;
pub mod nbt_codec;
pub mod nbt_document;
mod nbt_envelope;
pub mod nbt_file;
pub mod nbt_query;
mod nbt_utf;
pub mod numeric_biome;
mod oldnumeric;
pub mod painting;
mod patch;
pub mod piston;
pub mod player_catalog;
pub mod plugins;
pub mod region_external;
pub mod region_history;
pub mod region_plan;
pub mod region_report;
pub mod region_transaction;
pub mod root_order;
pub mod saved_light;
pub mod scene_annotations;
pub mod scheduled_ticks;
pub mod schematic;
pub mod section_visibility;
pub mod sections;
pub mod shared_chunk_save;
pub mod shared_text;
pub mod sign;
pub mod skull;
pub mod source_memory;
pub mod spawn_rules;
pub mod structure_context;
pub mod structure_document;
pub mod structure_exchange;
pub mod structure_transform;
pub mod surface;
pub mod surface_visibility;
mod transaction_artifacts;
pub mod wolf_variant;
pub mod world_data;
pub mod world_list;
mod world_write_lock;

use flate2::read::{GzDecoder, ZlibDecoder};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SECTOR: u64 = 4096;
const HEADER_BYTES: usize = 8192;
const MAX_COMPRESSED: u64 = 32 * 1024 * 1024;
const MAX_NBT: u64 = 64 * 1024 * 1024;
const MAX_NBT_DEPTH: u8 = 64;

#[derive(Debug)]
pub enum WorldError {
    Io(std::io::Error),
    Invalid(String),
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Invalid(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for WorldError {}
impl From<std::io::Error> for WorldError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
type Result<T> = std::result::Result<T, WorldError>;

fn invalid(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}

/// Java 4 KiB sector allocation. An absent header entry has no slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSlot {
    pub start_sector: u32,
    pub sector_count: u8,
}

#[derive(Debug, Clone)]
pub struct RegionIndex {
    pub path: PathBuf,
    slots: [Option<ChunkSlot>; 1024],
}

impl RegionIndex {
    /// Verify the full original header, offsets and lengths without unpacking
    /// any NBT. Reads from a file handle; it never opens the file for writing.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let size = file.metadata()?.len();
        if size < HEADER_BYTES as u64 {
            return Err(invalid(format!(
                "Region header is truncated: {}",
                path.display()
            )));
        }
        let mut header = [0u8; HEADER_BYTES];
        file.read_exact(&mut header)?;
        let mut slots = [None; 1024];
        // A corrupt Region can point multiple chunk slots at overlapping
        // source sectors. Never trust those ambiguous headers even for
        // read-only 2D/3D rendering. 1024 small spans are enough; avoid
        // allocating a bitmap proportional to an attacker-controlled file.
        let mut occupied: Vec<(u64, u64, usize)> = Vec::new();
        for (index, slot) in slots.iter_mut().enumerate() {
            let start = index * 4;
            let word = u32::from_be_bytes(header[start..start + 4].try_into().unwrap());
            if word == 0 {
                continue;
            }
            let first = word >> 8;
            let count = (word & 255) as u8;
            if first < 2 || count == 0 {
                return Err(invalid(format!(
                    "Invalid region sector pointer in {} at slot {index}",
                    path.display()
                )));
            }
            let end = (first as u64 + count as u64)
                .checked_mul(SECTOR)
                .ok_or_else(|| invalid("Region sector count overflow"))?;
            if end > size {
                return Err(invalid(format!(
                    "Region entry exceeds original file length in {} at slot {index}",
                    path.display()
                )));
            }
            occupied.push((first as u64, first as u64 + count as u64, index));
            *slot = Some(ChunkSlot {
                start_sector: first,
                sector_count: count,
            });
        }
        occupied.sort_unstable_by_key(|span| span.0);
        for pair in occupied.windows(2) {
            if pair[0].1 > pair[1].0 {
                return Err(invalid(format!(
                    "Original Region source has overlapping chunk allocations: slots {} and {} in {}",
                    pair[0].2,pair[1].2,path.display()
                )));
            }
        }
        Ok(Self { path, slots })
    }

    pub fn chunk_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// The original 32x32 local chunk coordinates, 0..31 on both axes.
    pub fn slot(&self, x: u8, z: u8) -> Option<ChunkSlot> {
        if x >= 32 || z >= 32 {
            return None;
        }
        self.slots[(z as usize * 32) + x as usize]
    }

    /// Returns the original decompressed chunk NBT without normalizing,
    /// rewriting or recreating its unknown/modded tags.
    /// Compression 1 = GZip, 2 = Zlib, 3 = uncompressed; unsupported
    /// unknown compressors fail closed. External .mcc streams are read only.
    pub fn read_chunk(&self, local_x: u8, local_z: u8) -> Result<Option<Vec<u8>>> {
        // Local-only coordinates cannot identify an external c.X.Z.mcc file.
        self.read_chunk_inner(local_x, local_z, None)
    }

    /// Exact world chunk position is required for Java external streams.
    /// This is a read-only data source; the region writer still rejects .mcc.
    pub fn read_chunk_at(&self, cx: i32, cz: i32) -> Result<Option<Vec<u8>>> {
        let extension = self.path.extension().and_then(|v| v.to_str());
        let expected = match extension {
            Some("mca") => format!("r.{}.{}.mca", cx.div_euclid(32), cz.div_euclid(32)),
            Some("mcr") => format!("r.{}.{}.mcr", cx.div_euclid(32), cz.div_euclid(32)),
            _ => return Err(invalid("Original source is not a Java Region filename")),
        };
        if self.path.file_name().and_then(|v| v.to_str()) != Some(expected.as_str()) {
            return Err(invalid("Original chunk is outside this Region file"));
        }
        self.read_chunk_inner(
            cx.rem_euclid(32) as u8,
            cz.rem_euclid(32) as u8,
            Some((cx, cz)),
        )
    }

    fn read_chunk_inner(
        &self,
        local_x: u8,
        local_z: u8,
        absolute: Option<(i32, i32)>,
    ) -> Result<Option<Vec<u8>>> {
        let Some(slot) = self.slot(local_x, local_z) else {
            return Ok(None);
        };
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(slot.start_sector as u64 * SECTOR))?;
        let mut length_bytes = [0u8; 4];
        file.read_exact(&mut length_bytes)?;
        let length = u32::from_be_bytes(length_bytes) as u64;
        if length < 1 || length + 4 > slot.sector_count as u64 * SECTOR || length > MAX_COMPRESSED {
            return Err(invalid(
                "Original chunk payload length is invalid or over budget",
            ));
        }
        let mut compression = [0u8; 1];
        file.read_exact(&mut compression)?;
        let method = compression[0] & 0x7f;
        let payload = if compression[0] & 0x80 != 0 {
            // Vanilla's external marker is exactly a five-byte stub in
            // one region sector. Never guess another source chunk's NBT.
            if length != 1 || slot.sector_count != 1 {
                return Err(invalid("Malformed external Java chunk marker"));
            }
            if !matches!(method, 1..=4) {
                return Err(invalid(format!(
                    "Unsupported external compression method {method}"
                )));
            }
            let (cx, cz) = absolute.ok_or_else(|| {
                invalid("An external Java chunk requires absolute X/Z coordinates")
            })?;
            let dir = self
                .path
                .parent()
                .ok_or_else(|| invalid("No parent directory for original external chunk"))?;
            let sidecar = dir.join(format!("c.{cx}.{cz}.mcc"));
            let metadata = fs::symlink_metadata(&sidecar)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(invalid(
                    "External chunk must be a regular, non-symlink file",
                ));
            }
            if metadata.len() > MAX_COMPRESSED {
                return Err(invalid("External chunk exceeds 32 MiB compressed budget"));
            }
            let mut data = Vec::new();
            File::open(&sidecar)?
                .take(MAX_COMPRESSED + 1)
                .read_to_end(&mut data)?;
            if data.len() as u64 > MAX_COMPRESSED {
                return Err(invalid("External chunk grew beyond 32 MiB read budget"));
            }
            data
        } else {
            let mut data = vec![0u8; (length - 1) as usize];
            file.read_exact(&mut data)?;
            data
        };
        let original = match method {
            1 => decompress_bounded(GzDecoder::new(&payload[..]))?,
            2 => decompress_bounded(ZlibDecoder::new(&payload[..]))?,
            3 if payload.len() as u64 <= MAX_NBT => payload,
            4 => lz4_block::decode(&payload)?,
            3 => return Err(invalid("Uncompressed chunk NBT exceeds safety budget")),
            flag => {
                return Err(invalid(format!(
                    "Unsupported original chunk compression type {flag}; do not modify"
                )))
            }
        };
        let (_, original) = nbt_envelope::NbtEnvelope::decode(original)?;
        Ok(Some(original))
    }
}

fn decompress_bounded(reader: impl Read) -> Result<Vec<u8>> {
    decompress_limited(reader, MAX_NBT)
}
fn decompress_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    reader.take(limit + 1).read_to_end(&mut result)?;
    if result.len() as u64 > limit {
        return Err(invalid(format!("NBT 解压结果超过 {limit} 字节预算")));
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct DimensionSummary {
    pub name: String,
    pub region_dir: PathBuf,
    pub regions: usize,
    pub chunks: usize,
    pub extension: String,
}

#[derive(Debug, Clone)]
pub struct WorldSummary {
    pub root: PathBuf,
    /// 从真实 level.dat 及其启用文件数据包取得的不可变维度与 biome 来源。
    pub world_data: std::sync::Arc<world_data::Snapshot>,
    /// level.dat 的现代 WorldGenSettings.seed，或旧版 RandomSeed；缺失时保持未知。
    pub seed: Option<i64>,
    pub level_name: Option<String>,
    pub data_version: Option<i32>,
    /// Original numeric level.dat Version: 19132 MCRegion, 19133 Anvil.
    pub storage_format: Option<i32>,
    /// Original Data.Version.Name if present, including snapshot labels.
    pub version_name: Option<String>,
    pub dimensions: Vec<DimensionSummary>,
    pub warnings: Vec<String>,
}

impl WorldSummary {
    pub fn total_regions(&self) -> usize {
        self.dimensions.iter().map(|d| d.regions).sum()
    }
    pub fn total_chunks(&self) -> usize {
        self.dimensions.iter().map(|d| d.chunks).sum()
    }
}

/// Index an extracted original Java world, without decoding all the chunks.
/// Missing/invalid region files emit warnings instead of generating chunks.
pub fn inspect_world(path: impl AsRef<Path>) -> Result<WorldSummary> {
    let root = fs::canonicalize(path.as_ref())?;
    if !root.is_dir() {
        return Err(invalid(
            "Expected a directory containing original level.dat",
        ));
    }
    // Legacy adapter accepts level.dat_old as an explicit backup fallback;
    // never generates, overwrites or silently converts either file.
    let file = world_list::metadata(&root)?
        .ok_or_else(|| invalid("世界目录缺少 level.dat 或 level.dat_old"))?;
    let compressed = read_limited_file(&file, MAX_COMPRESSED)?;
    let expanded = decompress_bounded(GzDecoder::new(&compressed[..]))?;
    let metadata = nbt_metadata(&expanded)?;
    let mut discovered = discover_legacy_dimensions(&root)?;
    let custom = root.join("dimensions");
    if real_directory(&custom)? {
        let mut modern = Vec::new();
        walk_custom_dimensions(&custom, &mut modern)?;
        modern.sort_by(|a, b| a.0.cmp(&b.0));
        discovered.extend(modern);
    }
    let mut identities = std::collections::BTreeMap::new();
    for (name, path) in &discovered {
        if let Some(previous) = identities.insert(name, path) {
            return Err(invalid(format!(
                "维度身份 {name} 同时指向两个来源：{}；{}",
                previous.display(),
                path.display()
            )));
        }
    }

    let mut dimensions = Vec::new();
    let mut warnings = Vec::new();
    for (name, region_dir) in discovered {
        let mut region_files = Vec::new();
        for item in fs::read_dir(&region_dir)? {
            let item = item?;
            if !item.file_type()?.is_file() {
                continue;
            }
            let path = item.path();
            if is_region_filename(&path) {
                region_files.push(path);
            }
        }
        region_files.sort();
        let mut chunks = 0usize;
        let mut formats = Vec::new();
        for filename in &region_files {
            let ext = filename.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !formats.contains(&ext) {
                formats.push(ext);
            }
            // Several actual modded 1.21.1 Java worlds create a zero-byte
            // Region placeholder before generating any source chunk.
            // Ignore it for chunk counts and warnings, but continue to
            // flag malformed nonempty headers as corrupt.
            if filename.metadata()?.len() == 0 {
                continue;
            }
            match RegionIndex::open(filename) {
                Ok(index) => chunks += index.chunk_count(),
                Err(error) => {
                    let diagnostic = format!("{}: {error}", filename.display());
                    log::warn!("[world-index] {diagnostic}");
                    warnings.push(diagnostic);
                }
            }
        }
        dimensions.push(DimensionSummary {
            name,
            region_dir,
            regions: region_files.len(),
            chunks,
            extension: formats.join(", "),
        });
    }

    // Classic MCEdit 1 also opened original Alpha chunk-file worlds. The
    // original MCEdit2 Anvil adapter did not, but the Rust compatibility
    // target includes the earlier read-only storage. Avoid guessing if
    // region-format worlds already have a separate authoritative index.
    if dimensions.is_empty() {
        let old = alpha::find_alpha_chunks(&root)?;
        if !old.is_empty() {
            dimensions.push(DimensionSummary {
                name: alpha::DIMENSION_NAME.into(),
                region_dir: root.clone(),
                regions: 0,
                chunks: old.len(),
                extension: "original c.x.z.dat (base36)".into(),
            });
        }
    }

    let world_data = world_data::Snapshot::from_level(
        &entity_editing::DiskIdentitySource,
        &root,
        metadata.data_version,
        &file,
        &compressed,
        &expanded,
    )?;
    warnings.extend(world_data.diagnostics.iter().cloned());
    Ok(WorldSummary {
        root,
        world_data,
        seed: metadata.seed.or(metadata.legacy_seed),
        level_name: metadata.name,
        data_version: metadata.data_version,
        storage_format: metadata.storage_format,
        version_name: metadata.version_name,
        dimensions,
        warnings,
    })
}

/// 不跟随目录链接；缺失是没有该存储布局，其余 I/O 错误正常传递。
fn real_directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// 原 AnvilWorldFolder._findDimensions：世界根目录的一层子目录含 region 即为来源。
/// ## 开头目录属于原版工作目录；不进入其中，也不递归普通子目录。
fn discover_legacy_dimensions(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut standard = std::collections::BTreeMap::new();
    let overworld = root.join("region");
    if real_directory(&overworld)? {
        standard.insert("minecraft:overworld", overworld);
    }
    let mut custom = Vec::new();
    for item in fs::read_dir(root)? {
        let item = item?;
        if !item.file_type()?.is_dir() {
            continue;
        }
        let filename = item.file_name();
        if filename.to_str().is_some_and(|name| name.starts_with("##")) {
            continue;
        }
        let directory = item.path();
        let region = directory.join("region");
        if !real_directory(&region)? {
            continue;
        }
        let name = filename.to_str().ok_or_else(|| {
            invalid(format!(
                "旧式维度目录名称不是 UTF-8：{}",
                directory.display()
            ))
        })?;
        match name {
            "DIM-1" => {
                standard.insert("minecraft:the_nether", region);
            }
            "DIM1" => {
                standard.insert("minecraft:the_end", region);
            }
            _ => {
                // 数字目录保留已有玩家 Int Dimension 的显式 legacy:n 身份。
                // 非标准拼写和任意命名目录使用真实原名，不猜成现代资源 ID。
                let number = name
                    .strip_prefix("DIM")
                    .and_then(|id| id.parse::<i32>().ok());
                let identity = match number {
                    Some(id) if name == format!("DIM{id}") => format!("legacy:{id}"),
                    _ => name.to_owned(),
                };
                custom.push((identity, region));
            }
        }
    }
    custom.sort_by(|a, b| a.0.cmp(&b.0));
    let mut found = Vec::with_capacity(standard.len() + custom.len());
    for id in [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ] {
        if let Some(path) = standard.remove(id) {
            found.push((id.to_owned(), path));
        }
    }
    found.extend(custom);
    Ok(found)
}

fn is_region_filename(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let mut parts = name.split('.');
    if parts.next() != Some("r") {
        return false;
    }
    for _ in 0..2 {
        if parts.next().and_then(|p| p.parse::<i32>().ok()).is_none() {
            return false;
        }
    }
    matches!(parts.next(), Some("mca") | Some("mcr")) && parts.next().is_none()
}

/// Custom modern Java dimensions: dimensions/namespace/name[/nested]/region.
/// Never follow links, traverse the world root, or inspect arbitrary folders.
fn walk_custom_dimensions(base: &Path, result: &mut Vec<(String, PathBuf)>) -> Result<()> {
    let mut pending = vec![base.to_path_buf()];
    while let Some(current) = pending.pop() {
        for item in fs::read_dir(&current)? {
            let item = item?;
            if !item.file_type()?.is_dir() {
                continue;
            }
            let path = item.path();
            if path.file_name().and_then(|s| s.to_str()) == Some("region") {
                let rel = current
                    .strip_prefix(base)
                    .map_err(|_| invalid("Custom dimension directory escaped root"))?;
                let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy());
                if let Some(namespace) = parts.next() {
                    let kind: Vec<_> = parts.map(|s| s.to_string()).collect();
                    if !kind.is_empty() {
                        result.push((format!("{namespace}:{}", kind.join("/")), path));
                    }
                }
            } else {
                pending.push(path);
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct Metadata {
    seed: Option<i64>,
    legacy_seed: Option<i64>,
    data_version: Option<i32>,
    storage_format: Option<i32>,
    version_name: Option<String>,
    name: Option<String>,
}

struct NbtReader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> NbtReader<'a> {
    fn next(&mut self, amount: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(amount)
            .ok_or_else(|| invalid("NBT position overflow"))?;
        let data = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| invalid("Truncated or malformed original NBT tag"))?;
        self.position = end;
        Ok(data)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.next(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.next(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.next(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String> {
        let count = self.u16()? as usize;
        crate::nbt_utf::decode(self.next(count)?)
    }
    fn skip(&mut self, size: usize) -> Result<()> {
        self.next(size)?;
        Ok(())
    }
    fn list_count(&mut self) -> Result<usize> {
        let v = self.i32()?;
        if !(0..=16_777_216).contains(&v) {
            return Err(invalid("NBT array/list count invalid or over limit"));
        }
        Ok(v as usize)
    }
    fn array(&mut self, item_size: usize) -> Result<()> {
        let count = self.list_count()?;
        let bytes = count
            .checked_mul(item_size)
            .ok_or_else(|| invalid("NBT array size overflow"))?;
        self.skip(bytes)
    }
    fn payload(&mut self, kind: u8, depth: u8) -> Result<()> {
        if depth > MAX_NBT_DEPTH {
            return Err(invalid("NBT nesting exceeds 64"));
        }
        match kind {
            1 => self.skip(1),
            2 => self.skip(2),
            3 | 5 => self.skip(4),
            4 | 6 => self.skip(8),
            7 => self.array(1),
            8 => {
                self.string()?;
                Ok(())
            }
            9 => {
                let element = self.u8()?;
                let count = self.list_count()?;
                if (element == 0 && count != 0) || element > 12 {
                    return Err(invalid("NBT List has invalid element type"));
                }
                for _ in 0..count {
                    self.payload(element, depth + 1)?;
                }
                Ok(())
            }
            10 => self.compound(depth + 1, false, &mut Metadata::default()),
            11 => self.array(4),
            12 => self.array(8),
            _ => Err(invalid(format!("Unknown NBT tag type {kind}"))),
        }
    }
    fn version_metadata(&mut self, depth: u8, meta: &mut Metadata) -> Result<()> {
        if depth > MAX_NBT_DEPTH {
            return Err(invalid("NBT Version depth exceeded"));
        }
        loop {
            let tag = self.u8()?;
            if tag == 0 {
                return Ok(());
            }
            let key = self.string()?;
            if key == "Name" && tag == 8 {
                meta.version_name = Some(self.string()?);
            } else {
                self.payload(tag, depth + 1)?;
            }
        }
    }
    fn worldgen_metadata(&mut self, depth: u8, meta: &mut Metadata) -> Result<()> {
        if depth > MAX_NBT_DEPTH {
            return Err(invalid("NBT WorldGenSettings depth exceeded"));
        }
        loop {
            let tag = self.u8()?;
            if tag == 0 {
                return Ok(());
            }
            let key = self.string()?;
            if key == "seed" {
                if tag != 4 {
                    return Err(invalid("WorldGenSettings.seed 必须为 Long"));
                }
                meta.seed = Some(i64::from_be_bytes(self.next(8)?.try_into().unwrap()));
            } else {
                self.payload(tag, depth + 1)?;
            }
        }
    }
    fn compound(&mut self, depth: u8, data: bool, meta: &mut Metadata) -> Result<()> {
        if depth > MAX_NBT_DEPTH {
            return Err(invalid("NBT nesting exceeds 64"));
        }
        loop {
            let tag = self.u8()?;
            if tag == 0 {
                return Ok(());
            }
            let name = self.string()?;
            match (data, name.as_str(), tag) {
                (true, "WorldGenSettings", 10) => self.worldgen_metadata(depth + 1, meta)?,
                (true, "RandomSeed", 4) => {
                    meta.legacy_seed = Some(i64::from_be_bytes(self.next(8)?.try_into().unwrap()));
                }
                (true, "WorldGenSettings", _) | (true, "RandomSeed", _) => {
                    return Err(invalid("level.dat 世界种子容器或数值类型无效"));
                }
                (true, "DataVersion", 3) => {
                    meta.data_version = Some(self.i32()?);
                }
                (true, "LevelName", 8) => {
                    meta.name = Some(self.string()?);
                }
                (true, "version", 3) | (true, "Version", 3) => {
                    meta.storage_format = Some(self.i32()?);
                }
                (true, "Version", 10) => {
                    self.version_metadata(depth + 1, meta)?;
                }
                (false, "Data", 10) if depth == 0 => {
                    self.compound(depth + 1, true, meta)?;
                }
                _ => self.payload(tag, depth + 1)?,
            }
        }
    }
}
fn nbt_metadata(input: &[u8]) -> Result<Metadata> {
    let mut n = NbtReader {
        bytes: input,
        position: 0,
    };
    if n.u8()? != 10 {
        return Err(invalid("level.dat has no original root Compound"));
    }
    n.string()?;
    let mut meta = Metadata::default();
    n.compound(0, false, &mut meta)?;
    if n.position != input.len() {
        return Err(invalid("level.dat has trailing unrecognized NBT bytes"));
    }
    Ok(meta)
}

/// Original MCEdit2 positional arguments can point to a containing folder,
/// level.dat or level.dat_old. Unsupported schematics and non-Java files are
/// rejected rather than pretending they were opened successfully.
pub fn inspect_input(path: impl AsRef<Path>) -> Result<WorldSummary> {
    let path = path.as_ref();
    match path.file_name() {
        Some(name)
            if (name == std::ffi::OsStr::new("level.dat")
                || name == std::ffi::OsStr::new("level.dat_old"))
                && path.is_file() =>
        {
            inspect_world(
                path.parent()
                    .ok_or_else(|| invalid("World metadata file has no parent directory"))?,
            )
        }
        _ => inspect_world(path),
    }
}

/// Future snapshot DataVersions are not inferred from numeric proximity.
/// The current known public snapshot is 26.4 Snapshot 5, DataVersion 5125.
/// A matching chunk tag schema can be read, but that does not authorize
/// writes until all related entities/BEs/biomes/region storage are validated.
pub fn compatibility_label(summary: &WorldSummary) -> &'static str {
    match summary.data_version {
        Some(5125) => "26.4 Snapshot 5: experimental READ ONLY",
        Some(5023) => "26.3 release: experimental READ ONLY",
        Some(v) if v > 5125 => "future or unknown snapshot: inspect-only, no writes",
        Some(v) if v >= 2844 => "modern Java 1.18+ format",
        Some(v) if v >= 1451 => "named Java block states",
        Some(_) => "numeric Java Anvil",
        None if summary.storage_format == Some(19132) => "original MCRegion format 19132",
        None if summary.storage_format == Some(19133) => "original numeric Anvil format 19133",
        None => "legacy Java world without DataVersion",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, write::ZlibEncoder, Compression};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let name = format!(
                "mcedit2-rust-fixture-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let dir = std::env::temp_dir().join(name);
            fs::create_dir_all(dir.join("region")).unwrap();
            Self(dir)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tag(kind: u8, name: &str, payload: &[u8]) -> Vec<u8> {
        let mut result = vec![kind];
        result.extend((name.len() as u16).to_be_bytes());
        result.extend(name.as_bytes());
        result.extend(payload);
        result
    }
    fn example_nbt() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend(tag(3, "DataVersion", &3955_i32.to_be_bytes()));
        let level_name = b"Native Rust Test";
        let mut name_payload = (level_name.len() as u16).to_be_bytes().to_vec();
        name_payload.extend(level_name);
        data.extend(tag(8, "LevelName", &name_payload));
        data.push(0); // original inner TAG_End
        let mut root = tag(10, "Data", &data);
        root.push(0); // original root TAG_End
        tag(10, "", &root)
    }
    fn write_fixture_world(temp: &Fixture) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&example_nbt()).unwrap();
        fs::write(temp.0.join("level.dat"), encoder.finish().unwrap()).unwrap();
        let chunk = example_nbt();
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(&chunk).unwrap();
        let compressed = zlib.finish().unwrap();
        let mut region = vec![0u8; 3 * SECTOR as usize];
        region[0..4].copy_from_slice(&((2u32 << 8) | 1).to_be_bytes());
        let length = (compressed.len() as u32) + 1;
        let at = (2 * SECTOR) as usize;
        region[at..at + 4].copy_from_slice(&length.to_be_bytes());
        region[at + 4] = 2;
        region[at + 5..at + 5 + compressed.len()].copy_from_slice(&compressed);
        fs::write(temp.0.join("region/r.0.0.mca"), region).unwrap();
        chunk
    }

    #[test]
    fn nbt_metadata_preserves_actual_version_and_name() {
        let metadata = nbt_metadata(&example_nbt()).unwrap();
        assert_eq!(metadata.data_version, Some(3955));
        assert_eq!(metadata.name.as_deref(), Some("Native Rust Test"));
    }

    #[test]
    fn nbt_rejects_truncated_and_unknown_tag_types() {
        let mut original = example_nbt();
        original.pop();
        assert!(nbt_metadata(&original).is_err());
        let invalid = tag(10, "", &[99, 0, 0, 0]);
        assert!(nbt_metadata(&invalid).is_err());
    }

    #[test]
    fn original_region_index_and_exact_zlib_nbt_are_read_only() {
        let temp = Fixture::new();
        let chunk = write_fixture_world(&temp);
        let source = temp.0.join("region/r.0.0.mca");
        let before = fs::read(&source).unwrap();
        let index = RegionIndex::open(&source).unwrap();
        assert_eq!(index.chunk_count(), 1);
        assert_eq!(index.slot(0, 0).unwrap().start_sector, 2);
        assert!(index.slot(32, 0).is_none());
        assert_eq!(index.read_chunk(0, 0).unwrap().unwrap(), chunk);
        assert!(index.read_chunk(2, 1).unwrap().is_none());
        let world = inspect_world(&temp.0).unwrap();
        assert_eq!(world.total_chunks(), 1);
        assert_eq!(world.total_regions(), 1);
        assert_eq!(world.data_version, Some(3955));
        assert_eq!(world.dimensions[0].name, "minecraft:overworld");
        assert!(world.warnings.is_empty());
        assert_eq!(fs::read(source).unwrap(), before);
        for i in 0..130 {
            fs::create_dir_all(temp.0.join(format!("dimensions/test/d{i}/region"))).unwrap();
        }
        let nested = "deep/a/b/c/d/e/f/g/h/i/j";
        fs::create_dir_all(temp.0.join(format!("dimensions/test/{nested}/region"))).unwrap();
        let world = inspect_world(&temp.0).unwrap();
        assert_eq!(world.dimensions.len(), 132);
        assert!(world
            .dimensions
            .iter()
            .any(|d| d.name == format!("test:{nested}")));
        assert_eq!(world.total_chunks(), 1);
    }

    #[test]
    fn truncated_region_is_skipped_with_warning_not_a_generated_chunk() {
        let temp = Fixture::new();
        write_fixture_world(&temp);
        let bad = temp.0.join("region/r.1.0.mca");
        fs::write(&bad, [1, 2, 3]).unwrap();
        let world = inspect_world(&temp.0).unwrap();
        assert_eq!(world.total_regions(), 2);
        assert_eq!(world.total_chunks(), 1);
        assert_eq!(world.warnings.len(), 1);
        assert!(RegionIndex::open(bad).is_err());
        for i in 2..132 {
            fs::write(temp.0.join(format!("region/r.{i}.0.mca")), [1, 2, 3]).unwrap();
        }
        let world = inspect_world(&temp.0).unwrap();
        assert_eq!(world.total_regions(), 132);
        assert_eq!(world.total_chunks(), 1);
        assert_eq!(world.warnings.len(), 131);
    }

    #[test]
    fn invalid_or_out_of_bounds_sector_is_never_read() {
        let temp = Fixture::new();
        write_fixture_world(&temp);
        let path = temp.0.join("region/r.0.0.mca");
        let mut raw = fs::read(&path).unwrap();
        raw[0..4].copy_from_slice(&((250u32 << 8) | 2).to_be_bytes());
        fs::write(&path, raw).unwrap();
        assert!(RegionIndex::open(path).is_err());
    }

    #[test]
    fn overlapping_original_region_slots_are_rejected_before_rendering() {
        let temp = Fixture::new();
        write_fixture_world(&temp);
        let path = temp.0.join("region/r.0.0.mca");
        let original = fs::read(&path).unwrap();
        // Both slots claim the same original physical sector.
        let mut duplicate = original.clone();
        duplicate[4..8].copy_from_slice(&((2u32 << 8) | 1).to_be_bytes());
        fs::write(&path, &duplicate).unwrap();
        assert!(RegionIndex::open(&path)
            .unwrap_err()
            .to_string()
            .contains("overlapping chunk allocations"));
        // Partial overlap also fails, even when both pointers fit the file.
        let mut partial = vec![0u8; 4 * SECTOR as usize];
        partial[..original.len()].copy_from_slice(&original);
        partial[0..4].copy_from_slice(&((2u32 << 8) | 2).to_be_bytes());
        partial[4..8].copy_from_slice(&((3u32 << 8) | 1).to_be_bytes());
        fs::write(&path, &partial).unwrap();
        assert!(RegionIndex::open(&path).is_err());
        // Adjacent non-overlapping allocations are valid header structure.
        partial[0..4].copy_from_slice(&((2u32 << 8) | 1).to_be_bytes());
        fs::write(&path, &partial).unwrap();
        let safe = RegionIndex::open(&path).unwrap();
        assert_eq!(safe.chunk_count(), 2);
        assert_eq!(safe.slot(1, 0).unwrap().start_sector, 3);
        assert_eq!(fs::read(&path).unwrap(), partial);
    }

    #[test]
    fn original_region_method_four_decodes_inline_and_external_lz4() {
        let temp = Fixture::new();
        let raw = write_fixture_world(&temp);
        let path = temp.0.join("region/r.0.0.mca");
        let mut stream = b"LZ4Block".to_vec();
        stream.push(0x16); // Java raw LZ4Block
        stream.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        stream.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        let checksum = xxhash_rust::xxh32::xxh32(&raw, 0x9747_b28c) & 0x0fff_ffff;
        stream.extend_from_slice(&checksum.to_le_bytes());
        stream.extend_from_slice(&raw);
        stream.extend_from_slice(b"LZ4Block");
        stream.push(0x16);
        stream.extend_from_slice(&[0u8; 12]);
        let mut bytes = fs::read(&path).unwrap();
        let at = (2 * SECTOR) as usize;
        assert!(stream.len() + 5 < SECTOR as usize);
        bytes[at..at + 4].copy_from_slice(&((stream.len() + 1) as u32).to_be_bytes());
        bytes[at + 4] = 4;
        bytes[at + 5..at + 5 + stream.len()].copy_from_slice(&stream);
        fs::write(&path, &bytes).unwrap();
        assert_eq!(
            RegionIndex::open(&path)
                .unwrap()
                .read_chunk_at(0, 0)
                .unwrap()
                .unwrap(),
            raw
        );
        bytes[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
        bytes[at + 4] = 0x84; // external Java method 4
        fs::write(&path, &bytes).unwrap();
        let sidecar = temp.0.join("region/c.0.0.mcc");
        fs::write(&sidecar, &stream).unwrap();
        assert_eq!(
            RegionIndex::open(&path)
                .unwrap()
                .read_chunk_at(0, 0)
                .unwrap()
                .unwrap(),
            raw
        );
        assert_eq!(fs::read(&sidecar).unwrap(), stream);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn java_external_mcc_is_read_only_and_rejects_bad_marker() {
        let temp = Fixture::new();
        let original = write_fixture_world(&temp);
        let region = temp.0.join("region/r.0.0.mca");
        let mut bytes = fs::read(&region).unwrap();
        let at = (2 * SECTOR) as usize;
        bytes[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
        bytes[at + 4] = 0x80 | 2;
        fs::write(&region, &bytes).unwrap();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original).unwrap();
        let sidecar = temp.0.join("region/c.0.0.mcc");
        let compressed = encoder.finish().unwrap();
        fs::write(&sidecar, &compressed).unwrap();
        let index = RegionIndex::open(&region).unwrap();
        assert!(index.read_chunk(0, 0).is_err());
        assert!(index.read_chunk_at(32, 0).is_err());
        assert_eq!(index.read_chunk_at(0, 0).unwrap().unwrap(), original);
        let world = inspect_world(&temp.0).unwrap();
        assert_eq!(
            crate::block_lookup::read_world_chunk_bytes(&world, "minecraft:overworld", 0, 0)
                .unwrap()
                .unwrap(),
            original
        );
        assert_eq!(fs::read(&region).unwrap(), bytes);
        assert_eq!(fs::read(&sidecar).unwrap(), compressed);
        fs::remove_file(&sidecar).unwrap();
        assert!(index.read_chunk_at(0, 0).is_err());
        fs::write(&sidecar, &compressed).unwrap();
        bytes[at..at + 4].copy_from_slice(&2u32.to_be_bytes());
        fs::write(&region, bytes).unwrap();
        assert!(RegionIndex::open(&region)
            .unwrap()
            .read_chunk_at(0, 0)
            .is_err());
    }

    #[test]
    fn negative_external_mcc_coordinates_are_not_local_slot_names() {
        let temp = Fixture::new();
        let region = temp.0.join("region/r.-1.0.mca");
        let mut bytes = vec![0u8; 3 * SECTOR as usize];
        let offset = (7 * 32 + 31) * 4;
        bytes[offset..offset + 4].copy_from_slice(&((2u32 << 8) | 1).to_be_bytes());
        let at = (2 * SECTOR) as usize;
        bytes[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
        bytes[at + 4] = 0x80 | 3;
        fs::write(&region, bytes).unwrap();
        let payload = example_nbt();
        fs::write(temp.0.join("region/c.-1.7.mcc"), &payload).unwrap();
        let index = RegionIndex::open(&region).unwrap();
        assert_eq!(index.read_chunk_at(-1, 7).unwrap().unwrap(), payload);
        assert!(index.read_chunk_at(31, 7).is_err());
    }

    #[test]
    fn region_filename_accepts_negative_positions_and_refuses_suspicious_extensions() {
        assert!(is_region_filename(Path::new("r.-1.3.mcr")));
        assert!(is_region_filename(Path::new("r.12.-9.mca")));
        assert!(!is_region_filename(Path::new("r.0.0.mca.tmp")));
        assert!(!is_region_filename(Path::new("notes.mca")));
    }
}

pub mod entity_name;

pub mod entity_animation;
pub mod entity_equipment;
pub mod entity_zombification;
pub mod piglin_appearance;

pub mod armor_trim;
mod registry_tags;

/// Bound the allocation while reading, including files growing after metadata.
pub(crate) fn read_limited_file(path: &std::path::Path, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit {
        return Err(invalid(format!(
            "文件不是普通文件或超过读取预算：{}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("文件在读取期间超过预算"));
    }
    Ok(bytes)
}
