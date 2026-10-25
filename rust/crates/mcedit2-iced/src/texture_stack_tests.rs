//! Filesystem and ZIP overlay regressions without an installed Minecraft client.
use super::*;
use image::ImageEncoder;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
static COUNTER: AtomicU64 = AtomicU64::new(0);
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "mcedit2-stack-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn path(&self, name: &str) -> std::path::PathBuf { self.0.join(name) }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
fn png(rgba: &[u8], height: u32) -> Vec<u8> {
    let mut data = Vec::new();
    image::codecs::png::PngEncoder::new(&mut data)
        .write_image(rgba, 1, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    data
}
fn put(root: &Path, name: &str, data: &[u8]) {
    let p = root.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}
fn color(bank: &TextureBank, id: &str) -> [u8; 4] {
    bank.get(id).unwrap().pixel(0.5, 0.5)
}
const STONE: &str = "assets/minecraft/textures/block/stone.png";
#[test]
fn zip_overlay_preserves_lower_models_and_sprites_without_stale_animation() {
    let temp = Fixture::new();
    let base = temp.path("base");
    let overlay = temp.path("overlay.zip");
    put(&base, STONE, &png(&[200, 0, 0, 255, 0, 200, 0, 255], 2));
    put(&base, "assets/minecraft/textures/block/stone.png.mcmeta",
        br#"{"animation":{"frames":[1,0]}}"#);
    put(&base, "assets/minecraft/textures/block/dirt.png",
        &png(&[120, 60, 20, 255], 1));
    put(&base, "assets/minecraft/models/block/stone.json",
        br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/stone"}}"#);
    let mut zip = zip::ZipWriter::new(File::create(&overlay).unwrap());
    zip.start_file("assets/minecraft/textures/blocks/stone.png",
        zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(&png(&[0, 0, 250, 255], 1)).unwrap();
    zip.start_file("assets/example/textures/block/casing.png",
        zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(&png(&[60, 70, 80, 255], 1)).unwrap();
    zip.finish().unwrap();
    let bank = TextureBank::open_stack(&[base.clone(), overlay.clone()]).unwrap();
    assert_eq!(color(&bank, "minecraft:stone"), [0, 0, 250, 255]);
    assert_eq!(color(&bank, "minecraft:dirt"), [120, 60, 20, 255]);
    assert_eq!(color(&bank, "example:casing"), [60, 70, 80, 255]);
    assert_eq!(bank.animation_count(), 0);
    assert_eq!(bank.model_count(), 1);
    assert!(bank.atlas().unwrap().entries.contains_key("minecraft:stone"));
    let reversed = TextureBank::open_stack(&[overlay, base]).unwrap();
    assert_eq!(color(&reversed, "minecraft:stone"), [0, 200, 0, 255]);
    assert_eq!(reversed.animation_count(), 1);
    assert!(TextureBank::open_stack(&[]).is_err());
    assert!(TextureBank::open_stack(&vec![temp.path("base"); 33]).is_err());
}
#[test]
fn metadata_only_overlay_changes_schedule_without_replacing_png() {
    let temp = Fixture::new();
    let base = temp.path("base");
    let meta = temp.path("meta");
    put(&base, STONE, &png(&[255, 0, 0, 255, 0, 255, 0, 255], 2));
    put(&base, "assets/minecraft/textures/block/stone.png.mcmeta",
        br#"{"animation":{"frames":[0,1]}}"#);
    put(&meta, "assets/minecraft/textures/block/stone.png.mcmeta",
        br#"{"animation":{"frames":[1,0]}}"#);
    let bank = TextureBank::open_stack(&[base.clone(), meta]).unwrap();
    assert_eq!(bank.animation_count(), 1);
    assert_eq!(color(&bank, "minecraft:stone"), [0, 255, 0, 255]);
    assert!(TextureBank::open_stack(&[base, temp.path("missing.zip")]).is_err());
}

#[test]
fn corrupt_modern_png_does_not_shadow_valid_legacy_texture_in_either_zip_order() {
    for modern_first in [true, false] {
        let temp = Fixture::new();
        let pack = temp.path("aliases.zip");
        let modern = "assets/example/textures/block/gearbox.png";
        let legacy = "assets/example/textures/blocks/gearbox.png";
        let mut zip = zip::ZipWriter::new(File::create(&pack).unwrap());
        for name in if modern_first { [modern, legacy] } else { [legacy, modern] } {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            if name == modern {
                zip.write_all(b"invalid PNG: corrupt high-priority alias").unwrap();
            } else {
                zip.write_all(&png(&[41, 92, 203, 255], 1)).unwrap();
            }
        }
        zip.finish().unwrap();
        let bank = TextureBank::open(&pack).unwrap();
        assert_eq!(color(&bank, "example:gearbox"), [41, 92, 203, 255]);
        assert!(bank.warnings.iter().any(|s| s.contains("example:gearbox")));
    }
}
#[test]
fn valid_modern_png_wins_over_legacy_regardless_of_archive_order() {
    for modern_first in [true, false] {
        let temp = Fixture::new();
        let pack = temp.path("aliases.zip");
        let modern = "assets/example/textures/block/casing.png";
        let legacy = "assets/example/textures/blocks/casing.png";
        let mut zip = zip::ZipWriter::new(File::create(&pack).unwrap());
        for name in if modern_first { [modern, legacy] } else { [legacy, modern] } {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            let pixel = if name == modern {
                [10, 244, 88, 255]
            } else {
                [201, 15, 31, 255]
            };
            zip.write_all(&png(&pixel, 1)).unwrap();
        }
        zip.finish().unwrap();
        let bank = TextureBank::open(&pack).unwrap();
        assert_eq!(color(&bank, "example:casing"), [10, 244, 88, 255]);
    }
}
#[test]
fn corrupt_upper_layer_sprite_preserves_lower_png_and_animation_metadata() {
    let temp = Fixture::new();
    let base = temp.path("base");
    let upper = temp.path("override.zip");
    put(&base, STONE, &png(&[255, 0, 0, 255, 0, 255, 0, 255], 2));
    put(&base, "assets/minecraft/textures/block/stone.png.mcmeta",
        br#"{"animation":{"frames":[1,0]}}"#);
    let mut zip = zip::ZipWriter::new(File::create(&upper).unwrap());
    zip.start_file(STONE, zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(b"not a valid PNG").unwrap();
    zip.finish().unwrap();
    let bank = TextureBank::open_stack(&[base, upper]).unwrap();
    assert_eq!(color(&bank, "minecraft:stone"), [0, 255, 0, 255]);
    assert_eq!(bank.animation_count(), 1);
    assert!(bank.warnings.iter().any(|s| s.contains("minecraft:stone")));
}

#[test]
fn zip_and_directory_use_the_same_asset_classifier_and_overlay_rules() {
    let temp = Fixture::new();
    let folder = temp.path("pack_dir");
    let archive = temp.path("pack.zip");
    let data = [
        (
            "assets/minecraft/textures/block/stone.png",
            png(&[49, 74, 201, 255], 1),
        ),
        (
            "assets/minecraft/textures/blocks/stone.png",
            png(&[201, 54, 74, 255], 1),
        ),
        (
            "assets/example/textures/block/machine/casing.png",
            png(&[81, 119, 39, 255], 1),
        ),
        (
            "assets/example/models/block/machine/casing.json",
            br##"{"parent":"minecraft:block/cube_all","textures":{"all":"example:block/machine/casing"}}"##.to_vec(),
        ),
        (
            "assets/example/blockstates/machine/casing.json",
            br#"{"variants":{"":{"model":"example:block/machine/casing"}}}"#.to_vec(),
        ),
    ];
    for (name, bytes) in &data {
        put(&folder, name, bytes);
    }
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    for (name, bytes) in data.iter().rev() {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    let dir_bank = TextureBank::open(&folder).unwrap();
    let zip_bank = TextureBank::open(&archive).unwrap();
    for bank in [&dir_bank, &zip_bank] {
        assert_eq!(bank.model_count(), 2);
        assert_eq!(color(bank, "minecraft:stone"), [49, 74, 201, 255]);
        assert_eq!(color(bank, "example:machine/casing"), [81, 119, 39, 255]);
        assert_eq!(bank.animation_count(), 0);
        assert!(bank.warnings.is_empty(), "{:?}", bank.warnings);
    }
    assert_eq!(dir_bank.count(), zip_bank.count());
    assert_eq!(dir_bank.atlas().unwrap().entries, zip_bank.atlas().unwrap().entries);
}
