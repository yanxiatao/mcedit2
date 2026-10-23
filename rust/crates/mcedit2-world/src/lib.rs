//! Read-only, bounds-checked original Java world, NBT metadata and region index.
//!
//! IMPORTANT: No generic NBT deserialization-and-reserialization, no world
//! writes, no missing-chunk generation, no inferred DataVersion. The original
//! .mca/.mcr and modded chunk payloads are never altered by this crate.

pub mod launch;
pub mod sections;
pub mod alpha;
pub mod block_lookup;
pub mod editing;
pub mod surface;
pub mod plugins;
pub mod be_snapshot;
pub mod be_journal;
pub mod be_operations;
pub mod region_report;
pub mod clipboard;
pub mod schematic;
pub mod region_plan;
pub mod region_transaction;
mod patch;
mod oldnumeric;

use flate2::read::{GzDecoder, ZlibDecoder};
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SECTOR: u64 = 4096;
const HEADER_BYTES: usize = 8192;
const MAX_COMPRESSED: u64 = 32 * 1024 * 1024;
const MAX_NBT: u64 = 64 * 1024 * 1024;
const MAX_REGION_FILES: usize = 16_384;
const MAX_DIMENSIONS: usize = 128;
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
    fn from(value: std::io::Error) -> Self { Self::Io(value) }
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
            return Err(invalid(format!("Region header is truncated: {}", path.display())));
        }
        let mut header = [0u8; HEADER_BYTES];
        file.read_exact(&mut header)?;
        let mut slots = [None; 1024];
        for (index, slot) in slots.iter_mut().enumerate() {
            let start = index * 4;
            let word = u32::from_be_bytes(header[start..start + 4].try_into().unwrap());
            if word == 0 { continue; }
            let first = word >> 8;
            let count = (word & 255) as u8;
            if first < 2 || count == 0 {
                return Err(invalid(format!(
                    "Invalid region sector pointer in {} at slot {index}", path.display()
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
            *slot = Some(ChunkSlot { start_sector: first, sector_count: count });
        }
        Ok(Self { path, slots })
    }

    pub fn chunk_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// The original 32x32 local chunk coordinates, 0..31 on both axes.
    pub fn slot(&self, x: u8, z: u8) -> Option<ChunkSlot> {
        if x >= 32 || z >= 32 { return None; }
        self.slots[(z as usize * 32) + x as usize]
    }

    /// Returns the original decompressed chunk NBT without normalizing,
    /// rewriting or recreating its unknown/modded tags.
    /// Compression 1 = GZip, 2 = Zlib, 3 = uncompressed; unsupported
    /// compressors and out-of-region sidecar references fail closed.
    pub fn read_chunk(&self, local_x: u8, local_z: u8) -> Result<Option<Vec<u8>>> {
        let Some(slot) = self.slot(local_x, local_z) else { return Ok(None); };
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(slot.start_sector as u64 * SECTOR))?;
        let mut length_bytes = [0u8; 4];
        file.read_exact(&mut length_bytes)?;
        let length = u32::from_be_bytes(length_bytes) as u64;
        if length < 1 || length + 4 > slot.sector_count as u64 * SECTOR
            || length > MAX_COMPRESSED
        {
            return Err(invalid("Original chunk payload length is invalid or over budget"));
        }
        let mut compression = [0u8; 1];
        file.read_exact(&mut compression)?;
        let mut payload = vec![0u8; (length - 1) as usize];
        file.read_exact(&mut payload)?;
        let original = match compression[0] {
            1 => decompress_bounded(GzDecoder::new(&payload[..]))?,
            2 => decompress_bounded(ZlibDecoder::new(&payload[..]))?,
            3 if payload.len() as u64 <= MAX_NBT => payload,
            3 => return Err(invalid("Uncompressed chunk NBT exceeds safety budget")),
            flag => return Err(invalid(format!(
                "Unsupported original chunk compression type {flag}; do not modify"
            ))),
        };
        Ok(Some(original))
    }
}

fn decompress_bounded(reader: impl Read) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    reader.take(MAX_NBT + 1).read_to_end(&mut result)?;
    if result.len() as u64 > MAX_NBT {
        return Err(invalid("Decompressed original NBT exceeds 64 MiB safety budget"));
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
        return Err(invalid("Expected a directory containing original level.dat"));
    }
    // Legacy adapter accepts level.dat_old as an explicit backup fallback;
    // never generates, overwrites or silently converts either file.
    let primary = root.join("level.dat");
    let backup = root.join("level.dat_old");
    let file = if primary.is_file() { primary } else { backup };
    let compressed = fs::read(&file)?;
    if compressed.len() as u64 > MAX_COMPRESSED {
        return Err(invalid("Original level.dat exceeds compressed safety budget"));
    }
    let expanded = decompress_bounded(GzDecoder::new(&compressed[..]))?;
    let metadata = nbt_metadata(&expanded)?;
    let mut discovered: Vec<(String, PathBuf)> = Vec::new();
    for (id, path) in [
        ("minecraft:overworld", root.join("region")),
        ("minecraft:the_nether", root.join("DIM-1/region")),
        ("minecraft:the_end", root.join("DIM1/region")),
    ] {
        if path.is_dir() && !fs::symlink_metadata(&path)?.file_type().is_symlink() {
            discovered.push((id.into(), path));
        }
    }
    let custom = root.join("dimensions");
    if custom.is_dir() && !fs::symlink_metadata(&custom)?.file_type().is_symlink() {
        walk_custom_dimensions(&custom, &custom, 0, &mut discovered)?;
    }
    if discovered.len() > MAX_DIMENSIONS {
        return Err(invalid("World has too many discoverable dimensions"));
    }

    let mut dimensions = Vec::new();
    let mut warnings = Vec::new();
    for (name, region_dir) in discovered {
        let mut region_files = Vec::new();
        for item in fs::read_dir(&region_dir)? {
            let item = item?;
            if !item.file_type()?.is_file() { continue; }
            let path = item.path();
            if is_region_filename(&path) { region_files.push(path); }
        }
        if region_files.len() > MAX_REGION_FILES {
            return Err(invalid(format!("Dimension {name} exceeds region index limit")));
        }
        region_files.sort();
        let mut chunks = 0usize;
        let mut formats = Vec::new();
        for filename in &region_files {
            let ext = filename.extension().and_then(|s| s.to_str()).unwrap_or("");
            if !formats.contains(&ext) { formats.push(ext); }
            match RegionIndex::open(filename) {
                Ok(index) => chunks += index.chunk_count(),
                Err(error) if warnings.len() < 128 => warnings.push(format!(
                    "{}: {error}", filename.display()
                )),
                Err(_) => {}
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
                name: "minecraft:overworld (Alpha chunk files)".into(),
                region_dir: root.clone(),
                regions: 0,
                chunks: old.len(),
                extension: "original c.x.z.dat (base36)".into(),
            });
        }
    }

    Ok(WorldSummary {
        root,
        level_name: metadata.name,
        data_version: metadata.data_version,
        storage_format: metadata.storage_format,
        version_name: metadata.version_name,
        dimensions,
        warnings,
    })
}

