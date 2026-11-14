//! 已解决旧 Region 事务的实际文件来源；仅由持有世界写锁的恢复入口构造。
use super::*;
use crate::history_source::{Archive, Backup};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn resolved(
    world: &WorldSummary,
    dimension: &str,
    regions: &[RegionWriteReceipt],
    rollback: bool,
    extra: impl IntoIterator<Item = PathBuf>,
) -> Result<Archive, WorldError> {
    let root = fs::canonicalize(&world.root)?;
    let mut files = BTreeMap::new();
    let mut heads = BTreeMap::new();
    let mut versions = Vec::new();
    let mut transitions = Vec::new();
    let mut artifacts = extra.into_iter().collect::<BTreeSet<_>>();
    for receipt in regions {
        let related = paths(
            receipt.region_path.clone(),
            &receipt.original_sha256,
            &receipt.replacement_sha256,
        )?;
        files.insert(
            receipt.region_path.clone(),
            Some(Backup {
                path: receipt.backup_path.clone(),
                sha: receipt.original_sha256.clone(),
            }),
        );
        heads.insert(
            receipt.region_path.clone(),
            Some(if rollback {
                receipt.original_sha256.clone()
            } else {
                receipt.replacement_sha256.clone()
            }),
        );
        artifacts.extend([
            related.backup,
            related.replacement_backup,
            related.staged,
            related.abandoned,
            related.restore,
            related.pending,
            related.completed,
            related.rolled_back,
        ]);
        if let Some(path) = &receipt.replacement_backup {
            versions.push((
                receipt.region_path.clone(),
                Backup {
                    path: path.clone(),
                    sha: receipt.replacement_sha256.clone(),
                },
            ));
            if rollback {
                transitions.push((
                    receipt.region_path.clone(),
                    Some(receipt.original_sha256.clone()),
                    Some(receipt.replacement_sha256.clone()),
                ));
                transitions.push((
                    receipt.region_path.clone(),
                    Some(receipt.replacement_sha256.clone()),
                    Some(receipt.original_sha256.clone()),
                ));
            }
        }
    }
    let archive = Archive::from_verified(root, files, heads, artifacts, BTreeSet::new())
        .retain_verified_versions(versions)
        .retain_verified_transitions(transitions);
    let changes = regions
        .iter()
        .filter(|receipt| receipt.replacement_backup.is_some())
        .map(|receipt| {
            crate::region_history::Change::verified(
                world,
                dimension,
                &archive,
                &receipt.region_path,
                &receipt.original_sha256,
                &receipt.replacement_sha256,
                rollback,
            )
        })
        .collect::<Result<Vec<_>, WorldError>>()?;
    Ok(archive.register_region_changes(changes))
}
