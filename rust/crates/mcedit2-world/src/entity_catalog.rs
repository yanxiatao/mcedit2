//! Exact original Java entity NBT inspection and source-bound snapshot.
//! Modern worlds store Entities in separate entities/r.x.z.mca files;
//! old worlds embed Entities under Level. No entity editing is permitted.
use crate::block_lookup::read_world_chunk_bytes;
use crate::sections::parse_root;
use crate::{RegionIndex, WorldError, WorldSummary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
fn fail(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
fn sha(x: &[u8]) -> String {
    Sha256::digest(x)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[derive(Debug, Clone, PartialEq)]
pub struct OriginalEntity {
    pub id: Option<String>,
    pub pos: Option<[f64; 3]>,
    pub fields: usize,
    pub bytes: usize,
    pub sha256: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct EntityCatalog {
    pub source: String,
    pub chunk: (i32, i32),
    pub source_sha256: String,
    pub entities: Vec<OriginalEntity>,
}
fn source(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
) -> Result<Option<(String, Vec<u8>)>, WorldError> {
    let dimension = world
        .dimensions
        .iter()
        .find(|v| v.name == dim)
        .ok_or_else(|| fail("Unknown original dimension"))?;
    if let Some(parent) = dimension.region_dir.parent() {
        let name = parent.join("entities").join(format!(
            "r.{}.{}.mca",
            cx.div_euclid(32),
            cz.div_euclid(32)
        ));
        if name.exists() {
            if fs::symlink_metadata(&name)?.file_type().is_symlink() {
                return Err(fail("Entity Region symlink refused"));
            }
            let real = fs::canonicalize(&name)?;
            if !real.starts_with(fs::canonicalize(&world.root)?) {
                return Err(fail("Entity Region escapes original world"));
            }
            let index = RegionIndex::open(&real)?;
            return Ok(index
                .read_chunk(cx.rem_euclid(32) as u8, cz.rem_euclid(32) as u8)?
                .map(|raw| (format!("entity-region:{}", real.display()), raw)));
        }
    }
    read_world_chunk_bytes(world, dim, cx, cz)
        .map(|v| v.map(|raw| ("legacy-main-chunk".into(), raw)))
}
fn parse(original: &[u8], label: &str, cx: i32, cz: i32) -> Result<EntityCatalog, WorldError> {
    let root = parse_root(original).map_err(|e| fail(format!("Entity NBT invalid: {e}")))?;
    let modern = label.starts_with("entity-region:");
    let container = if modern {
        &root
    } else {
        root.field("Level").unwrap_or(&root)
    };
    if container.kind != 10 {
        return Err(fail("Entity storage not a Compound"));
    }
    if modern {
        let position = container
            .require("Position", 11)
            .map_err(|e| fail(e.to_string()))?;
        if position.raw.len() != 12
            || i32::from_be_bytes(position.raw[..4].try_into().unwrap()) != 2
            || i32::from_be_bytes(position.raw[4..8].try_into().unwrap()) != cx
            || i32::from_be_bytes(position.raw[8..12].try_into().unwrap()) != cz
        {
            return Err(fail("Entity file Position does not match its Region slot"));
        }
    }
    let mut entities = Vec::new();
    if let Some(list) = container.field("Entities") {
        if list.kind != 9 || list.items.len() > 16384 {
            return Err(fail(
                "Original entity List has incorrect NBT schema or count",
            ));
        }
        for entry in &list.items {
            if entry.kind != 10 {
                return Err(fail("Entity entry must be Compound"));
            }
            let id = entry
                .field("id")
                .map(|v| v.string().map_err(|e| fail(e.to_string())))
                .transpose()?;
            let pos = if let Some(v) = entry.field("Pos") {
                if v.kind != 9 || v.items.len() != 3 || v.items.iter().any(|x| x.kind != 6) {
                    return Err(fail("Entity Pos Double List malformed"));
                }
                let mut xyz = [0.0f64; 3];
                for (i, item) in v.items.iter().enumerate() {
                    xyz[i] = f64::from_be_bytes(
                        item.raw
                            .try_into()
                            .map_err(|_| fail("Invalid entity Double"))?,
                    );
                    if !xyz[i].is_finite() {
                        return Err(fail("Non-finite entity position"));
                    }
                }
                Some(xyz)
            } else {
                None
            };
            entities.push(OriginalEntity {
                id,
                pos,
                fields: entry.fields.len(),
                bytes: entry.raw.len(),
                sha256: sha(entry.raw),
            });
        }
    }
    Ok(EntityCatalog {
        source: label.into(),
        chunk: (cx, cz),
        source_sha256: sha(original),
        entities,
    })
}
pub fn inspect(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
) -> Result<EntityCatalog, WorldError> {
    let (label, raw) =
        source(world, dim, cx, cz)?.ok_or_else(|| fail("Original entity chunk not present"))?;
    parse(&raw, &label, cx, cz)
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntitySnapshot {
    version: u32,
    root: String,
    dimension: String,
    chunk: [i32; 2],
    source: String,
    digest: String,
    original_nbt: Vec<u8>,
}
pub fn export(
    world: &WorldSummary,
    dim: &str,
    cx: i32,
    cz: i32,
    path: &Path,
) -> Result<usize, WorldError> {
    if path.extension().and_then(|s| s.to_str()) != Some("mcedit2entities") {
        return Err(fail("Entity snapshot requires .mcedit2entities"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?) {
        return Err(fail("Entity snapshot must be outside Minecraft world"));
    }
    let (label, raw) =
        source(world, dim, cx, cz)?.ok_or_else(|| fail("Entity source disappeared"))?;
    if raw.len() > 8 * 1024 * 1024 {
        return Err(fail("Entity source >8MiB"));
    }
    parse(&raw, &label, cx, cz)?;
    let data = EntitySnapshot {
        version: 1,
        root: world.root.to_string_lossy().into(),
        dimension: dim.into(),
        chunk: [cx, cz],
        source: label,
        digest: sha(&raw),
        original_nbt: raw,
    };
    let bytes = serde_json::to_vec(&data)
        .map_err(|e| fail(format!("Entity snapshot JSON invalid: {e}")))?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(fail("Entity snapshot >32MiB"));
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let status = file.write_all(&bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = status {
        let _ = fs::remove_file(path);
        return Err(e.into());
    }
    Ok(bytes.len())
}
pub fn verify(world: &WorldSummary, dim: &str, path: &Path) -> Result<usize, WorldError> {
    if path.extension().and_then(|s| s.to_str()) != Some("mcedit2entities") {
        return Err(fail("Unknown entity snapshot extension"));
    }
    let stat = fs::symlink_metadata(path)?;
    if !stat.file_type().is_file() || stat.file_type().is_symlink() || stat.len() > 32 * 1024 * 1024
    {
        return Err(fail("Entity snapshot is not a safe regular file"));
    }
    let snap: EntitySnapshot = serde_json::from_slice(&fs::read(path)?)
        .map_err(|e| fail(format!("Entity snapshot JSON invalid: {e}")))?;
    if snap.version != 1
        || snap.root != world.root.to_string_lossy()
        || snap.dimension != dim
        || snap.original_nbt.len() > 8 * 1024 * 1024
        || sha(&snap.original_nbt) != snap.digest
    {
        return Err(fail("Entity snapshot checksum/world identity invalid"));
    }
    let (origin, actual) = source(world, dim, snap.chunk[0], snap.chunk[1])?
        .ok_or_else(|| fail("Original entity data disappeared"))?;
    if origin != snap.source || actual != snap.original_nbt {
        return Err(fail(
            "Entity Region differs from immutable original snapshot",
        ));
    }
    parse(&actual, &origin, snap.chunk[0], snap.chunk[1])?;
    Ok(actual.len())
}
