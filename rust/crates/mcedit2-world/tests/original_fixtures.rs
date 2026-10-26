//! Read-only regression on the original checked-in Java fixtures.
use mcedit2_world::{inspect_world, RegionIndex};
use std::fs;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../test_files")
}

/// Native Rust must open old Java sample data without running the removed
/// Python fixture generator, and inspection must never write to that data.
#[test]
fn original_java_world_fixtures_are_read_only() {
    for name in ["AnvilWorld", "AnvilWorld_1.8"] {
        let root = fixtures().join(name);
        let metadata_file = root.join("level.dat");
        let before = fs::read(&metadata_file).expect("checked-in original level.dat");
        let summary = inspect_world(&root).expect("native reader opens original Java fixture");
        assert_eq!(summary.root, fs::canonicalize(&root).unwrap());
        assert!(summary.total_regions() > 0, "{name}: no Region files found");
        assert!(summary.total_chunks() > 0, "{name}: no source chunks indexed");
        assert_eq!(fs::read(&metadata_file).unwrap(), before, "{name}: metadata was modified");
    }
}

/// Confirm signed Region lookup, original chunk NBT and immutable sector data.
/// The negative-X Region is from the repository's original 1.8 fixture.
#[test]
fn negative_region_chunk_nbt_is_inspected_without_mutating_source() {
    let path = fixtures()
        .join("AnvilWorld_1.8")
        .join("region")
        .join("r.-1.0.mca");
    let before = fs::read(&path).expect("original negative Region");
    let index = RegionIndex::open(&path).expect("valid original Region header");
    assert!(index.chunk_count() > 0);

    let (x, z) = (0u8..32)
        .flat_map(|x| (0u8..32).map(move |z| (x, z)))
        .find(|&(x, z)| index.slot(x, z).is_some())
        .expect("at least one negative-X source chunk");
    let cx = i32::from(x) - 32;
    let cz = i32::from(z);
    let first = index
        .read_chunk_at(cx, cz)
        .expect("original compressed NBT")
        .expect("existing original chunk");
    assert_eq!(first.first(), Some(&10), "Java root NBT must be a compound");
    assert_eq!(first, index.read_chunk_at(cx, cz).unwrap().unwrap());
    assert_eq!(fs::read(&path).unwrap(), before, "inspecting changed Region bytes");
}
