//! 旧原生 Region 事务的内容凭据；只保留真实文件版本及槽位摘要，不持有 Region 镜像。
use crate::{
    history_source::{Archive, FileVersion},
    region_plan, WorldError, WorldSummary,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

fn err(message: impl Into<String>) -> WorldError {
    WorldError::Invalid(message.into())
}
fn digest(raw: &[u8]) -> [u8; 32] {
    Sha256::digest(raw).into()
}

/// 同一逻辑几何的保存标记、光照和未知元数据不改变这个摘要。
pub(crate) fn geometry(raw: &[u8]) -> Result<[u8; 32], WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| err(error.to_string()))?;
    let container = root.field("Level").unwrap_or(&root);
    let mut hash = Sha256::new();
    let mut feed = |name: &str, tag: &crate::sections::Tag<'_>| {
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        hash.update([tag.kind]);
        hash.update((tag.raw.len() as u64).to_be_bytes());
        hash.update(tag.raw);
    };
    for name in ["Blocks", "Data", "Add"] {
        if let Some(tag) = container.field(name) {
            feed(name, tag);
        }
    }
    for name in ["Sections", "sections"] {
        if let Some(list) = container.field(name) {
            let mut sections = list.items.iter().collect::<Vec<_>>();
            sections.sort_by_key(|section| section.field("Y").map(|tag| tag.raw));
            for section in sections {
                let fields = [
                    "Blocks",
                    "Data",
                    "Add",
                    "Palette",
                    "BlockStates",
                    "block_states",
                ];
                if !fields.iter().any(|name| section.field(name).is_some()) {
                    continue;
                }
                let y = section
                    .field("Y")
                    .ok_or_else(|| err("Region 内容来源的几何区段没有 Y"))?;
                feed("Y", y);
                for field in fields {
                    if let Some(tag) = section.field(field) {
                        feed(field, tag);
                    }
                }
            }
        }
    }
    Ok(hash.finalize().into())
}
fn entities(raw: &[u8]) -> Result<[u8; 32], WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| err(error.to_string()))?;
    let mut hash = Sha256::new();
    for (scope, container) in
        std::iter::once((0, &root)).chain(root.field("Level").map(|level| (1, level)))
    {
        for (name, tag) in &container.fields {
            if !matches!(name.as_str(), "Entities" | "entities") {
                continue;
            }
            hash.update([scope]);
            hash.update(name.as_bytes());
            hash.update([tag.kind]);
            hash.update((tag.raw.len() as u64).to_be_bytes());
            hash.update(tag.raw);
        }
    }
    Ok(hash.finalize().into())
}
fn payload(raw: &[u8]) -> Result<[u8; 32], WorldError> {
    let root = crate::sections::parse_root(raw).map_err(|error| err(error.to_string()))?;
    let mut hash = Sha256::new();
    for (scope, container) in
        std::iter::once((0, &root)).chain(root.field("Level").map(|level| (1, level)))
    {
        let mut fields = container
            .fields
            .iter()
            .filter(|(name, _)| {
                !matches!(
                    name.as_str(),
                    "Level"
                        | "DataVersion"
                        | "xPos"
                        | "yPos"
                        | "zPos"
                        | "Blocks"
                        | "Data"
                        | "Add"
                        | "Sections"
                        | "sections"
                        | "Biomes"
                        | "biomes"
                        | "BlockLight"
                        | "SkyLight"
                        | "HeightMap"
                        | "Heightmaps"
                        | "Entities"
                        | "entities"
                )
            })
            .collect::<Vec<_>>();
        fields.sort_by(|(left, _), (right, _)| left.cmp(right));
        hash.update([scope]);
        for (name, tag) in fields {
            hash.update((name.len() as u64).to_be_bytes());
            hash.update(name.as_bytes());
            hash.update([tag.kind]);
            hash.update((tag.raw.len() as u64).to_be_bytes());
            hash.update(tag.raw);
        }
    }
    Ok(hash.finalize().into())
}
#[derive(Debug, Clone)]
pub(crate) struct Slot {
    pub before_nbt: [u8; 32],
    pub after_nbt: [u8; 32],
    pub before_geometry: [u8; 32],
    pub after_geometry: [u8; 32],
    pub before_payload: [u8; 32],
    pub after_payload: [u8; 32],
}
#[derive(Debug)]
pub struct Change {
    identity: String,
    dimension: String,
    pub(crate) before: FileVersion,
    pub(crate) after: FileVersion,
    pub(crate) slots: BTreeMap<(i32, i32), Slot>,
    rollback: bool,
}
impl Change {
    pub fn dimension(&self) -> &str {
        &self.dimension
    }
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }
    pub(crate) fn rollback(&self) -> bool {
        self.rollback
    }
    pub(crate) fn retained_bytes(&self) -> usize {
        self.before.retained_bytes()
            + self.after.retained_bytes()
            + self.slots.len() * 256
            + self.dimension.len()
            + self.identity.len()
    }
    pub(crate) fn verified(
        world: &WorldSummary,
        dimension: &str,
        archive: &Archive,
        path: &Path,
        original: &str,
        replacement: &str,
        rollback: bool,
    ) -> Result<Self, WorldError> {
        let old = archive.retained_version(path, original)?;
        let new = archive.retained_version(path, replacement)?;
        let before = archive
            .read_version(&old, 256 * 1024 * 1024)?
            .ok_or_else(|| err("原 Region 版本缺失"))?;
        let after = archive
            .read_version(&new, 256 * 1024 * 1024)?
            .ok_or_else(|| err("替换 Region 版本缺失"))?;
        let old_slots = region_plan::parse_slots(&before)?;
        let new_slots = region_plan::parse_slots(&after)?;
        if after.len() <= before.len() || after.get(8192..before.len()) != Some(&before[8192..]) {
            return Err(err("旧 Region 事务没有保留原始扇区"));
        }
        let relative = path
            .strip_prefix(archive.world_root())
            .map_err(|_| err("Region 内容凭据越过世界根"))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| err("Region 文件名不是 UTF-8"))?;
        let parts = name.split('.').collect::<Vec<_>>();
        if parts.len() != 4 || parts[0] != "r" || !matches!(parts[3], "mca" | "mcr") {
            return Err(err("Region 内容凭据的文件名无效"));
        }
        let rx = parts[1].parse::<i32>().map_err(|_| err("Region X 无效"))?;
        let rz = parts[2].parse::<i32>().map_err(|_| err("Region Z 无效"))?;
        let expected =
            crate::region_transaction::expected_region_path(world, dimension, rx, rz, parts[3])?;
        if expected != path {
            return Err(err("Region 内容凭据所属维度不同"));
        }
        let mut slots = BTreeMap::new();
        for index in 0..1024 {
            let a = old_slots[index];
            let b = new_slots[index];
            if a.map(|slot| (slot.start, slot.sectors)) == b.map(|slot| (slot.start, slot.sectors))
            {
                continue;
            }
            let a = a.ok_or_else(|| err("旧 Region 事务不能新增槽"))?;
            let b = b.ok_or_else(|| err("旧 Region 事务不能删除槽"))?;
            let x = i32::try_from(i64::from(rx) * 32 + (index % 32) as i64)
                .map_err(|_| err("槽 X 越界"))?;
            let z = i32::try_from(i64::from(rz) * 32 + (index / 32) as i64)
                .map_err(|_| err("槽 Z 越界"))?;
            let (a_method, a_envelope, a_raw) = region_plan::original_nbt(&before, a)?;
            let (b_method, b_envelope, b_raw) = region_plan::original_nbt(&after, b)?;
            let a_chunk = crate::block_lookup::checked_original_chunk_bytes(Some(&a_raw), x, z)?
                .expect("已读取原槽");
            let b_chunk = crate::block_lookup::checked_original_chunk_bytes(Some(&b_raw), x, z)?
                .expect("已读取替换槽");
            if a_method != b_method
                || a_envelope != b_envelope
                || a_chunk.layout != b_chunk.layout
                || a_chunk.data_version != b_chunk.data_version
            {
                return Err(err("旧 Region 事务改变了区块版本、布局或压缩方式"));
            }
            if entities(&a_raw)? != entities(&b_raw)? {
                return Err(err("旧 Region 事务没有独立实体变更证明"));
            }
            if a_raw == b_raw {
                continue;
            }
            let (a_raw, b_raw) = if rollback {
                (&b_raw, &a_raw)
            } else {
                (&a_raw, &b_raw)
            };
            slots.insert(
                (x, z),
                Slot {
                    before_nbt: digest(a_raw),
                    after_nbt: digest(b_raw),
                    before_geometry: geometry(a_raw)?,
                    after_geometry: geometry(b_raw)?,
                    before_payload: payload(a_raw)?,
                    after_payload: payload(b_raw)?,
                },
            );
        }
        let identity = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(dimension, relative, original, replacement, rollback))
                    .map_err(|error| err(error.to_string()))?
            )
        );
        let (before, after) = if rollback { (new, old) } else { (old, new) };
        Ok(Self {
            identity,
            dimension: dimension.into(),
            before,
            after,
            slots,
            rollback,
        })
    }
}
