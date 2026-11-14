//! 多 Region 的可恢复保存组。每个文件独立原子替换，整个组通过日志恢复一致性。
//! 不声称外部程序能在一系列文件替换之间观察到原子快照；世界必须离线。
use super::*;
use std::collections::BTreeSet;

const MAX_GROUP_BYTES: usize = 1024 * 1024 * 1024;
const ACTIVE_NAMES: [&str; 3] = [
    ".mcedit2-group.pending.json",
    ".mcedit2-group.finalizing.json",
    ".mcedit2-group.rolling-back.json",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    x: i32,
    z: i32,
    extension: String,
    original: String,
    replacement: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    version: u32,
    dimension: String,
    entries: Vec<Entry>,
}
#[derive(Debug, Clone)]
pub struct RegionGroupReceipt {
    pub marker_path: PathBuf,
    /// 已处理的文件；回滚尚未写入任何数据的条目不需要创建备份。
    pub regions: Vec<RegionWriteReceipt>,
    pub total_regions: usize,
}

pub(super) fn require_no_active_group(world: &WorldSummary) -> Result<(), WorldError> {
    crate::nbt_file::group::require_inactive(&world.root)?;
    for name in ACTIVE_NAMES {
        absent(&world.root.join(name))?;
    }
    Ok(())
}

fn check_group(group: &Group, dimension: &str) -> Result<(), WorldError> {
    if group.version != 1 || group.dimension != dimension || group.entries.is_empty() {
        return Err(refuse("保存组版本、维度或 Region 数量无效"));
    }
    let mut coordinates = BTreeSet::new();
    for entry in &group.entries {
        if !coordinates.insert((entry.x, entry.z, &entry.extension))
            || !matches!(entry.extension.as_str(), "mca" | "mcr")
            || [&entry.original, &entry.replacement].iter().any(|sha| {
                sha.len() != 64
                    || !sha
                        .bytes()
                        .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
            })
            || entry.original == entry.replacement
        {
            return Err(refuse("保存组包含重复坐标或无效摘要"));
        }
    }
    Ok(())
}

fn preflight(
    world: &WorldSummary,
    dimension: &str,
    candidates: &[NativeRegionCandidate],
) -> Result<Group, WorldError> {
    require_no_active_group(world)?;
    if candidates.is_empty()
        || candidates
            .iter()
            .try_fold(0usize, |sum, c| sum.checked_add(c.image.len()))
            .is_none_or(|n| n > MAX_GROUP_BYTES)
    {
        return Err(refuse("保存组不能为空，候选总计不能超过 1 GiB"));
    }
    let mut entries = Vec::new();
    // 在写入日志及任何 Region 之前，对所有源文件和恢复路径完成预检。
    for candidate in candidates {
        let ext = candidate
            .path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| refuse("候选文件缺少扩展名"))?;
        let region = expected_region_path(
            world,
            dimension,
            candidate.region_x,
            candidate.region_z,
            ext,
        )?;
        ensure_regular(&candidate.path)?;
        if fs::canonicalize(&candidate.path)? != region {
            return Err(refuse("保存组候选路径不属于源世界"));
        }
        if !candidate.section_proofs.is_empty() {
            return Err(refuse("新 section 须通过混合组复核维度合同并保存"));
        }
        let original = read_region_bytes(&region)?;
        verify_candidate(&original, candidate)?;
        let replacement = sha(&candidate.image);
        let files = paths(region, &candidate.original_region_sha256, &replacement)?;
        preflight_artifacts(&files, &planned_marker(candidate))?;
        entries.push(Entry {
            x: candidate.region_x,
            z: candidate.region_z,
            extension: ext.into(),
            original: candidate.original_region_sha256.clone(),
            replacement,
        });
    }
    let group = Group {
        version: 1,
        dimension: dimension.into(),
        entries,
    };
    check_group(&group, dimension)?;
    Ok(group)
}

pub fn validate_region_group(
    world: &WorldSummary,
    dimension: &str,
    candidates: &[NativeRegionCandidate],
) -> Result<usize, WorldError> {
    Ok(preflight(world, dimension, candidates)?.entries.len())
}

