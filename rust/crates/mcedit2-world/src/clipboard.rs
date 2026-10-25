//! A bounded block-only source clipboard, not an NBT schematic converter.
//! No entity, Block Entity or opaque modded NBT is ever serialized.
use crate::editing::{parse_state, render_state};
use crate::sections::State;
use crate::{WorldError, WorldSummary};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

const FORMAT: &str = "MCEDIT2_RUST_BLOCK_ONLY_CLIPBOARD";
const VERSION: u32 = 1;
const MAX_CELLS: usize = 4096;
const MAX_BYTES: usize = 8 * 1024 * 1024;
fn refuse(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockClipboard {
    pub format: String,
    pub version: u32,
    pub source_root: String,
    pub dimension: String,
    pub data_version: Option<i32>,
    pub numeric: bool,
    pub source_min: [i32; 3],
    pub size: [u16; 3],
    // Y-major, then Z, then X; no Block Entity NBT or entities.
    pub blocks: Vec<State>,
}
impl BlockClipboard {
    pub fn validate(&self, world: &WorldSummary, dimension: &str) -> Result<(), WorldError> {
        if self.format != FORMAT || self.version != VERSION {
            return Err(refuse("Unknown native block-only clipboard format/version"));
        }
        if self.source_root != world.root.to_string_lossy()
            || self.dimension != dimension
            || self.data_version != world.data_version
        {
            return Err(refuse(
                "Clipboard must match the same original world, dimension and DataVersion",
            ));
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
        if volume > MAX_CELLS || volume != self.blocks.len() {
            return Err(refuse(
                "Clipboard size exceeds 4096 blocks or state count differs",
            ));
        }
        for state in &self.blocks {
            if (self.numeric != state.name.starts_with("numeric:"))
                || parse_state(&render_state(state))? != *state
            {
                return Err(refuse(
                    "Clipboard contains invalid, noncanonical or mismatched block states",
                ));
            }
        }
        Ok(())
    }
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
fn reorient(state: &State, operation: HorizontalTransform) -> Result<State, WorldError> {
    if operation == HorizontalTransform::Identity {
        return Ok(state.clone());
    }
    if state.name.starts_with("numeric:") {
        return Err(refuse(
            "Cannot rotate/mirror numeric metadata without version-specific ID mappings",
        ));
    }
    let mut next = state.clone();
    for (key, value) in &mut next.properties {
        match key.as_str() {
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
            "waterlogged" | "lit" | "powered" => {}
            _ => {
                return Err(refuse(format!(
                    "Cannot prove safe rotation for unknown state property '{key}'"
                )))
            }
        }
    }
    // Validate the transformed literal rather than inventing a new schema.
    let parsed = parse_state(&render_state(&next))?;
    if parsed != next {
        return Err(refuse("Rotated state did not roundtrip canonically"));
    }
    Ok(next)
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
        let quarter = matches!(
            operation,
            HorizontalTransform::Rotate90 | HorizontalTransform::Rotate270
        );
        let out_w = if quarter { z } else { w };
        let out_z = if quarter { w } else { z };
        // Keep the original source provenance. Only dimensions and block
        // order/states rotate; no Block Entity NBT exists in the clipboard.
        let mut blocks = vec![None; self.blocks.len()];
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
                    blocks[destination] = Some(reorient(&self.blocks[original], operation)?);
                }
            }
        }
        let mut copy = self.clone();
        copy.size = [out_w as u16, h as u16, out_z as u16];
        copy.blocks = blocks
            .into_iter()
            .map(|v| v.ok_or_else(|| refuse("Transformation left an unset block cell")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(copy)
    }
}

pub(crate) fn new(
    world: &WorldSummary,
    dimension: &str,
    numeric: bool,
    source_min: [i32; 3],
    size: [u16; 3],
    blocks: Vec<State>,
) -> Result<BlockClipboard, WorldError> {
    let clipboard = BlockClipboard {
        format: FORMAT.into(),
        version: VERSION,
        source_root: world.root.to_string_lossy().to_string(),
        dimension: dimension.into(),
        data_version: world.data_version,
        numeric,
        source_min,
        size,
        blocks,
    };
    clipboard.validate(world, dimension)?;
    Ok(clipboard)
}
pub fn save(world: &WorldSummary, path: &Path, clip: &BlockClipboard) -> Result<(), WorldError> {
    clip.validate(world, &clip.dimension)?;
    if path.extension().and_then(|v| v.to_str()) != Some("mcedit2clip") {
        return Err(refuse("Clipboard must have .mcedit2clip extension"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if fs::canonicalize(parent)?.starts_with(fs::canonicalize(&world.root)?) {
        return Err(refuse(
            "Clipboard cannot be saved inside the original world folder",
        ));
    }
    let bytes = serde_json::to_vec_pretty(clip)
        .map_err(|e| refuse(format!("Clipboard encode failed: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(refuse("Clipboard JSON exceeds 8 MiB limit"));
    }
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    let writing = file.write_all(&bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = writing {
        let _ = fs::remove_file(path);
        return Err(e.into());
    }
    Ok(())
}
pub fn load(
    world: &WorldSummary,
    dimension: &str,
    path: &Path,
) -> Result<BlockClipboard, WorldError> {
    if path.extension().and_then(|v| v.to_str()) != Some("mcedit2clip") {
        return Err(refuse("Only .mcedit2clip is accepted"));
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_BYTES as u64 {
        return Err(refuse("Clipboard must be a regular file under 8 MiB"));
    }
    let bytes = fs::read(path)?;
    if bytes.len() > MAX_BYTES {
        return Err(refuse("Clipboard JSON exceeds 8 MiB"));
    }
    let clipboard: BlockClipboard = serde_json::from_slice(&bytes)
        .map_err(|e| refuse(format!("Invalid clipboard JSON: {e}")))?;
    clipboard.validate(world, dimension)?;
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
            data_version: Some(4000),
            numeric: false,
            source_min: [1, 0, 2],
            size: [2, 1, 3],
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
