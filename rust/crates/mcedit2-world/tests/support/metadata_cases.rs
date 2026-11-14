//! 世界 / 玩家 NBT 与多次地图导入的完整保存、来源拒绝和回滚。
use super::metadata_fixture::{field, gzip, nbt, Fixture};
use mcedit2_world::{
    map_catalog, map_data, map_edit,
    mixed_transaction::{self, MixedCandidate},
    nbt_document::{Operation, Value},
    nbt_file::{self, FileDocument},
    player_catalog,
};
use std::fs;
const BUDGET: usize = 256 * 1024 * 1024;
fn mosaic(version: i32) -> map_edit::Mosaic {
    map_edit::quantize_mosaic(Some(version), 1, 1, &[255, 0, 0, 255], BUDGET, || false).unwrap()
}
fn maps(f: &Fixture) -> (map_edit::Candidate, map_edit::Candidate, map_edit::Mosaic) {
    let a = f.image("first.png");
    let b = f.image("second.png");
    let mosaic = mosaic(f.world.data_version.unwrap());
    let catalog = map_catalog::scan(&f.root, || false).unwrap();
    let first = map_edit::prepare_import(
        &f.world,
        "minecraft:overworld",
        &catalog,
        &mosaic,
        (a.clone(), sha(&fs::read(a).unwrap())),
        BUDGET,
        || false,
    )
    .unwrap();
    let second = map_edit::prepare_import_after(
        &f.world,
        "minecraft:the_nether",
        &catalog,
        &mosaic,
        (b.clone(), sha(&fs::read(b).unwrap())),
        std::slice::from_ref(&first),
        BUDGET,
        || false,
    )
    .unwrap();
    (first, second, mosaic)
}
fn candidates(f: &Fixture) -> (nbt_file::Candidate, nbt_file::Candidate) {
    let mut level = FileDocument::open(&f.root, &f.root.join("level.dat"), false).unwrap();
    level
        .document
        .apply_batch(&[
            Operation::Replace {
                path: "/Data/LevelName".into(),
                value: Value::String("已编辑世界".into()),
            },
            Operation::Replace {
                path: "/Data/DayTime".into(),
                value: Value::Long(54000),
            },
        ])
        .unwrap();
    assert_eq!(level.document.undo_depth(), 1);
    assert_eq!(
        level.document.inspect("/Data/RandomSeed").unwrap(),
        Value::Long(i64::MAX)
    );
    let candidate = level.candidate().unwrap();
    level.document.undo().unwrap();
    assert!(!level.document.changed());
    level.document.redo().unwrap();
    let mut player = FileDocument::open(
        &f.root,
        &f.root
            .join("playerdata/00000000-0000-0000-0000-000000000001.dat"),
        false,
    )
    .unwrap();
    let opaque = player.document.inspect("/Inventory/0").unwrap();
    player
        .document
        .apply_batch(&[
            Operation::Replace {
                path: "/XpTotal".into(),
                value: Value::Int(987),
            },
            Operation::Replace {
                path: if f.world.data_version == Some(3955) {
                    "/Inventory/0/count"
                } else {
                    "/Inventory/0/Count"
                }
                .into(),
                value: if f.world.data_version == Some(3955) {
                    Value::Int(16)
                } else {
                    Value::Byte(16)
                },
            },
        ])
        .unwrap();
    assert_eq!(player.document.undo_depth(), 1);
    player.document.undo().unwrap();
    assert_eq!(player.document.inspect("/Inventory/0").unwrap(), opaque);
    player.document.redo().unwrap();
    (candidate, player.candidate().unwrap())
}
pub(super) fn complete_metadata_pipeline() {
    metadata_preserves_pending_session();
    reusable_nbt_transactions();
    for version in [1343, 3465, 3955] {
        let f = Fixture::new(version);
        let all = player_catalog::load_all(&f.world, &|| false).unwrap();
        assert_eq!(all.players.len(), 2);
        assert_eq!(
            all.players
                .iter()
                .find(|player| player.uuid.unwrap()[15] == 1)
                .unwrap()
                .sources
                .len(),
            3
        );
        let current = player_catalog::load(&f.world, "minecraft:overworld", &|| false).unwrap();
        assert_eq!(current.players.len(), 1);
        assert!(player_catalog::load_all(&f.world, &|| true).is_err());
        let player_old = f.file("playerdata/00000000-0000-0000-0000-000000000001.dat");
        let (first, second, mosaic) = maps(&f);
        let ids = |candidate: &map_edit::Candidate| {
            candidate
                .changes
                .iter()
                .map(|change| match change {
                    map_edit::Change::Create { proof, .. } => proof.id,
                    _ => panic!("应为创建动作"),
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&first), vec![0]);
        assert_eq!(ids(&second), vec![1]);
        let combined = map_edit::compose(&f.world, &[&first, &second], BUDGET).unwrap();
        assert_eq!(combined.source_images.len(), 2);
        assert!(map_edit::compose(&f.world, &[&first, &second], 1).is_err());
        assert!(
            map_edit::compose_cancellable(&f.world, &[&first, &second], BUDGET, || true).is_err()
        );
        let mut bad = first.clone();
        let Some(map_edit::Counter::New { bytes, .. }) = &mut bad.counter else {
            panic!("首轮计数应新建")
        };
        bytes.push(0);
        assert!(map_edit::compose(&f.world, &[&bad, &second], BUDGET).is_err());
        let old_image = fs::read(&first.source_images[0].0).unwrap();
        fs::write(&first.source_images[0].0, b"changed").unwrap();
        assert!(map_edit::compose(&f.world, &[&first, &second], BUDGET).is_err());
        fs::write(&first.source_images[0].0, old_image).unwrap();
        let (world_edit, player_edit) = candidates(&f);
        let all = vec![
            MixedCandidate::Map(first),
            MixedCandidate::Nbt(world_edit),
            MixedCandidate::Map(second),
            MixedCandidate::Nbt(player_edit),
        ];
        assert!(mixed_transaction::prepare(&f.world, &all, true, "SAVE").is_err());
        assert_eq!(
            mixed_transaction::prepare(&f.world, &all, false, "SAVE").unwrap(),
            5
        );
        assert_eq!(f.file("level.dat"), f.level);
        assert_eq!(
            f.file("playerdata/00000000-0000-0000-0000-000000000001.dat"),
            player_old
        );
        assert!(!f.root.join("data/map_0.dat").exists());
        assert_eq!(mixed_transaction::finish(&f.world, false).unwrap(), 5);
        let loaded = map_data::read(&f.root, 0, Some(version)).unwrap();
        assert_eq!(
            &loaded.rgba[..4],
            &map_data::color_for_version(version, mosaic.tiles[0].colors[0] as u8).unwrap()
        );
        assert_eq!(
            map_data::read(&f.root, 1, Some(version)).unwrap().dimension,
            "minecraft:the_nether"
        );
        let counter = FileDocument::open(&f.root, &f.root.join("data/idcounts.dat"), true).unwrap();
        assert_eq!(
            counter
                .document
                .inspect(if version == 1343 { "/map" } else { "/data/map" })
                .unwrap(),
            if version == 1343 {
                Value::Short(1)
            } else {
                Value::Int(1)
            }
        );
        let level = FileDocument::open(&f.root, &f.root.join("level.dat"), true).unwrap();
        assert_eq!(
            level.document.inspect("/Data/LevelName").unwrap(),
            Value::String("已编辑世界".into())
        );
        assert_eq!(
            level.document.inspect("/Data/OpaqueWorld").unwrap(),
            Value::LongArray(vec![i64::MIN, i64::MAX])
        );
        let player = FileDocument::open(
            &f.root,
            &f.root
                .join("playerdata/00000000-0000-0000-0000-000000000001.dat"),
            true,
        )
        .unwrap();
        assert_eq!(
            player.document.inspect("/XpTotal").unwrap(),
            Value::Int(987)
        );
        assert_eq!(
            player.document.inspect("/OpaquePlayer").unwrap(),
            Value::ByteArray(vec![-128, 0, 127, -1])
        );
        assert!(mixed_transaction::pending_summary(&f.world)
            .unwrap()
            .is_none());
        // 已有计数后继续导入，并与删除旧地图合组；回滚保留全部原字节。
        let map0 = f.file("data/map_0.dat");
        let map1 = f.file("data/map_1.dat");
        let count = f.file("data/idcounts.dat");
        let catalog = map_catalog::scan(&f.root, || false).unwrap();
        let source = f.image("third.png");
        let next = map_edit::prepare_import(
            &f.world,
            "minecraft:overworld",
            &catalog,
            &mosaic,
            (source.clone(), sha(&fs::read(source).unwrap())),
            BUDGET,
            || false,
        )
        .unwrap();
        assert_eq!(ids(&next), vec![2]);
        let deletion = map_edit::prepare_delete(&f.world, &catalog.entries[0]).unwrap();
        let retry = vec![MixedCandidate::Map(next), MixedCandidate::Map(deletion)];
        for _ in 0..2 {
            assert_eq!(
                mixed_transaction::prepare(&f.world, &retry, false, "SAVE").unwrap(),
                3
            );
            assert_eq!(mixed_transaction::rollback(&f.world, false).unwrap(), 3);
        }
        assert_eq!(f.file("data/map_0.dat"), map0);
        assert_eq!(f.file("data/map_1.dat"), map1);
        assert_eq!(f.file("data/idcounts.dat"), count);
        assert!(!f.root.join("data/map_2.dat").exists());
        mixed_transaction::prepare(&f.world, &retry, false, "SAVE").unwrap();
        mixed_transaction::finish(&f.world, false).unwrap();
        assert!(!f.root.join("data/map_0.dat").exists());
        assert!(f.root.join("data/map_2.dat").is_file());
        assert_eq!(f.file("data/map_1.dat"), map1);
    }
    // 首次导入的 data 目录归属和回滚清理，单独使用新世界避免复用归档日志。
    let f = Fixture::new(3955);
    let (first, second, _) = maps(&f);
    mixed_transaction::prepare(
        &f.world,
        &[MixedCandidate::Map(first), MixedCandidate::Map(second)],
        false,
        "SAVE",
    )
    .unwrap();
    mixed_transaction::rollback(&f.world, false).unwrap();
    assert!(!f.root.join("data").exists());
    assert_eq!(f.file("level.dat"), f.level);
    // 独立 NBT 安装与回滚、只读和外部源变化不依赖地图事务。
    let f = Fixture::new(3955);
    let (level, _) = candidates(&f);
    for _ in 0..2 {
        nbt_file::apply(&level).unwrap();
        assert_ne!(f.file("level.dat"), f.level);
        nbt_file::rollback(&f.root, &f.root.join("level.dat"), false).unwrap();
        assert_eq!(f.file("level.dat"), f.level);
    }
    nbt_file::apply(&level).unwrap();
    nbt_file::finalize(&f.root, &f.root.join("level.dat"), false).unwrap();
    let mut read_only = FileDocument::open(&f.root, &f.root.join("level.dat"), true).unwrap();
    read_only
        .document
        .apply(&Operation::Replace {
            path: "/Data/Time".into(),
            value: Value::Long(1),
        })
        .unwrap();
    assert!(read_only.candidate().is_err());
    let mut stale = FileDocument::open(&f.root, &f.root.join("level.dat"), false).unwrap();
    stale
        .document
        .apply(&Operation::Replace {
            path: "/Data/Time".into(),
            value: Value::Long(2),
        })
        .unwrap();
    fs::write(
        f.root.join("level.dat"),
        gzip(&nbt(vec![field("Data", Value::Compound(vec![]))])),
    )
    .unwrap();
    assert!(stale.candidate().is_err());
}

fn metadata_preserves_pending_session() {
    use mcedit2_world::editing::{parse_state, Bounds, Session};
    let mut f = Fixture::new(3955);
    f.add_chunk();
    let original = f.file("region/r.0.0.mca");
    let mut session = Session::new(f.world.clone(), "minecraft:overworld").unwrap();
    let preview = session
        .prepare(
            Bounds::new((0, 0, 0), (0, 0, 0)).unwrap(),
            &parse_state("minecraft:dirt").unwrap(),
            None,
        )
        .unwrap();
    session.commit(preview).unwrap();
    let mut seed = FileDocument::open(&f.root, &f.root.join("level.dat"), false).unwrap();
    seed.document
        .apply(&Operation::Replace {
            path: "/Data/RandomSeed".into(),
            value: Value::Long(7),
        })
        .unwrap();
    let seed = seed.candidate().unwrap();
    seed.verify_session_metadata(&session).unwrap();
    let mut version = FileDocument::open(&f.root, &f.root.join("level.dat"), false).unwrap();
    version
        .document
        .apply(&Operation::Replace {
            path: "/Data/DataVersion".into(),
            value: Value::Int(3465),
        })
        .unwrap();
    assert!(version
        .candidate()
        .unwrap()
        .verify_session_metadata(&session)
        .is_err());
    let player = FileDocument::open(
        &f.root,
        &f.root
            .join("playerdata/00000000-0000-0000-0000-000000000001.dat"),
        false,
    )
    .unwrap();
    let overwrite = player.candidate_as(&f.root.join("level.dat")).unwrap();
    assert!(overwrite.verify_session_metadata(&session).is_err());
    nbt_file::apply(&seed).unwrap();
    nbt_file::finalize(&f.root, &f.root.join("level.dat"), false).unwrap();
    session
        .refresh_metadata(mcedit2_world::inspect_world(&f.root).unwrap())
        .unwrap();
    assert_eq!(session.original_world().seed, Some(7));
    assert_eq!(session.pending_cells(), 1);
    assert_eq!(session.undo_depth(), 1);
    session.undo().unwrap();
    assert!(!session.has_pending_changes());
    session.redo().unwrap();
    assert_eq!(session.pending_cells(), 1);
    assert_eq!(f.file("region/r.0.0.mca"), original);
}

fn reusable_nbt_transactions() {
    let f = Fixture::new(3955);
    let (level, player) = candidates(&f);
    let group = vec![level.clone(), player];
    for _ in 0..2 {
        nbt_file::group::prepare(&f.root, &group, false).unwrap();
        nbt_file::group::rollback(&f.root, false).unwrap();
        assert_eq!(f.file("level.dat"), f.level);
    }
    nbt_file::group::prepare(&f.root, &group, false).unwrap();
    nbt_file::group::finish(&f.root, false).unwrap();
    let mut source = FileDocument::open(&f.root, &f.root.join("level.dat"), false).unwrap();
    let before_mixed = f.file("level.dat");
    source
        .document
        .apply(&Operation::Replace {
            path: "/Data/Time".into(),
            value: Value::Long(123456),
        })
        .unwrap();
    let changes = [MixedCandidate::Nbt(source.candidate().unwrap())];
    for _ in 0..2 {
        mixed_transaction::prepare(&f.world, &changes, false, "SAVE").unwrap();
        mixed_transaction::rollback(&f.world, false).unwrap();
        assert_eq!(f.file("level.dat"), before_mixed);
    }
    mixed_transaction::prepare(&f.world, &changes, false, "SAVE").unwrap();
    mixed_transaction::finish(&f.world, false).unwrap();
    let source = FileDocument::open(&f.root, &f.root.join("level.dat"), false).unwrap();
    assert_eq!(
        source.document.inspect("/Data/Time").unwrap(),
        Value::Long(123456)
    );
    let copy = source.candidate_as(&f.root.join("copy.nbt")).unwrap();
    for _ in 0..2 {
        nbt_file::apply_as(&copy).unwrap();
        nbt_file::rollback_as(&f.root, &f.root.join("copy.nbt"), false).unwrap();
        assert!(!f.root.join("copy.nbt").exists());
    }
    nbt_file::apply_as(&copy).unwrap();
    nbt_file::finalize_as(&f.root, &f.root.join("copy.nbt"), false).unwrap();
    assert_eq!(f.file("copy.nbt"), f.file("level.dat"));
    let f = Fixture::new(3955);
    let (level, _) = candidates(&f);
    nbt_file::apply(&level).unwrap();
    nbt_file::rollback(&f.root, &f.root.join("level.dat"), false).unwrap();
    let backup = std::fs::read_dir(&f.root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".backup")
        })
        .unwrap();
    fs::write(backup, "损坏的备份".as_bytes()).unwrap();
    assert!(nbt_file::apply(&level).is_err());
    assert_eq!(f.file("level.dat"), f.level);
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
