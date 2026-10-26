//! Durable, source-verified replay journal for Block Entity scalar operations.
//! This is a detached edit checkpoint, NEVER a Minecraft region writer.
//! Refuse world/dimension/DataVersion changes and malformed/unbounded entries.
use crate::{WorldError, WorldSummary};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
const MAX_FILE: u64 = 256 * 1024;
const MAX_EDITS: usize = 24;
fn refuse(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeJournalEdit {
    pub xyz: [i32; 3],
    pub field: String,
    pub value: String,
    pub original_chunk_sha256: String,
    pub resulting_chunk_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeJournal {
    pub version: u32,
    pub world_root: String,
    pub dimension: String,
    pub data_version: Option<i32>,
    pub edits: Vec<BeJournalEdit>,
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.as_bytes().iter().all(|b| b.is_ascii_hexdigit())
}
impl BeJournal {
    pub fn validate(&self, world: &WorldSummary, dimension: &str) -> Result<(), WorldError> {
        if self.version != 1
            || self.world_root != world.root.to_string_lossy()
            || self.dimension != dimension
            || self.data_version != world.data_version
        {
            return Err(refuse(
                "BE journal does not match original world/dimension/DataVersion",
            ));
        }
        if self.edits.is_empty() || self.edits.len() > MAX_EDITS {
            return Err(refuse(
                "BE journal requires 1..24 original scalar operations",
            ));
        }
        for e in &self.edits {
            if !valid_hash(&e.original_chunk_sha256)
                || !valid_hash(&e.resulting_chunk_sha256)
                || e.field.is_empty()
                || e.field.len() > 128
                || e.value.len() > 4096
                || matches!(e.field.as_str(), "x" | "y" | "z" | "id" | "keepPacked")
            {
                return Err(refuse("BE journal contains invalid scalar edit metadata"));
            }
        }
        Ok(())
    }
}
pub fn save(
    world: &WorldSummary,
    dimension: &str,
    data: &BeJournal,
    path: &Path,
) -> Result<(), WorldError> {
    data.validate(world, dimension)?;
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_none_or(|n| !n.ends_with(".mcedit2bejournal.json"))
    {
        return Err(refuse("BE edit journal requires .mcedit2bejournal.json"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?) {
        return Err(refuse("BE edit journal must be outside the original world"));
    }
    let bytes = serde_json::to_vec_pretty(data)
        .map_err(|e| refuse(format!("BE journal encode failed: {e}")))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(refuse("BE journal exceeds 256 KiB"));
    }
    let mut out = OpenOptions::new().write(true).create_new(true).open(path)?;
    let writing = out.write_all(&bytes).and_then(|_| out.sync_all());
    drop(out);
    if let Err(e) = writing {
        let _ = fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}
pub fn load(world: &WorldSummary, dimension: &str, path: &Path) -> Result<BeJournal, WorldError> {
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_none_or(|n| !n.ends_with(".mcedit2bejournal.json"))
    {
        return Err(refuse(
            "BE journal extension must be .mcedit2bejournal.json",
        ));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() > MAX_FILE {
        return Err(refuse("BE journal must be regular file <= 256 KiB"));
    }
    let journal: BeJournal = serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| refuse(format!("Malformed BE journal JSON: {e}")))?;
    journal.validate(world, dimension)?;
    Ok(journal)
}
