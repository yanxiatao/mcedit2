//! Unified read-only block lookup across original Java save generations.
//! Alpha individual .dat, MCRegion .mcr, numeric and named Anvil .mca,
//! modern negative-height worlds and compatible snapshot Section schemas.
use crate::alpha::read_alpha_chunk_bytes;
use crate::sections::{self, Layout, State};
use crate::{RegionIndex, WorldError, WorldSummary};
use std::collections::HashMap;
use std::{fs, path::Path};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockDetails {
    pub state: State,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub original_data_version: Option<i32>,
    pub layout: Layout,
    pub block_x: i32,
    pub block_y: i32,
    pub block_z: i32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    NoChunk,
    NoSection,
    Present(BlockDetails),
}
impl Lookup {
    pub fn description(&self) -> String {
        match self {
            Self::NoChunk => "No original chunk is stored here; nothing was generated.".into(),
            Self::NoSection => "Original chunk has no block Section for this Y.".into(),
            Self::Present(v) => {
                let properties = if v.state.properties.is_empty() {
                    String::new()
                } else {
                    format!(" {:?}", v.state.properties)
                };
                format!(
                    "Block ({},{},{}) => {}{} | source chunk ({},{}) | {:?} | chunk DataVersion: {}",
                    v.block_x,v.block_y,v.block_z,v.state.name,properties,
                    v.chunk_x,v.chunk_z,v.layout,
                    v.original_data_version.map_or_else(||"absent".into(),|v|v.to_string())
                )
            }
        }
    }
}
fn invalid(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}

/// Real modded 1.21.1 worlds can contain 0-byte r.X.Z.mca
/// placeholders for a not-yet-generated region. Refuse malformed nonempty
/// indexes, but let a genuine *empty regular file* behave like no chunk.
/// Never chase symlinks or convert an incomplete header to missing terrain.
fn open_original_region_or_empty(path: &Path) -> Result<Option<RegionIndex>, WorldError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid(
            "Original Region source must be a regular file, not a symlink",
        ));
    }
    if metadata.len() == 0 {
        return Ok(None);
    }
    RegionIndex::open(path).map(Some)
}
/// Fetch decompressed original NBT bytes without rewriting any unknown tags.
/// Consumers must validate the decoded chunk and its original coordinates
/// before trusting bytes in a patch or write plan.
pub fn read_world_chunk_bytes(
    world: &WorldSummary,
    dimension: &str,
    cx: i32,
    cz: i32,
) -> Result<Option<Vec<u8>>, WorldError> {
    let source = world
        .dimensions
        .iter()
        .find(|d| d.name == dimension)
        .ok_or_else(|| invalid(format!("Unknown original dimension {dimension}")))?;
    if source.extension.contains("c.x.z.dat") {
        return read_alpha_chunk_bytes(&source.region_dir, cx, cz);
    }
    let rx = cx.div_euclid(32);
    let rz = cz.div_euclid(32);
    let basename = format!("r.{rx}.{rz}");
    let anvil = source.region_dir.join(format!("{basename}.mca"));
    let mcr = source.region_dir.join(format!("{basename}.mcr"));
    if anvil.exists() && mcr.exists() {
        return Err(invalid(
            "Both original .mca and .mcr exist for one region position",
        ));
    }
    let path = if anvil.exists() {
        anvil
    } else if mcr.exists() {
        mcr
    } else {
        return Ok(None);
    };
    match open_original_region_or_empty(&path)? {
        Some(index) => index.read_chunk_at(cx, cz),
        None => Ok(None),
    }
}