fn begin(
    world: &WorldSummary,
    dimension: &str,
    candidates: &[NativeRegionCandidate],
) -> Result<(Group, PathBuf), WorldError> {
    let group = preflight(world, dimension, candidates)?;
    let path = world.root.join(ACTIVE_NAMES[0]);
    let metadata = serde_json::to_vec(&group).map_err(|e| refuse(e.to_string()))?;
    if metadata.len() > MAX_GROUP_BYTES {
        return Err(refuse("保存组日志超过工作集预算"));
    }
    append_atomic_marker(&path, &metadata)?;
    parent_sync(&path)?;
    Ok((group, path))
}

/// 完整预检后依次替换文件。失败时报告原始错误，日志和备份保留，解决方向由调用方显式选择。
/// 成功后仍需要 finalize_region_group 或 rollback_region_group 显式解决日志。
pub fn apply_region_group(
    world: &WorldSummary,
    dimension: &str,
    candidates: &[NativeRegionCandidate],
    confirmation: &str,
) -> Result<RegionGroupReceipt, WorldError> {
    let _guard = crate::world_write_lock::acquire(&world.root, false)?;
    let has_be = candidates
        .iter()
        .any(|c| !c.be_replay.is_empty() || !c.be_history.is_empty());
    if (has_be && confirmation != "SAVE_BE_NBT") || (!has_be && confirmation != "SAVE") {
        return Err(refuse("保存组需要与内容匹配的 SAVE 或 SAVE_BE_NBT 确认"));
    }
    let (group, marker_path) = begin(world, dimension, candidates)?;
    let mut regions = Vec::new();
    for candidate in candidates {
        let receipt =
            apply_region_with_mode(world, dimension, candidate, has_be).map_err(|error| {
                log::error!(
                    "[region-group] 写入 {} 失败：{error}；保留活动记录 {}，请显式选择解决方向",
                    candidate.path.display(),
                    marker_path.display()
                );
                error
            })?;
        regions.push(receipt);
    }
    Ok(RegionGroupReceipt {
        marker_path,
        regions,
        total_regions: group.entries.len(),
    })
}