fn is_region_filename(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let mut parts = name.split('.');
    if parts.next() != Some("r") { return false; }
    for _ in 0..2 {
        if !matches!(parts.next().and_then(|p| p.parse::<i32>().ok()), Some(_)) {
            return false;
        }
    }
    matches!(parts.next(), Some("mca") | Some("mcr")) && parts.next().is_none()
}

/// Custom modern Java dimensions: dimensions/namespace/name[/nested]/region.
/// Never follow links, traverse the world root, or inspect arbitrary folders.
fn walk_custom_dimensions(
    base: &Path, current: &Path, depth: u8,
    result: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    if depth >= 8 { return Ok(()); }
    for item in fs::read_dir(current)? {
        let item = item?;
        if !item.file_type()?.is_dir() { continue; }
        let path = item.path();
        if path.file_name().and_then(|s| s.to_str()) == Some("region") {
            let rel = current.strip_prefix(base)
                .map_err(|_| invalid("Custom dimension directory escaped root"))?;
            let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy());
            if let Some(namespace) = parts.next() {
                let kind: Vec<_> = parts.map(|s| s.to_string()).collect();
                if !kind.is_empty() {
                    if result.len() >= MAX_DIMENSIONS {
                        return Err(invalid("Custom dimension safety limit exceeded"));
                    }
                    result.push((format!("{namespace}:{}", kind.join("/")), path));
                }
            }
        } else {
            walk_custom_dimensions(base, &path, depth + 1, result)?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Metadata {
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
        let end = self.position.checked_add(amount)
            .ok_or_else(|| invalid("NBT position overflow"))?;
        let data = self.bytes.get(self.position..end)
            .ok_or_else(|| invalid("Truncated or malformed original NBT tag"))?;
        self.position = end;
        Ok(data)
    }
    fn u8(&mut self) -> Result<u8> { Ok(self.next(1)?[0]) }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.next(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.next(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String> {
        let count = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.next(count)?).to_string())
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
        let bytes = count.checked_mul(item_size)
            .ok_or_else(|| invalid("NBT array size overflow"))?;
        self.skip(bytes)
    }
    fn payload(&mut self, kind: u8, depth: u8) -> Result<()> {
        if depth > MAX_NBT_DEPTH { return Err(invalid("NBT nesting exceeds 64")); }
        match kind {
            1 => self.skip(1),
            2 => self.skip(2),
            3 | 5 => self.skip(4),
            4 | 6 => self.skip(8),
            7 => self.array(1),
            8 => { self.string()?; Ok(()) },
            9 => {
                let element = self.u8()?;
                let count = self.list_count()?;
                if (element == 0 && count != 0) || element > 12 {
                    return Err(invalid("NBT List has invalid element type"));
                }
                for _ in 0..count { self.payload(element, depth + 1)?; }
                Ok(())
            }
            10 => self.compound(depth + 1, false, &mut Metadata::default()),
            11 => self.array(4),
            12 => self.array(8),
            _ => Err(invalid(format!("Unknown NBT tag type {kind}"))),
        }
    }
    fn version_metadata(&mut self, depth: u8, meta: &mut Metadata) -> Result<()> {
        if depth > MAX_NBT_DEPTH { return Err(invalid("NBT Version depth exceeded")); }
        loop {
            let tag = self.u8()?;
            if tag == 0 { return Ok(()); }
            let key = self.string()?;
            if key == "Name" && tag == 8 {
                meta.version_name = Some(self.string()?);
            } else {
                self.payload(tag, depth + 1)?;
            }
        }
    }
    fn compound(&mut self, depth: u8, data: bool, meta: &mut Metadata) -> Result<()> {
        if depth > MAX_NBT_DEPTH { return Err(invalid("NBT nesting exceeds 64")); }
        loop {
            let tag = self.u8()?;
            if tag == 0 { return Ok(()); }
            let name = self.string()?;
            match (data, name.as_str(), tag) {
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
    let mut n = NbtReader { bytes: input, position: 0 };
    if n.u8()? != 10 { return Err(invalid("level.dat has no original root Compound")); }
    n.string()?;
    let mut meta = Metadata::default();
    n.compound(0, false, &mut meta)?;
    if n.position != input.len() {
        return Err(invalid("level.dat has trailing unrecognized NBT bytes"));
    }
    Ok(meta)
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
            let name = format!("mcedit2-rust-fixture-{}-{}",
                std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
            let dir = std::env::temp_dir().join(name);
            fs::create_dir_all(dir.join("region")).unwrap();
            Self(dir)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
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
    fn region_filename_accepts_negative_positions_and_refuses_suspicious_extensions() {
        assert!(is_region_filename(Path::new("r.-1.3.mcr")));
        assert!(is_region_filename(Path::new("r.12.-9.mca")));
        assert!(!is_region_filename(Path::new("r.0.0.mca.tmp")));
        assert!(!is_region_filename(Path::new("notes.mca")));
    }
}


/// Original MCEdit2 positional arguments can point to a containing folder,
/// level.dat or level.dat_old. Unsupported schematics and non-Java files are
/// rejected rather than pretending they were opened successfully.
pub fn inspect_input(path: impl AsRef<Path>) -> Result<WorldSummary> {
    let path = path.as_ref();
    match path.file_name() {
        Some(name) if (name == std::ffi::OsStr::new("level.dat")
                || name == std::ffi::OsStr::new("level.dat_old")) && path.is_file() => {
            inspect_world(path.parent().ok_or_else(|| invalid(
                "World metadata file has no parent directory"))?)
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
        Some(v) if v >= 2844 => "modern Java 1.18+: inspect-only, no writes",
        Some(v) if v >= 1451 => "named Java block states: inspect-only, no writes",
        Some(_) => "numeric Java Anvil: inspect-only, no writes",
        None if summary.storage_format == Some(19132) =>
            "original MCRegion format 19132: inspect-only, no writes",
        None if summary.storage_format == Some(19133) =>
            "original numeric Anvil format 19133: inspect-only, no writes",
        None => "legacy Java world without DataVersion: inspect-only, no writes",
    }
}