/// Decode a source chunk and verify its storage coordinates and all Block
/// Entity positions before permitting any original-world edit operation.
fn checked_original_chunk(
    raw: Option<Vec<u8>>,
    cx: i32,
    cz: i32,
) -> Result<Option<sections::Chunk>, WorldError> {
    let chunk = raw
        .map(|bytes| {
            sections::decode_sections(&bytes)
                .map_err(|e| invalid(format!("Original chunk NBT refused: {e}")))
        })
        .transpose()?;
    if let Some(chunk) = chunk.as_ref() {
        if chunk
            .block_entities
            .iter()
            .any(|&(x, _, z)| x.div_euclid(16) != cx || z.div_euclid(16) != cz)
        {
            return Err(invalid(
                "Original Block Entity coordinates outside owning chunk",
            ));
        }
        if chunk.x.is_some_and(|v| v != cx) || chunk.z.is_some_and(|v| v != cz) {
            return Err(invalid(
                "Original chunk coordinates do not match its region slot",
            ));
        }
    }
    Ok(chunk)
}
pub fn read_world_chunk(
    world: &WorldSummary,
    dimension: &str,
    cx: i32,
    cz: i32,
) -> Result<Option<sections::Chunk>, WorldError> {
    checked_original_chunk(read_world_chunk_bytes(world, dimension, cx, cz)?, cx, cz)
}

/// Short-lived, read-only Region header cache for one distant preview worker
/// batch. Multiple sampled columns from the same 32x32 source region reuse
/// its validated 8 KiB index; actual chunks are still opened, decompressed,
/// coordinate-checked and BE-checked on every request. No world writes,
/// persistent cache, or absent-chunk synthesis is performed.
pub struct ReadOnlyRegionCache<'a> {
    world: &'a WorldSummary,
    dimension: &'a str,
    source: &'a crate::DimensionSummary,
    regions: HashMap<(i32, i32), Option<RegionIndex>>,
}
impl<'a> ReadOnlyRegionCache<'a> {
    pub fn new(world: &'a WorldSummary, dimension: &'a str) -> Result<Self, WorldError> {
        let source = world
            .dimensions
            .iter()
            .find(|d| d.name == dimension)
            .ok_or_else(|| invalid(format!("Unknown original dimension {dimension}")))?;
        Ok(Self {
            world,
            dimension,
            source,
            regions: HashMap::new(),
        })
    }
    pub fn read_chunk(&mut self, cx: i32, cz: i32) -> Result<Option<sections::Chunk>, WorldError> {
        // Alpha saves store one individual file per chunk, not Region headers.
        if self.source.extension.contains("c.x.z.dat") {
            return read_world_chunk(self.world, self.dimension, cx, cz);
        }
        let key = (cx.div_euclid(32), cz.div_euclid(32));
        if !self.regions.contains_key(&key) {
            let basename = format!("r.{}.{}", key.0, key.1);
            let mca = self.source.region_dir.join(format!("{basename}.mca"));
            let mcr = self.source.region_dir.join(format!("{basename}.mcr"));
            if mca.exists() && mcr.exists() {
                return Err(invalid(
                    "Both original .mca and .mcr exist for one region position",
                ));
            }
            let candidate = if mca.exists() {
                Some(mca)
            } else if mcr.exists() {
                Some(mcr)
            } else {
                None
            };
            let region = candidate
                .map(|path| open_original_region_or_empty(&path))
                .transpose()?
                .flatten();
            // Keep each background worker's header footprint below ~256 KiB.
            if self.regions.len() >= 32 {
                self.regions.clear();
            }
            self.regions.insert(key, region);
        }
        let raw = match self.regions.get(&key).and_then(Option::as_ref) {
            Some(region) => region.read_chunk_at(cx, cz)?,
            None => None,
        };
        checked_original_chunk(raw, cx, cz)
    }
}