fn read_active(
    world: &WorldSummary,
    dimension: &str,
) -> Result<(Group, PathBuf, usize), WorldError> {
    let mut found = None;
    for (phase, name) in ACTIVE_NAMES.iter().enumerate() {
        let path = world.root.join(name);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if found.is_some()
                    || !meta.is_file()
                    || meta.file_type().is_symlink()
                    || meta.len() > MAX_GROUP_BYTES as u64
                {
                    return Err(refuse("保存组日志重复、非普通文件或超过工作集预算"));
                }
                let group: Group = serde_json::from_slice(&fs::read(&path)?)
                    .map_err(|e| refuse(format!("保存组日志无效：{e}")))?;
                check_group(&group, dimension)?;
                found = Some((group, path, phase));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    found.ok_or_else(|| refuse("当前世界没有活动的多 Region 保存组"))
}

fn matching_marker(path: &Path, entry: &Entry) -> Result<Marker, WorldError> {
    ensure_regular(path)?;
    if fs::metadata(path)?.len() > 4096 {
        return Err(refuse("Region 恢复记录超过 4 KiB"));
    }
    let marker: Marker =
        serde_json::from_slice(&fs::read(path)?).map_err(|e| refuse(e.to_string()))?;
    verify_marker(&marker)?;
    if marker.original_sha256 != entry.original || marker.replacement_sha256 != entry.replacement {
        return Err(refuse("单文件恢复记录与保存组不一致"));
    }
    Ok(marker)
}

fn exists(path: &Path) -> Result<bool, WorldError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            ensure_regular(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn resolve(
    world: &WorldSummary,
    dimension: &str,
    rollback: bool,
) -> Result<RegionGroupReceipt, WorldError> {
    let (group, mut active, phase) = read_active(world, dimension)?;
    let wanted = if rollback { 2 } else { 1 };
    if phase != 0 && phase != wanted {
        return Err(refuse("保存组已开始另一种恢复操作，必须先继续完成该操作"));
    }
    let bytes = serde_json::to_vec(&group).map_err(|error| refuse(error.to_string()))?;
    let fingerprint = sha(&bytes);
    let terminal = world.root.join(format!(
        ".mcedit2-group.{fingerprint}.{}.json",
        if rollback { "rolled-back" } else { "completed" }
    ));
    crate::transaction_artifacts::verify(&terminal, &bytes)?;
    let mut related = Vec::new();
    // 所有摘要、日志与备份先验证，避免发现外部改动前已经恢复了一半文件。
    for entry in &group.entries {
        let region = expected_region_path(world, dimension, entry.x, entry.z, &entry.extension)?;
        let files = paths(region, &entry.original, &entry.replacement)?;
        let current = sha(&read_region_bytes(&files.region)?);
        if !(current == entry.replacement || rollback && current == entry.original) {
            return Err(refuse(format!(
                "Region {} 已被外部修改或不符合恢复方向；保存组保持不变",
                files.region.display()
            )));
        }
        let terminal = if rollback {
            &files.rolled_back
        } else {
            &files.completed
        };
        if exists(&files.pending)? {
            let (marker, _) = read_pending(files.region.clone())?;
            if marker.original_sha256 != entry.original
                || marker.replacement_sha256 != entry.replacement
            {
                return Err(refuse("待处理 Region 日志与保存组不一致"));
            }
            verify_archives(&files, &marker)?;
        } else if exists(terminal)? {
            let marker = matching_marker(terminal, entry)?;
            verify_archives(&files, &marker)?;
            let expected = if rollback {
                &entry.original
            } else {
                &entry.replacement
            };
            if current != *expected {
                return Err(refuse("已完成 Region 记录与当前数据不符"));
            }
            ensure_regular(&files.backup)?;
            let bytes = read_region_bytes(&files.backup)?;
            if bytes.len() as u64 != marker.original_size || sha(&bytes) != entry.original {
                return Err(refuse("已处理 Region 的原始备份损坏"));
            }
        } else if !(rollback && current == entry.original) {
            return Err(refuse("保存组缺少对应 Region 的恢复记录"));
        }
        related.push(files);
    }
    if phase == 0 {
        let selected = world.root.join(ACTIVE_NAMES[wanted]);
        absent(&selected)?;
        fs::rename(&active, &selected)?;
        parent_sync(&selected)?;
        active = selected;
    }
    let mut regions = Vec::new();
    for (entry, files) in group.entries.iter().zip(related) {
        if exists(&files.pending)? {
            regions.push(if rollback {
                rollback_region(world, dimension, entry.x, entry.z, &entry.extension)?
            } else {
                finalize_region(world, dimension, entry.x, entry.z, &entry.extension)?
            });
        } else {
            let terminal = if rollback {
                files.rolled_back.clone()
            } else {
                files.completed.clone()
            };
            if exists(&terminal)? {
                if !rollback {
                    let replacement = read_region_bytes(&files.region)?;
                    if sha(&replacement) != entry.replacement {
                        return Err(refuse("已完成 Region 的替换版本在恢复期间改变"));
                    }
                    crate::transaction_artifacts::store(&files.replacement_backup, &replacement)?;
                }
                let replacement_backup = retained_replacement(&files, &entry.replacement)?;
                regions.push(RegionWriteReceipt {
                    region_path: files.region,
                    backup_path: files.backup,
                    marker_path: terminal,
                    original_sha256: entry.original.clone(),
                    replacement_sha256: entry.replacement.clone(),
                    replacement_backup,
                });
            }
        }
    }
    crate::transaction_artifacts::archive(&active, &terminal)?;
    parent_sync(&terminal)?;
    Ok(RegionGroupReceipt {
        marker_path: terminal,
        regions,
        total_regions: group.entries.len(),
    })
}

pub fn finalize_region_group(
    world: &WorldSummary,
    dimension: &str,
) -> Result<RegionGroupReceipt, WorldError> {
    finalize_region_group_with_history(world, dimension).map(|(receipt, _)| receipt)
}
pub fn finalize_region_group_with_history(
    world: &WorldSummary,
    dimension: &str,
) -> Result<(RegionGroupReceipt, crate::history_source::Archive), WorldError> {
    let _guard = crate::world_write_lock::acquire(&world.root, false)?;
    let receipt = resolve(world, dimension, false)?;
    let archive = resolved_history(world, dimension, &receipt, false)?;
    Ok((receipt, archive))
}
pub fn rollback_region_group(
    world: &WorldSummary,
    dimension: &str,
) -> Result<RegionGroupReceipt, WorldError> {
    rollback_region_group_with_history(world, dimension).map(|(receipt, _)| receipt)
}
pub fn rollback_region_group_with_history(
    world: &WorldSummary,
    dimension: &str,
) -> Result<(RegionGroupReceipt, crate::history_source::Archive), WorldError> {
    let _guard = crate::world_write_lock::acquire(&world.root, false)?;
    let receipt = resolve(world, dimension, true)?;
    let archive = resolved_history(world, dimension, &receipt, true)?;
    Ok((receipt, archive))
}
fn resolved_history(
    world: &WorldSummary,
    dimension: &str,
    receipt: &RegionGroupReceipt,
    rollback: bool,
) -> Result<crate::history_source::Archive, WorldError> {
    history::resolved(
        world,
        dimension,
        &receipt.regions,
        rollback,
        ACTIVE_NAMES
            .iter()
            .map(|name| world.root.join(name))
            .chain(std::iter::once(receipt.marker_path.clone())),
    )
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        editing::{parse_state, Bounds, Session},
        inspect_world,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture {
        root: PathBuf,
        world: WorldSummary,
        plans: Vec<NativeRegionCandidate>,
        originals: Vec<(PathBuf, Vec<u8>)>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.root) {
                eprintln!("清理保存组测试目录失败：{error}");
            }
        }
    }
    impl Fixture {
        fn new() -> Self {
            use crate::chunk_creation::{empty_chunk, BiomeSource, ChunkCreationProfile};
            use crate::nbt_document::{Field, Value};
            use std::io::Write;
            let root = std::env::temp_dir().join(format!(
                "mcedit2-group-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("region")).unwrap();
            let mut level = vec![10, 0, 0];
            level.extend(
                Value::Compound(vec![Field {
                    name: "Data".into(),
                    value: Value::Compound(vec![
                        Field {
                            name: "DataVersion".into(),
                            value: Value::Int(3955),
                        },
                        Field {
                            name: "version".into(),
                            value: Value::Int(19133),
                        },
                        Field {
                            name: "LevelName".into(),
                            value: Value::String("保存组原始来源".into()),
                        },
                    ]),
                }])
                .payload()
                .unwrap(),
            );
            let mut gzip =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gzip.write_all(&level).unwrap();
            fs::write(root.join("level.dat"), gzip.finish().unwrap()).unwrap();
            let profile = ChunkCreationProfile {
                data_version: 3955,
                min_y: 0,
                height: 16,
                has_skylight: true,
                dimension_source: "user:保存组验证维度".into(),
                initial_biome: "minecraft:plains".into(),
                biome_source: BiomeSource::UserSpecified {
                    origin: "保存组验证的显式 biome".into(),
                    entries: ["minecraft:plains".into()].into_iter().collect(),
                },
            };
            let mut originals = Vec::new();
            for rx in [-1, 0] {
                let raw = empty_chunk(&profile, (rx * 32, 0)).unwrap();
                let raw = crate::nbt_document::patch(
                    &raw,
                    &crate::nbt_document::Operation::InsertField {
                        path: String::new(),
                        name: "OpaqueGroupData".into(),
                        value: Value::ByteArray(vec![-1, 0, 47, -128]),
                    },
                )
                .unwrap();
                let compressed = crate::region_plan::compress(2, &raw).unwrap();
                let sectors = (compressed.len() + 5).div_ceil(4096);
                let mut image = vec![0; (sectors + 3) * 4096];
                image[0..4].copy_from_slice(&((2u32 << 8) | sectors as u32).to_be_bytes());
                image[8192..8196].copy_from_slice(&((compressed.len() + 1) as u32).to_be_bytes());
                image[8196] = 2;
                image[8197..8197 + compressed.len()].copy_from_slice(&compressed);
                image[(sectors + 2) * 4096..].fill(0xa7);
                let path = root.join(format!("region/r.{rx}.0.mca"));
                fs::write(&path, &image).unwrap();
                originals.push((path, image));
            }
            let world = inspect_world(&root).unwrap();
            let mut session = Session::new(world.clone(), "minecraft:overworld").unwrap();
            for rx in [-1, 0] {
                let point = (rx * 32 * 16, 0, 0);
                let preview = session
                    .prepare(
                        Bounds::new(point, point).unwrap(),
                        &parse_state("minecraft:stone").unwrap(),
                        None,
                    )
                    .unwrap();
                session.commit(preview).unwrap();
            }
            let plans = session.prepare_native_region_candidates(123).unwrap();
            assert_eq!(plans.len(), 2);
            Self {
                root,
                world,
                plans,
                originals,
            }
        }
        fn assert_original(&self) {
            for (path, bytes) in &self.originals {
                assert_eq!(&fs::read(path).unwrap(), bytes);
            }
        }
    }
    const DIM: &str = "minecraft:overworld";

    fn group_save_and_finalize_keep_every_immutable_original_backup() {
        let fixture = Fixture::new();
        let other = Fixture::new();
        let playerdata = other.root.join("playerdata");
        fs::create_dir(&playerdata).unwrap();
        assert_eq!(
            crate::world_write_lock::scope(&playerdata).unwrap(),
            fs::canonicalize(&other.root).unwrap()
        );
        {
            let _first = crate::world_write_lock::acquire(&fixture.root, false).unwrap();
            assert!(crate::world_write_lock::acquire(&fixture.root.join("."), false).is_err());
            assert!(
                crate::world_write_lock::acquire_pair(&other.root, &fixture.root, false).is_err()
            );
            let _second = crate::world_write_lock::acquire(&other.root, false).unwrap();
            assert!(crate::world_write_lock::acquire(&playerdata, false).is_err());
        }
        {
            let _pair =
                crate::world_write_lock::acquire_pair(&fixture.root, &other.root, false).unwrap();
            assert!(crate::world_write_lock::acquire(&fixture.root, false).is_err());
            assert!(crate::world_write_lock::acquire(&other.root, false).is_err());
        }
        let source_bytes = fs::read(fixture.root.join("level.dat")).unwrap();
        let mut document = crate::nbt_file::FileDocument::open(
            &fixture.root,
            &fixture.root.join("level.dat"),
            false,
        )
        .unwrap();
        document
            .document
            .apply(&crate::nbt_document::Operation::Replace {
                path: "/Data/LevelName".into(),
                value: crate::nbt_document::Value::String("另存为会话".into()),
            })
            .unwrap();
        let target = playerdata.join("copied.nbt");
        let copy = document.candidate_as(&target).unwrap();
        {
            let _guard = crate::world_write_lock::acquire(&other.root, false).unwrap();
            assert!(crate::nbt_file::apply_as(&copy).is_err());
        }
        apply_region_group(&other.world, DIM, &other.plans, "SAVE").unwrap();
        assert!(crate::nbt_file::apply_as(&copy).is_err());
        rollback_region_group(&other.world, DIM).unwrap();
        crate::nbt_file::apply_as(&copy).unwrap();
        crate::nbt_file::finalize_as(&fixture.root, &target, false).unwrap();
        let copied = fs::read(&target).unwrap();
        document
            .document
            .apply(&crate::nbt_document::Operation::Replace {
                path: "/Data/LevelName".into(),
                value: crate::nbt_document::Value::String("第二次另存为".into()),
            })
            .unwrap();
        let replacement = document.candidate_as(&target).unwrap();
        crate::nbt_file::apply_as(&replacement).unwrap();
        crate::nbt_file::resolve_as(&fixture.root, &target, false, false).unwrap();
        assert_eq!(fs::read(&target).unwrap(), copied);
        assert_eq!(
            fs::read(fixture.root.join("level.dat")).unwrap(),
            source_bytes
        );
        // 同一内容重复回滚后仍可提交，原始备份和历史终态保持不可变。
        for _ in 0..2 {
            apply_region_group(&fixture.world, DIM, &fixture.plans, "SAVE").unwrap();
            rollback_region_group(&fixture.world, DIM).unwrap();
            fixture.assert_original();
        }
        let receipt = apply_region_group(&fixture.world, DIM, &fixture.plans, "SAVE").unwrap();
        assert_eq!(receipt.total_regions, 2);
        assert_eq!(receipt.regions.len(), 2);
        for plan in &fixture.plans {
            assert_eq!(fs::read(&plan.path).unwrap(), plan.image);
        }
        assert!(require_no_active_group(&fixture.world).is_err());
        let completed = finalize_region_group(&fixture.world, DIM).unwrap();
        assert!(completed.marker_path.exists());
        require_no_active_group(&fixture.world).unwrap();
        for receipt in completed.regions {
            assert!(receipt.backup_path.exists());
            assert_eq!(
                sha(&fs::read(receipt.backup_path).unwrap()),
                receipt.original_sha256
            );
        }
        let single = Fixture::new();
        let plan = &single.plans[0];
        for _ in 0..2 {
            apply_single_region(&single.world, DIM, plan).unwrap();
            rollback_pending_region(&single.world, DIM, plan.region_x, plan.region_z, "mca")
                .unwrap();
            single.assert_original();
        }
        let files = paths(
            plan.path.clone(),
            &plan.original_region_sha256,
            &sha(&plan.image),
        )
        .unwrap();
        let terminal = fs::read(&files.rolled_back).unwrap();
        fs::write(&files.rolled_back, b"damaged archive").unwrap();
        assert!(apply_single_region(&single.world, DIM, plan).is_err());
        single.assert_original();
        fs::write(&files.rolled_back, terminal).unwrap();
        let backup = fs::read(&files.backup).unwrap();
        fs::write(&files.backup, b"damaged backup").unwrap();
        assert!(apply_single_region(&single.world, DIM, plan).is_err());
        single.assert_original();
        fs::write(&files.backup, backup).unwrap();
        apply_single_region(&single.world, DIM, plan).unwrap();
        finalize_single_region(&single.world, DIM, plan.region_x, plan.region_z, "mca").unwrap();
    }

    fn interrupted_write_rolls_back_written_region_and_keeps_unwritten_region() {
        let fixture = Fixture::new();
        begin(&fixture.world, DIM, &fixture.plans).unwrap();
        apply_region_with_mode(&fixture.world, DIM, &fixture.plans[0], false).unwrap();
        assert!(apply_single_region(&fixture.world, DIM, &fixture.plans[1]).is_err());
        let receipt = rollback_region_group(&fixture.world, DIM).unwrap();
        assert_eq!(receipt.total_regions, 2);
        assert_eq!(receipt.regions.len(), 1);
        fixture.assert_original();
        require_no_active_group(&fixture.world).unwrap();
    }

    fn external_change_refuses_entire_recovery_before_mutating_other_regions() {
        let fixture = Fixture::new();
        begin(&fixture.world, DIM, &fixture.plans).unwrap();
        apply_region_with_mode(&fixture.world, DIM, &fixture.plans[0], false).unwrap();
        let mut foreign = fs::read(&fixture.plans[1].path).unwrap();
        foreign.push(1);
        fs::write(&fixture.plans[1].path, &foreign).unwrap();
        assert!(rollback_region_group(&fixture.world, DIM).is_err());
        assert_eq!(
            fs::read(&fixture.plans[0].path).unwrap(),
            fixture.plans[0].image
        );
        assert_eq!(fs::read(&fixture.plans[1].path).unwrap(), foreign);
    }

    fn interrupted_finalization_resumes_and_cannot_switch_recovery_direction() {
        let fixture = Fixture::new();
        apply_region_group(&fixture.world, DIM, &fixture.plans, "SAVE").unwrap();
        fs::rename(
            fixture.world.root.join(ACTIVE_NAMES[0]),
            fixture.world.root.join(ACTIVE_NAMES[1]),
        )
        .unwrap();
        let first = &fixture.plans[0];
        finalize_region(&fixture.world, DIM, first.region_x, first.region_z, "mca").unwrap();
        assert!(rollback_region_group(&fixture.world, DIM).is_err());
        let receipt = finalize_region_group(&fixture.world, DIM).unwrap();
        assert_eq!(receipt.regions.len(), 2);
    }

    fn interrupted_rollback_resumes_from_verified_per_region_records() {
        let fixture = Fixture::new();
        apply_region_group(&fixture.world, DIM, &fixture.plans, "SAVE").unwrap();
        fs::rename(
            fixture.world.root.join(ACTIVE_NAMES[0]),
            fixture.world.root.join(ACTIVE_NAMES[2]),
        )
        .unwrap();
        let first = &fixture.plans[0];
        rollback_region(&fixture.world, DIM, first.region_x, first.region_z, "mca").unwrap();
        let receipt = rollback_region_group(&fixture.world, DIM).unwrap();
        assert_eq!(receipt.regions.len(), 2);
        fixture.assert_original();
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Recovery {
            group_save_and_finalize_keep_every_immutable_original_backup();
            interrupted_write_rolls_back_written_region_and_keeps_unwritten_region();
            external_change_refuses_entire_recovery_before_mutating_other_regions();
            interrupted_finalization_resumes_and_cannot_switch_recovery_direction();
            interrupted_rollback_resumes_from_verified_per_region_records();
        }
    }
}
