//! Exact, external ORIGINAL Block Entity compound byte snapshot.
//! All data is an opaque byte vector. Strict source NBT digest verification
//! is required on reload. NEVER imports this as a world change.
use crate::block_lookup::read_world_chunk_bytes;
use crate::sections::{decode_sections, original_block_entity_bytes};
use crate::{WorldError, WorldSummary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
const LIMIT: usize = 8 * 1024 * 1024;
fn refuse(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn digest(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalBeSnapshot {
    pub version: u32,
    pub original_world_root: String,
    pub dimension: String,
    pub xyz: [i32; 3],
    pub source_chunk_sha256: String,
    pub raw_compound_sha256: String,
    /// Exact original compound payload (no NBT type/name envelope).
    pub raw_compound: Vec<u8>,
}
impl OriginalBeSnapshot {
    fn check(&self, world: &WorldSummary, dimension: &str) -> Result<(), WorldError> {
        if self.version != 1
            || self.original_world_root != world.root.to_string_lossy()
            || self.dimension != dimension
            || self.raw_compound.len() > LIMIT
            || self.raw_compound.is_empty()
            || digest(&self.raw_compound) != self.raw_compound_sha256
        {
            return Err(refuse("Original BE snapshot identity or checksum invalid"));
        }
        Ok(())
    }
}
pub fn capture(
    world: &WorldSummary,
    dimension: &str,
    xyz: (i32, i32, i32),
) -> Result<OriginalBeSnapshot, WorldError> {
    let raw = read_world_chunk_bytes(world, dimension, xyz.0.div_euclid(16), xyz.2.div_euclid(16))?
        .ok_or_else(|| refuse("Original Block Entity chunk not found"))?;
    let source = decode_sections(&raw).map_err(|e| refuse(format!("Original NBT refused: {e}")))?;
    if source.x.is_some_and(|x| x != xyz.0.div_euclid(16))
        || source.z.is_some_and(|z| z != xyz.2.div_euclid(16))
    {
        return Err(refuse("Block Entity chunk owner differs from source"));
    }
    let compound = original_block_entity_bytes(&raw, xyz)
        .map_err(|e| refuse(format!("BE source NBT refused: {e}")))?
        .ok_or_else(|| refuse("No original Block Entity at selected coordinates"))?;
    if compound.len() > LIMIT {
        return Err(refuse("Opaque original Block Entity is larger than 8 MiB"));
    }
    Ok(OriginalBeSnapshot {
        version: 1,
        original_world_root: world.root.to_string_lossy().to_string(),
        dimension: dimension.into(),
        xyz: [xyz.0, xyz.1, xyz.2],
        source_chunk_sha256: digest(&raw),
        raw_compound_sha256: digest(&compound),
        raw_compound: compound,
    })
}
pub fn save(
    world: &WorldSummary,
    dimension: &str,
    snapshot: &OriginalBeSnapshot,
    path: &Path,
) -> Result<(), WorldError> {
    snapshot.check(world, dimension)?;
    if path.extension().and_then(|x| x.to_str()) != Some("mcedit2be") {
        return Err(refuse("Block Entity snapshot must be .mcedit2be"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?) {
        return Err(refuse("Snapshot must not be saved inside source world"));
    }
    let bytes = serde_json::to_vec(snapshot)
        .map_err(|e| refuse(format!("Snapshot encoding invalid: {e}")))?;
    // JSON byte-array encoding is larger than raw NBT; budget both forms.
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(refuse(
            "Opaque snapshot JSON exceeds maximum safe file size",
        ));
    }
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    let result = output.write_all(&bytes).and_then(|_| output.sync_all());
    drop(output);
    if let Err(e) = result {
        let _ = fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}
pub fn verify_file(
    world: &WorldSummary,
    dimension: &str,
    path: &Path,
) -> Result<OriginalBeSnapshot, WorldError> {
    if path.extension().and_then(|v| v.to_str()) != Some("mcedit2be") {
        return Err(refuse("Not a .mcedit2be original snapshot"));
    }
    let info = fs::symlink_metadata(path)?;
    if !info.file_type().is_file() || info.file_type().is_symlink() || info.len() > 32 * 1024 * 1024
    {
        return Err(refuse("Unsafe snapshot file type/size"));
    }
    let obj: OriginalBeSnapshot = serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| refuse(format!("Original snapshot JSON invalid: {e}")))?;
    obj.check(world, dimension)?;
    let point = (obj.xyz[0], obj.xyz[1], obj.xyz[2]);
    let current = capture(world, dimension, point)?;
    if obj.source_chunk_sha256 != current.source_chunk_sha256
        || obj.raw_compound != current.raw_compound
        || obj.raw_compound_sha256 != current.raw_compound_sha256
    {
        return Err(refuse(
            "Original source chunk/Block Entity changed since snapshot",
        ));
    }
    Ok(obj)
}