/// Read ONE actual block, reusing native indexed chunk decoding.
pub fn read_world_block(
    world: &WorldSummary,
    dimension: &str,
    x: i32,
    y: i32,
    z: i32,
) -> Result<Lookup, WorldError> {
    let cx = x.div_euclid(16);
    let cz = z.div_euclid(16);
    let lx = x.rem_euclid(16) as u8;
    let lz = z.rem_euclid(16) as u8;
    let Some(chunk) = read_world_chunk(world, dimension, cx, cz)? else {
        return Ok(Lookup::NoChunk);
    };
    let Some(state) = chunk.block(lx, y, lz) else {
        return Ok(Lookup::NoSection);
    };
    Ok(Lookup::Present(BlockDetails {
        state: state.clone(),
        chunk_x: cx,
        chunk_z: cz,
        original_data_version: chunk.data_version,
        layout: chunk.layout,
        block_x: x,
        block_y: y,
        block_z: z,
    }))
}
pub fn parse_xyz(s: &str) -> Result<(i32, i32, i32), WorldError> {
    let parts: Vec<_> = s.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return Err(invalid(
            "Block query requires X,Y,Z as three signed integers",
        ));
    }
    let parse = |v: &str| {
        v.parse::<i32>()
            .map_err(|_| invalid(format!("Invalid original block coordinate: {v}")))
    };
    Ok((parse(parts[0])?, parse(parts[1])?, parse(parts[2])?))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn true_zero_byte_placeholder_is_absent_but_truncated_region_is_corrupt() {
        let dir = std::env::temp_dir().join(format!(
            "mcedit2-empty-original-region-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let region = dir.join("r.-1.-2.mca");
        std::fs::write(&region, []).unwrap();
        let world = WorldSummary {
            root: dir.clone(),
            level_name: None,
            data_version: Some(3955),
            storage_format: Some(19133),
            version_name: Some("1.21.1".into()),
            dimensions: vec![crate::DimensionSummary {
                name: "minecraft:overworld".into(),
                region_dir: dir.clone(),
                regions: 1,
                chunks: 0,
                extension: "mca".into(),
            }],
            warnings: vec![],
        };
        // Both direct and 3D/2D cached source paths agree that a real
        // zero-byte file is not an NBT Section or generated air.
        assert!(
            read_world_chunk_bytes(&world, "minecraft:overworld", -32, -64)
                .unwrap()
                .is_none()
        );
        assert!(
            read_world_block(&world, "minecraft:overworld", -512, 64, -1024).unwrap()
                == Lookup::NoChunk
        );
        let mut cache = ReadOnlyRegionCache::new(&world, "minecraft:overworld").unwrap();
        assert!(cache.read_chunk(-32, -64).unwrap().is_none());
        assert_eq!(std::fs::metadata(&region).unwrap().len(), 0);
        // A nonempty partial header is corruption, not a placeholder.
        std::fs::write(&region, [0u8; 4096]).unwrap();
        assert!(read_world_chunk_bytes(&world, "minecraft:overworld", -32, -64).is_err());
        let mut fresh = ReadOnlyRegionCache::new(&world, "minecraft:overworld").unwrap();
        assert!(fresh.read_chunk(-32, -64).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn exact_signed_xyz_parsing_and_error_paths() {
        assert_eq!(parse_xyz("-255, -64, 400").unwrap(), (-255, -64, 400));
        for bad in ["1,2", "1,2,3,4", "3.5,0,0", "nan,0,0", "2147483648,0,0"] {
            assert!(parse_xyz(bad).is_err());
        }
    }
    #[test]
    fn absent_region_and_invalid_dimension_do_not_generate_chunks() {
        let dir = std::env::temp_dir().join(format!("mcedit2-lookup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let world = WorldSummary {
            root: dir.clone(),
            level_name: Some("Native test".into()),
            data_version: Some(5125),
            storage_format: None,
            version_name: Some("26.4 Snapshot 5".into()),
            dimensions: vec![crate::DimensionSummary {
                name: "minecraft:overworld".into(),
                region_dir: dir.clone(),
                regions: 0,
                chunks: 0,
                extension: "mca".into(),
            }],
            warnings: vec![],
        };
        assert_eq!(
            read_world_block(&world, "minecraft:overworld", -1, -64, 0).unwrap(),
            Lookup::NoChunk
        );
        let mut cache = ReadOnlyRegionCache::new(&world, "minecraft:overworld").unwrap();
        for (cx, cz) in [(-2, -2), (-1, 0), (0, 0), (31, 31)] {
            assert!(cache.read_chunk(cx, cz).unwrap().is_none());
        }
        assert!(ReadOnlyRegionCache::new(&world, "minecraft:not-real").is_err());
        assert!(read_world_block(&world, "minecraft:not-real", 0, 0, 0).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
