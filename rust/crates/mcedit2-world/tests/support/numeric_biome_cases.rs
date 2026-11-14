//! 旧 biome 整批验收：真实 Region 容器、捕获、变换、载荷交换、历史与保存 / 回滚。
use super::*;
use mcedit2_world::{
    clipboard::{CaptureOptions, HorizontalTransform, PasteOptions},
    nbt_document::{self, Document, Operation, Value},
    numeric_biome::Storage,
    structure_exchange::Format,
    structure_transform::Transform,
};
pub(super) fn source(storage: Storage) -> (Vec<u8>, i32) {
    let numeric = storage == Storage::ByteColumns;
    let version: i32 = match storage {
        Storage::ByteColumns => 1343,
        Storage::IntColumns => 1631,
        Storage::IntQuarts => 2586,
    };
    let mut fields = vec![tag(1, "Y", &[0])];
    if numeric {
        fields.push(byte_array("Blocks", &vec![1; 4096]));
        fields.push(byte_array("Data", &vec![0; 2048]));
        fields.push(byte_array("BlockLight", &vec![0; 2048]));
        fields.push(byte_array("SkyLight", &vec![0; 2048]));
    } else {
        let mut palette = vec![10];
        palette.extend(1i32.to_be_bytes());
        palette.extend(compound(vec![tag(
            8,
            "Name",
            &nbt_string("minecraft:stone"),
        )]));
        fields.push(tag(9, "Palette", &palette));
    }
    let mut sections = vec![10];
    sections.extend(1i32.to_be_bytes());
    sections.extend(compound(fields));
    let count = if storage == Storage::IntQuarts {
        1024
    } else {
        256
    };
    let value = match storage {
        Storage::ByteColumns => {
            Value::ByteArray((0..count).map(|i| (200 + i) as u8 as i8).collect())
        }
        Storage::IntColumns => Value::IntArray((0..count).map(|i| -1000 + i).collect()),
        Storage::IntQuarts => Value::IntArray((0..count).map(|i| 100000 + i).collect()),
    };
    let raw = tag(
        10,
        "",
        &compound(vec![
            tag(3, "DataVersion", &version.to_be_bytes()),
            tag(
                10,
                "Level",
                &compound(vec![
                    tag(3, "xPos", &0i32.to_be_bytes()),
                    tag(3, "zPos", &0i32.to_be_bytes()),
                    tag(9, "Sections", &sections),
                    tag(9, "TileEntities", &[10, 0, 0, 0, 0]),
                    tag(8, "OpaqueSibling", &nbt_string("保留所有原字节")),
                    tag(1, "LightPopulated", &[1]),
                    tag(11, "HeightMap", &{
                        let mut v = 256i32.to_be_bytes().to_vec();
                        for _ in 0..256 {
                            v.extend(16i32.to_be_bytes());
                        }
                        v
                    }),
                ]),
            ),
        ]),
    );
    (
        nbt_document::patch(
            &raw,
            &Operation::InsertField {
                path: "/Level".into(),
                name: "Biomes".into(),
                value,
            },
        )
        .unwrap(),
        version,
    )
}
pub(super) fn complete_legacy_biome_pipeline() {
    for storage in [
        Storage::ByteColumns,
        Storage::IntColumns,
        Storage::IntQuarts,
    ] {
        let fixture = NativeWorld::create(true);
        let (raw, version) = source(storage);
        overwrite_region(&fixture, &raw);
        let original = fixture.bytes_now();
        let mut world = fixture.metadata();
        world.data_version = Some(version);
        world.version_name = None;
        let mut session = Session::new(world.clone(), "minecraft:overworld").unwrap();
        let side = if storage == Storage::IntQuarts { 4 } else { 2 };
        let bounds = Bounds::new((0, 0, 0), (side - 1, side - 1, side - 1)).unwrap();
        let clip = session
            .capture_structure_cancellable(
                bounds,
                CaptureOptions {
                    entities: false,
                    biomes: true,
                },
                || false,
            )
            .unwrap();
        let volume = clip.numeric_biomes.as_ref().unwrap();
        assert!(clip.biomes.is_none());
        assert_eq!(volume.data_version, Some(version));
        let first = match storage {
            Storage::ByteColumns => 200,
            Storage::IntColumns => -1000,
            Storage::IntQuarts => 100000,
        };
        assert_eq!(volume.cells[0], first);
        let decoded = mcedit2_world::sections::decode_sections(&raw).unwrap();
        let bad = nbt_document::patch(
            &raw,
            &Operation::Replace {
                path: "/Level/Biomes".into(),
                value: Value::IntArray(vec![1; 1000]),
            },
        )
        .unwrap();
        assert!(mcedit2_world::sections::decode_sections(&bad).is_err());
        if storage == Storage::IntQuarts {
            let unbound = nbt_document::patch(
                &raw,
                &Operation::Replace {
                    path: "/DataVersion".into(),
                    value: Value::Int(2730),
                },
            )
            .unwrap();
            let unbound = mcedit2_world::sections::decode_sections(&unbound).unwrap();
            assert_eq!(
                unbound.numeric_biomes.as_ref().unwrap().values,
                decoded.numeric_biomes.as_ref().unwrap().values
            );
            assert_eq!(unbound.numeric_biome(0, 0, 0), None);
            let expanded = nbt_document::patch(
                &raw,
                &Operation::Replace {
                    path: "/Level/Biomes".into(),
                    value: Value::IntArray(vec![100000; 1536]),
                },
            )
            .unwrap();
            let expanded = mcedit2_world::sections::decode_sections(&expanded).unwrap();
            assert_eq!(expanded.block(0, 0, 0).unwrap().name, "minecraft:stone");
            assert_eq!(expanded.numeric_biomes.as_ref().unwrap().values.len(), 1536);
            assert_eq!(expanded.numeric_biome(0, 0, 0), None);
        }
        let column = mcedit2_world::biome_source::Column::from_chunk(&decoded).unwrap();
        let mut region = mcedit2_world::biome_source::Region::new(None);
        region.insert(0, 0, column);
        assert_eq!(region.numeric_sample((0, 0, 0)), Some(first));
        assert_eq!(region.sample((0, 0, 0)), None);
        assert!(!region.is_empty());
        let overlay = region
            .clone()
            .with_numeric_biomes([8, 0, 8], volume)
            .unwrap();
        assert_eq!(overlay.numeric_sample((8, 0, 8)), Some(first));
        assert_eq!(
            overlay.numeric_at((8, 0, 8)).unwrap().data_version,
            Some(version)
        );
        assert_eq!(overlay.sample((8, 0, 8)), None);
        assert!(!overlay.same_neighborhood(&region, 0, 0));
        assert!(overlay.same_neighborhood(&overlay.clone(), 0, 0));
        assert!(overlay
            .with_block_biomes([0, 0, 0], [1, 1, 1], &["minecraft:plains".into()])
            .is_err());
        let mut detached = clip.clone();
        detached.source_bound = false;
        let mut foreign = world.clone();
        foreign.root = fixture.path.join("other-world");
        assert!(detached.validate(&foreign, "minecraft:overworld").is_err());
        let rotated = detached
            .resampled_cancellable(
                Transform {
                    rotation: [0., 20., 0.],
                    rotate_states: false,
                    ..Default::default()
                },
                || false,
            )
            .unwrap();
        let rotated_biomes = rotated.numeric_biomes.as_ref().unwrap();
        assert_eq!(rotated_biomes.data_version, volume.data_version);
        assert_eq!(rotated_biomes.size, rotated.size);
        assert_eq!(rotated_biomes.cells.len(), rotated.blocks.len());
        assert!(rotated_biomes
            .cells
            .iter()
            .all(|value| volume.cells.contains(value)));
        let mirror = detached.transformed(HorizontalTransform::MirrorX).unwrap();
        assert_eq!(
            mirror.numeric_biomes.as_ref().unwrap().cells[0],
            volume.cells[side as usize - 1]
        );
        let scaled = detached
            .resampled_cancellable(
                Transform {
                    scale: [2., 1., 1.],
                    rotate_states: false,
                    ..Default::default()
                },
                || false,
            )
            .unwrap();
        let values = &scaled.numeric_biomes.as_ref().unwrap().cells;
        assert_eq!(values[0], values[1]);
        assert_eq!(values[0], first);
        assert_eq!(values[2], volume.cells[1]);
        let directory = std::env::temp_dir().join(format!(
            "mcedit2-numeric-biomes-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("snapshot.mcedit2clip");
        Format::Clipboard.write(&world, &detached, &path).unwrap();
        let restored = Format::Clipboard
            .read(&world, "minecraft:overworld", &path)
            .unwrap();
        assert_eq!(restored.numeric_biomes, detached.numeric_biomes);
        if storage == Storage::ByteColumns {
            let path = directory.join("snapshot.schematic");
            Format::Legacy.write(&world, &detached, &path).unwrap();
            let restored = Format::Legacy
                .read(&world, "minecraft:overworld", &path)
                .unwrap();
            assert_eq!(restored.numeric_biomes, detached.numeric_biomes);
        }
        assert!(Format::SpongeV3
            .write(&world, &detached, &directory.join("unsupported.schem"))
            .is_err());
        let destination = if storage == Storage::IntQuarts {
            (4, 4, 4)
        } else {
            (4, 1, 4)
        };
        let (preview, _) = session
            .prepare_resampled_structure_paste_cancellable(
                &clip,
                Transform::default(),
                destination,
                PasteOptions {
                    include_biomes: true,
                },
                None,
                || false,
            )
            .unwrap();
        session.commit_structure(preview).unwrap();
        assert_eq!(session.pending_cells(), 0);
        assert!(session.has_pending_changes());
        let staged = session.staged_biome_chunk(0, 0).unwrap().unwrap().to_vec();
        let data = mcedit2_world::sections::decode_sections(&staged).unwrap();
        assert_eq!(data.numeric_biome(4, destination.1, 4), Some(first));
        if storage != Storage::IntQuarts {
            assert_eq!(data.numeric_biome(4, 200, 4), Some(first));
        } else {
            assert_eq!(data.numeric_biome(4, 0, 4), decoded.numeric_biome(4, 0, 4));
        }
        assert_eq!(
            Document::new(staged.clone())
                .unwrap()
                .inspect("/Level/OpaqueSibling")
                .unwrap(),
            Value::String("保留所有原字节".into())
        );
        let proof = session.biome_proofs().pop().unwrap();
        let json = serde_json::to_vec(&proof).unwrap();
        let replay: mcedit2_world::biome_editing::Proof = serde_json::from_slice(&json).unwrap();
        assert_eq!(replay.replay(&raw).unwrap(), staged);
        let mut wrong = replay.clone();
        wrong.numeric.as_mut().unwrap().edits[0].original += 1;
        assert!(wrong.replay(&raw).is_err());
        assert!(session.undo().unwrap());
        assert!(!session.has_pending_changes());
        assert!(session.redo().unwrap());
        assert_eq!(session.staged_biome_chunk(0, 0).unwrap().unwrap(), staged);
        let plans = session.preview_be_region_candidates(471).unwrap();
        assert_eq!(plans.len(), 1);
        assert!(!plans[0].biome_proofs.is_empty());
        assert_eq!(fixture.bytes_now(), original);
        let receipt = mcedit2_world::region_transaction::apply_single_region(
            &world,
            "minecraft:overworld",
            &plans[0],
        )
        .unwrap();
        assert_ne!(fixture.bytes_now(), original);
        assert_eq!(fs::read(&receipt.backup_path).unwrap(), original);
        let read =
            mcedit2_world::block_lookup::read_world_chunk(&world, "minecraft:overworld", 0, 0)
                .unwrap()
                .unwrap();
        assert_eq!(read.numeric_biome(4, destination.1, 4), Some(first));
        mcedit2_world::region_transaction::rollback_pending_region(
            &world,
            "minecraft:overworld",
            0,
            0,
            "mca",
        )
        .unwrap();
        assert_eq!(fixture.bytes_now(), original);
        assert!(session.undo().unwrap());
        let before = session.revision();
        let mut mismatched = detached.clone();
        mismatched.numeric_biomes.as_mut().unwrap().data_version = Some(version + 1);
        assert!(session
            .prepare_resampled_structure_paste_cancellable(
                &mismatched,
                Transform::default(),
                destination,
                PasteOptions {
                    include_biomes: true
                },
                None,
                || false
            )
            .is_err());
        assert!(std::sync::Arc::ptr_eq(&before, &session.revision()));
        if storage == Storage::IntQuarts {
            assert!(session
                .prepare_resampled_structure_paste_cancellable(
                    &clip,
                    Transform::default(),
                    (5, 4, 4),
                    PasteOptions {
                        include_biomes: true
                    },
                    None,
                    || false
                )
                .is_err());
        }
        let (preview, _) = session
            .prepare_resampled_structure_paste_cancellable(
                &clip,
                Transform::default(),
                destination,
                PasteOptions {
                    include_biomes: true,
                },
                None,
                || false,
            )
            .unwrap();
        let changed = nbt_document::patch(
            &raw,
            &Operation::Replace {
                path: "/Level/OpaqueSibling".into(),
                value: Value::String("外部变化".into()),
            },
        )
        .unwrap();
        overwrite_region(&fixture, &changed);
        assert!(session.commit_structure(preview).is_err());
        assert!(!session.has_pending_changes());
        if storage == Storage::ByteColumns {
            overwrite_region(&fixture, &raw);
            combined_relight(&world, &fixture, &raw);
        }
        fs::remove_dir_all(directory).unwrap();
    }
}

fn combined_relight(world: &WorldSummary, fixture: &NativeWorld, raw: &[u8]) {
    // 正式加载器给事务层提供 canonical 路径；临时元数据也遵守同一约束。
    let mut canonical = world.clone();
    canonical.root = fs::canonicalize(&canonical.root).unwrap();
    for dimension in &mut canonical.dimensions {
        dimension.region_dir = fs::canonicalize(&dimension.region_dir).unwrap();
    }
    let world = &canonical;
    use mcedit2_world::{
        chunk_postprocess::Policy,
        legacy_lighting::{Dimension, Traits},
        mixed_transaction::{self, MixedCandidate},
    };
    let level = tag(
        10,
        "",
        &compound(vec![tag(
            10,
            "Data",
            &compound(vec![tag(3, "DataVersion", &1343i32.to_be_bytes())]),
        )]),
    );
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(&level).unwrap();
    fs::write(world.root.join("level.dat"), gzip.finish().unwrap()).unwrap();
    let policy = Policy::LegacyRelight {
        profile: Dimension::confirm(
            world,
            "minecraft:overworld",
            false,
            "明确的无天空光测试维度",
        )
        .unwrap(),
        traits: Traits::vanilla_1_12(true).unwrap(),
        operation_budget: 1_000_000,
        memory_budget: 128 * 1024 * 1024,
    };
    let mut session = Session::new(world.clone(), "minecraft:overworld").unwrap();
    let bounds = Bounds::new((0, 0, 0), (1, 1, 1)).unwrap();
    let options = CaptureOptions {
        entities: false,
        biomes: true,
    };
    let source = session
        .capture_structure_cancellable(bounds, options, || false)
        .unwrap();
    let (preview, _) = session
        .prepare_resampled_structure_paste_cancellable(
            &source,
            Transform::default(),
            (4, 1, 4),
            PasteOptions {
                include_biomes: true,
            },
            None,
            || false,
        )
        .unwrap();
    session.commit_structure(preview).unwrap();
    let meta = session
        .prepare_postprocessed_candidates(&policy, 501, || false)
        .unwrap();
    assert!(matches!(&meta[..], [MixedCandidate::Region { .. }]));
    assert!(session.undo().unwrap());
    let point = Bounds::new((1, 1, 1), (1, 1, 1)).unwrap();
    session
        .commit(
            session
                .prepare(point, &parse_state("numeric:41[data=0]").unwrap(), None)
                .unwrap(),
        )
        .unwrap();
    let source = session
        .capture_structure_cancellable(bounds, options, || false)
        .unwrap();
    let (preview, _) = session
        .prepare_resampled_structure_paste_cancellable(
            &source,
            Transform::default(),
            (4, 1, 4),
            PasteOptions {
                include_biomes: true,
            },
            None,
            || false,
        )
        .unwrap();
    session.commit_structure(preview).unwrap();
    let plans = session
        .prepare_postprocessed_candidates(&policy, 502, || false)
        .unwrap();
    let [MixedCandidate::LegacyLight(batch)] = &plans[..] else {
        panic!("方块与 biome 应为同一光照候选")
    };
    assert!(!batch.proof.biome_proofs.is_empty());
    let mut tampered = plans.clone();
    if let MixedCandidate::LegacyLight(batch) = &mut tampered[0] {
        batch.proof.biome_proofs.clear();
    }
    assert!(mixed_transaction::prepare(world, &tampered, false, "SAVE").is_err());
    assert_eq!(fixture.bytes_now(), region_bytes(raw));
    assert_eq!(
        mixed_transaction::prepare(world, &plans, false, "SAVE").unwrap(),
        1
    );
    assert_eq!(mixed_transaction::finish(world, false).unwrap(), 1);
    let read = mcedit2_world::block_lookup::read_world_chunk(world, "minecraft:overworld", 0, 0)
        .unwrap()
        .unwrap();
    assert_eq!(read.numeric_biome(4, 1, 4), Some(200));
    assert_eq!(read.block(5, 2, 5).unwrap().name, "numeric:41");
}
