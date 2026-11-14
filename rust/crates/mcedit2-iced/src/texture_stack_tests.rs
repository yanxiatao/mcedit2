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
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

fn native_animation_and_resource_boundaries_keep_pixels_and_source_identity() {
    let fixture = Fixture::new();
    let base = fixture.path("source");
    let overlay = fixture.path("metadata.zip");
    let image = png(&[255, 0, 0, 255, 0, 255, 0, 255], 2);
    put(&base, STONE, &image);
    put(
        &base,
        "assets/demo/blockstates/cube.json",
        br#"{"variants":{"":{"model":"demo:block/cube"}}}r"#,
    );
    put(
        &base,
        "assets/demo/models/block/cube.json",
        br#"{"parent":"block/cube_all","textures":{"all":"minecraft:block/stone"}}s"#,
    );
    let metadata_name = "assets/minecraft/textures/block/stone.png.mcmeta";
    let write_overlay = |metadata: &[u8]| {
        let mut archive = zip::ZipWriter::new(File::create(&overlay).unwrap());
        archive
            .start_file(metadata_name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(metadata).unwrap();
        archive.finish().unwrap();
    };
    write_overlay(br#"{"animation":{"frames":[-1,1]}}"#);
    let bank = TextureBank::open_stack_java(&[base.clone(), overlay.clone()]).unwrap();
    assert_eq!(color(&bank, "minecraft:stone"), [255, 0, 0, 255]);
    assert_eq!(bank.animation_count(), 0);
    // 图集首帧 4 字节和独立整图 8 字节都常驻，诊断与采样元数据单独计量。
    assert_eq!(
        bank.statistics.decoded_bytes, 12,
        "诊断常驻内存不能混入实际图片 RGBA 字节统计"
    );
    assert!(
        bank.warnings.iter().any(|d| d.contains("引用 -1")
            && d.contains(base.to_str().unwrap())
            && d.contains(overlay.to_str().unwrap())
            && d.contains(metadata_name)),
        "{:?}",
        bank.warnings
    );
    let state = mcedit2_world::editing::parse_state("demo:cube").unwrap();
    let (uv, _, page) = bank
        .atlas_face_style_at(&state, crate::isometric::Face::Top, 0, (0, 0, 0))
        .unwrap();
    assert_eq!(bank.atlas_pixel(uv[0], page), Some([255, 0, 0, 255]));
    assert!(bank.warnings.iter().any(|d| d.contains("未消费")
        && d.contains("cube.json")
        && d.contains(base.to_str().unwrap())));
    write_overlay(br#"{"animation":{"frames":[1,0]}}tail"#);
    let bank = TextureBank::open_stack_java(&[base.clone(), overlay.clone()]).unwrap();
    assert_eq!(color(&bank, "minecraft:stone"), [0, 255, 0, 255]);
    assert_eq!(bank.animation_count(), 1);
    assert!(bank.warnings.iter().any(|d| d.contains("未消费")
        && d.contains(metadata_name)
        && d.contains(overlay.to_str().unwrap())));
    write_overlay(br#"{"animation":{"frames":[{"index":-1}]}}"#);
    let bank = TextureBank::open_stack_java(&[base.clone(), overlay.clone()]).unwrap();
    assert_eq!(bank.statistics.failed_png, 1);
    assert!(!bank
        .atlas()
        .unwrap()
        .entries
        .contains_key("minecraft:stone"));
    assert_eq!(
        bank.simple_image("minecraft:stone")
            .unwrap()
            .pixel(0.5, 0.25),
        [255, 0, 0, 255]
    );
    assert!(
        bank.warnings
            .iter()
            .any(|error| error.contains("stone.png") && error.contains("无效")),
        "图集动画错误必须报告，独立整图继续使用自己的实际图片：{:?}",
        bank.warnings
    );
    let mut archive = zip::ZipWriter::new(File::create(&overlay).unwrap());
    archive
        .start_file(
            "assets/demo/models/block/cube.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(br#"{"parent":"#).unwrap();
    archive.finish().unwrap();
    let bank = TextureBank::open_stack_java(&[base.clone(), overlay.clone()]).unwrap();
    let (uv, _, page) = bank
        .atlas_face_style_at(&state, crate::isometric::Face::Top, 0, (0, 0, 0))
        .unwrap();
    assert_eq!(
        bank.atlas_pixel(uv[0], page),
        Some([248, 0, 248, 255]),
        "上层定义损坏时必须显示缺失材质，不能保留旧纹理"
    );
    assert!(bank.warnings.iter().any(|d| d.contains("模型")
        && d.contains("无效")
        && d.contains(overlay.to_str().unwrap())));
    assert_eq!(std::fs::read(base.join(STONE)).unwrap(), image);
}

fn java_composite_children_keep_texture_uv_geometry_and_declared_alpha() {
    use crate::isometric::Face;
    let fixture = Fixture::new();
    let root = fixture.path("java_composite");
    put(
        &root,
        "assets/demo/blockstates/machine.json",
        br#"{"variants":{"":{"model":"demo:block/machine"}}}"#,
    );
    put(&root,"assets/demo/models/block/machine.json",br#"{"loader":"neoforge:composite","children":{"a":{"parent":"demo:block/part","textures":{"all":"demo:block/red"},"render_type":"minecraft:solid","transform":{"translation":[-0.25,0,0],"scale":[0.5,1,1],"origin":"center"}},"b":{"parent":"demo:block/part","textures":{"all":"demo:block/blue"},"render_type":"minecraft:translucent","transform":{"translation":[0.25,0,0],"scale":[0.5,1,1],"origin":"center"}}}}"#);
    put(
        &root,
        "assets/demo/models/block/part.json",
        br#"{"parent":"block/cube_all"}"#,
    );
    put(
        &root,
        "assets/demo/textures/block/red.png",
        &png(&[255, 0, 0, 128], 1),
    );
    put(
        &root,
        "assets/demo/textures/block/blue.png",
        &png(&[0, 0, 255, 128], 1),
    );
    let bank = Arc::new(TextureBank::open_stack_java(std::slice::from_ref(&root)).unwrap());
    assert!(bank.warnings.is_empty(), "{:?}", bank.warnings);
    let state = mcedit2_world::editing::parse_state("demo:machine").unwrap();
    assert_eq!(bank.geometry(&state).unwrap().len(), 2);
    let red = bank
        .atlas_face_style_at(&state, Face::Top, 0, (0, 0, 0))
        .unwrap();
    let blue = bank
        .atlas_face_style_at(&state, Face::Top, 1, (0, 0, 0))
        .unwrap();
    assert_eq!(red.1, AlphaMode::AuthoredOpaque);
    assert_eq!(blue.1, AlphaMode::DeclaredBlend);
    assert_ne!(red.0, blue.0);
    let scene = crate::spectator::SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface: Arc::new(mcedit2_world::surface::SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 15,
            width: 1,
            depth: 1,
            voxels: vec![mcedit2_world::surface::SurfaceVoxel::new(
                [0, 0, 0],
                mcedit2_world::surface::VoxelData {
                    state: state.into(),
                    biome: None,
                    original_block_entity: false,
                    faces: 63,
                    kind: mcedit2_world::surface::VoxelMaterial::Solid,
                    staged: false,
                    span: 1,
                    fluid: None,
                    light: Default::default(),
                },
            )],
            missing_columns: 0,
            source_chunks: 1,
            source_tiles: vec![],
        }),
        texture_bank: Some(bank),
        show_textures: true,
        block_layers: Default::default(),
        camera: crate::spectator::Camera::default(),
        quality: crate::gpu_mesh::Quality::Fast,
        cache: Default::default(),
        pending: Default::default(),
        presented: Default::default(),
        presented_view: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Default::default(),
        lighting: crate::lighting::Table::unlit(),
    };
    let vertices = crate::spectator::build(&scene);
    assert_eq!(vertices.len(), 72);
    let left = vertices
        .iter()
        .filter(|v| v.mode & crate::material_mode::FORCE_OPAQUE_ALPHA != 0)
        .collect::<Vec<_>>();
    let right = vertices
        .iter()
        .filter(|v| v.mode & 3 == 2)
        .collect::<Vec<_>>();
    assert_eq!((left.len(), right.len()), (36, 36));
    assert!(
        vertices
            .iter()
            .all(|v| v.mode & crate::material_mode::VIEW_OFFSET_Z == 0),
        "声明通道不能改变普通材质的几何深度"
    );
    assert!(left.iter().all(|v| v.xyz[0] >= -0.5 && v.xyz[0] <= 0.));
    assert!(right.iter().all(|v| v.xyz[0] >= 0. && v.xyz[0] <= 0.5));
}

fn vertical_static_png_preserves_every_row_in_directory_zip_and_atlas() {
    let fixture = Fixture::new();
    let directory = fixture.path("vertical_static");
    let archive = fixture.path("vertical_static.zip");
    let pixels = [12, 34, 56, 255, 78, 90, 123, 128, 210, 220, 230, 0];
    let bytes = png(&pixels, 3);
    put(&directory, STONE, &bytes);
    {
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        writer
            .start_file(STONE, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&bytes).unwrap();
        writer.finish().unwrap();
    }
    for path in [directory, archive] {
        let bank = TextureBank::open_stack_java(&[path]).unwrap();
        let tile = bank.get("minecraft:stone").unwrap();
        assert_eq!((tile.width, tile.height), (1, 3));
        assert_eq!(tile.rgba, pixels);
        assert_eq!(tile.alpha_mode(), AlphaMode::Blend);
        assert_eq!(bank.animation_count(), 0);
        let slot = &bank.atlas().unwrap().entries["minecraft:stone"];
        for (row, expected) in pixels.chunks_exact(4).enumerate() {
            let uv = slot.sample(0.5, (row as f32 + 0.5) / 3.0);
            assert_eq!(
                bank.atlas_pixel(uv, slot.page).unwrap().as_slice(),
                expected
            );
        }
    }
}

fn vertical_static_overlay_replaces_animated_strip_without_inheriting_metadata() {
    let fixture = Fixture::new();
    let base = fixture.path("animated_base");
    let overlay = fixture.path("static_overlay");
    put(&base, STONE, &png(&[200, 0, 0, 255, 0, 200, 0, 255], 2));
    put(
        &base,
        &format!("{STONE}.mcmeta"),
        br#"{"animation":{"frames":[1,0]}}"#,
    );
    let pixels = [12, 34, 56, 255, 78, 90, 123, 128];
    put(&overlay, STONE, &png(&pixels, 2));
    let animated = TextureBank::open_stack_java(std::slice::from_ref(&base)).unwrap();
    assert_eq!(color(&animated, "minecraft:stone"), [0, 200, 0, 255]);
    assert_eq!(animated.animation_count(), 1);
    let bank = TextureBank::open_stack_java(&[base, overlay]).unwrap();
    let tile = bank.get("minecraft:stone").unwrap();
    assert_eq!((tile.width, tile.height), (1, 2));
    assert_eq!(tile.rgba, pixels);
    assert_eq!(bank.animation_count(), 0);
}

fn zip_overlay_preserves_lower_models_and_sprites_without_stale_animation() {
    let temp = Fixture::new();
    let base = temp.path("base");
    let overlay = temp.path("overlay.zip");
    put(&base, STONE, &png(&[200, 0, 0, 255, 0, 200, 0, 255], 2));
    put(
        &base,
        "assets/minecraft/textures/block/stone.png.mcmeta",
        br#"{"animation":{"frames":[1,0]}}"#,
    );
    put(
        &base,
        "assets/minecraft/textures/block/dirt.png",
        &png(&[120, 60, 20, 255], 1),
    );
    put(
        &base,
        "assets/minecraft/models/block/stone.json",
        br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/stone"}}"#,
    );
    let mut zip = zip::ZipWriter::new(File::create(&overlay).unwrap());
    zip.start_file(
        "assets/minecraft/textures/blocks/stone.png",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&png(&[0, 0, 250, 255], 1)).unwrap();
    zip.start_file(
        "assets/example/textures/block/casing.png",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&png(&[60, 70, 80, 255], 1)).unwrap();
    zip.finish().unwrap();
    let bank = TextureBank::open_stack(&[base.clone(), overlay.clone()]).unwrap();
    assert_eq!(color(&bank, "minecraft:stone"), [0, 0, 250, 255]);
    assert_eq!(color(&bank, "minecraft:dirt"), [120, 60, 20, 255]);
    assert_eq!(color(&bank, "example:casing"), [60, 70, 80, 255]);
    assert_eq!(bank.animation_count(), 0);
    assert_eq!(bank.model_count(), 1);
    assert!(bank
        .atlas()
        .unwrap()
        .entries
        .contains_key("minecraft:stone"));
    let reversed = TextureBank::open_stack(&[overlay, base]).unwrap();
    assert_eq!(color(&reversed, "minecraft:stone"), [0, 200, 0, 255]);
    assert_eq!(reversed.animation_count(), 1);
    assert!(TextureBank::open_stack(&[]).is_err());
    // Modpack layering no longer fails based on an arbitrary pack count.
    // Functional overlay precedence is already verified above.
}

fn metadata_only_overlay_changes_schedule_without_replacing_png() {
    let temp = Fixture::new();
    let base = temp.path("base");
    let meta = temp.path("meta");
    put(&base, STONE, &png(&[255, 0, 0, 255, 0, 255, 0, 255], 2));
    put(
        &base,
        "assets/minecraft/textures/block/stone.png.mcmeta",
        br#"{"animation":{"frames":[0,1]}}"#,
    );
    put(
        &meta,
        "assets/minecraft/textures/block/stone.png.mcmeta",
        br#"{"animation":{"frames":[1,0]}}"#,
    );
    let bank = TextureBank::open_stack(&[base.clone(), meta]).unwrap();
    assert_eq!(bank.animation_count(), 1);
    assert_eq!(color(&bank, "minecraft:stone"), [0, 255, 0, 255]);
    assert!(TextureBank::open_stack(&[base, temp.path("missing.zip")]).is_err());
}

fn corrupt_modern_png_does_not_shadow_valid_legacy_texture_in_either_zip_order() {
    for modern_first in [true, false] {
        let temp = Fixture::new();
        let pack = temp.path("aliases.zip");
        let modern = "assets/example/textures/block/gearbox.png";
        let legacy = "assets/example/textures/blocks/gearbox.png";
        let mut zip = zip::ZipWriter::new(File::create(&pack).unwrap());
        for name in if modern_first {
            [modern, legacy]
        } else {
            [legacy, modern]
        } {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            if name == modern {
                zip.write_all(b"invalid PNG: corrupt high-priority alias")
                    .unwrap();
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

fn valid_modern_png_wins_over_legacy_regardless_of_archive_order() {
    for modern_first in [true, false] {
        let temp = Fixture::new();
        let pack = temp.path("aliases.zip");
        let modern = "assets/example/textures/block/casing.png";
        let legacy = "assets/example/textures/blocks/casing.png";
        let mut zip = zip::ZipWriter::new(File::create(&pack).unwrap());
        for name in if modern_first {
            [modern, legacy]
        } else {
            [legacy, modern]
        } {
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

fn corrupt_upper_layer_sprite_preserves_lower_png_and_animation_metadata() {
    let temp = Fixture::new();
    let base = temp.path("base");
    let upper = temp.path("override.zip");
    put(&base, STONE, &png(&[255, 0, 0, 255, 0, 255, 0, 255], 2));
    put(
        &base,
        "assets/minecraft/textures/block/stone.png.mcmeta",
        br#"{"animation":{"frames":[1,0]}}"#,
    );
    let mut zip = zip::ZipWriter::new(File::create(&upper).unwrap());
    zip.start_file(STONE, zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"not a valid PNG").unwrap();
    zip.finish().unwrap();
    let bank = TextureBank::open_stack(&[base, upper]).unwrap();
    assert_eq!(color(&bank, "minecraft:stone"), [0, 255, 0, 255]);
    assert_eq!(bank.animation_count(), 1);
    assert!(bank.warnings.iter().any(|s| s.contains("minecraft:stone")));
}

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
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
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
    assert_eq!(
        dir_bank.atlas().unwrap().entries,
        zip_bank.atlas().unwrap().entries
    );
}

fn spectator_emits_real_extended_modded_model_vertices_with_valid_atlas_uv() {
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
    use std::sync::{Arc, Mutex};
    let temp = Fixture::new();
    let path = temp.path("mod_assets");
    put(
        &path,
        "assets/example/textures/item/large.png",
        &png(&[31, 102, 202, 255], 1),
    );
    put(
        &path,
        "assets/example/blockstates/large.json",
        br#"{"variants":{"":{"model":"example:item/large"}}}"#,
    );
    put(
        &path,
        "assets/example/models/item/large.json",
        br##"{"textures":{"all":"example:item/large"},"elements":[
        {"from":[-16,0,-8],"to":[32,16,24],"faces":{
          "up":{"texture":"#all","uv":[0,0,16,16]},
          "north":{"texture":"#all","uv":[0,0,16,16]}}}]}"##,
    );
    let bank = Arc::new(TextureBank::open(&path).unwrap());
    let state = State {
        name: "example:large".into(),
        properties: vec![],
    };
    let scene = crate::spectator::SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface: Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 64,
            y_max: 111,
            width: 16,
            depth: 16,
            voxels: vec![SurfaceVoxel::new(
                [0, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: state.into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )],
            source_tiles: vec![],
            source_chunks: 1,
            missing_columns: 0,
        }),
        texture_bank: Some(bank),
        show_textures: true,
        block_layers: Default::default(),
        camera: crate::spectator::Camera::default(),
        quality: crate::gpu_mesh::Quality::Balanced,
        cache: Arc::new(Mutex::new(None)),
        pending: Arc::new(Mutex::new(false)),
        presented: Default::default(),
        presented_view: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Arc::new(Mutex::new(
            crate::chunk_mesh_cache::ChunkMeshCache::default(),
        )),
        lighting: crate::lighting::Table::unlit(),
    };
    let mesh = crate::spectator::build(&scene);
    assert_eq!(mesh.len(), 12);
    let extents = |axis: usize| {
        (
            mesh.iter()
                .map(|v| v.xyz[axis])
                .fold(f32::INFINITY, f32::min),
            mesh.iter()
                .map(|v| v.xyz[axis])
                .fold(f32::NEG_INFINITY, f32::max),
        )
    };
    assert_eq!(extents(0), (-1.5, 1.5));
    assert_eq!(extents(1), (5.5, 6.5));
    assert_eq!(extents(2), (-1.0, 1.0));
    assert!(mesh
        .iter()
        .all(|v| v.uv.iter().all(|component| component.is_finite())));
}

fn generic_mod_model_sprites_resolve_in_both_zip_and_directory() {
    let fixture = Fixture::new();
    let directory = fixture.path("nested_model");
    let archive = fixture.path("nested_model.zip");
    let entries = [
        (
            "assets/create.addon/textures/item/machine/gizmo.png",
            png(&[22, 66, 204, 255], 1),
        ),
        (
            "assets/create.addon/textures/entity/machine/side.png",
            png(&[208, 32, 78, 255], 1),
        ),
        (
            "assets/create.addon/blockstates/gizmo.json",
            br#"{"variants":{"":{"model":"create.addon:item/machine/gizmo"}}}"#.to_vec(),
        ),
        (
            "assets/create.addon/models/item/machine/gizmo.json",
            br##"{"parent":"create.addon:custom/model/base","textures":{
                "all":"create.addon:item/machine/gizmo",
                "side":"create.addon:entity/machine/side"}}"##
                .to_vec(),
        ),
        (
            "assets/create.addon/models/custom/model/base.json",
            br##"{"elements":[{"from":[-16,0,-8],"to":[32,16,24],"faces":{
                "north":{"texture":"#side","uv":[0,0,16,16]},
                "up":{"texture":"#all","uv":[0,0,16,16]}}}]}"##
                .to_vec(),
        ),
    ];
    for (filename, contents) in &entries {
        put(&directory, filename, contents);
    }
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    for (filename, contents) in entries.iter().rev() {
        zip.start_file(*filename, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap();
    for pack in [directory, archive] {
        let bank = TextureBank::open(&pack).unwrap();
        assert_eq!(
            color(&bank, "create.addon:item/machine/gizmo"),
            [22, 66, 204, 255]
        );
        assert_eq!(
            color(&bank, "create.addon:entity/machine/side"),
            [208, 32, 78, 255]
        );
        let state = mcedit2_world::sections::State {
            name: "create.addon:gizmo".into(),
            properties: vec![],
        };
        let elements = bank.geometry_at(&state, (1, 64, 1)).unwrap();
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].bounds.min, [-1.5, -0.5, -1.0]);
        for face in [crate::isometric::Face::Top, crate::isometric::Face::NegZ] {
            assert!(bank
                .atlas_face_style_at(&state, face, 0, (1, 64, 1))
                .is_some());
        }
        assert!(bank
            .atlas()
            .unwrap()
            .entries
            .contains_key("create.addon:item/machine/gizmo"));
        assert!(bank
            .atlas()
            .unwrap()
            .entries
            .contains_key("create.addon:entity/machine/side"));
    }
}

fn original_json_repeating_uvs_emit_real_atlas_safe_spectator_faces() {
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
    use std::sync::{Arc, Mutex};
    let fx = Fixture::new();
    let path = fx.path("uv_pack");
    put(
        &path,
        "assets/example/textures/item/tiling.png",
        &png(&[105, 80, 230, 255], 1),
    );
    put(
        &path,
        "assets/example/blockstates/tile.json",
        br#"{"variants":{"":{"model":"example:block/tiling"}}}"#,
    );
    put(
        &path,
        "assets/example/models/block/tiling.json",
        br##"{
        "textures":{"all":"example:item/tiling"},
        "elements":[{"from":[0,0,0],"to":[16,16,16],
        "faces":{"up":{"texture":"#all","uv":[-16,0,32,32]}}}]
    }"##,
    );
    let bank = Arc::new(TextureBank::open(&path).unwrap());
    let state = State {
        name: "example:tile".into(),
        properties: vec![],
    };
    assert!(bank
        .atlas_repeating_face_at(&state, crate::isometric::Face::Top, 0, (0, 70, 0))
        .is_some());
    let slot = bank
        .atlas()
        .unwrap()
        .entries
        .get("example:item/tiling")
        .unwrap()
        .clone();
    let scene = crate::spectator::SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface: Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 64,
            y_max: 111,
            width: 16,
            depth: 16,
            voxels: vec![SurfaceVoxel::new(
                [0, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: state.into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )],
            source_tiles: vec![],
            source_chunks: 1,
            missing_columns: 0,
        }),
        texture_bank: Some(bank),
        show_textures: true,
        block_layers: Default::default(),
        camera: crate::spectator::Camera::default(),
        quality: crate::gpu_mesh::Quality::Balanced,
        cache: Arc::new(Mutex::new(None)),
        pending: Arc::new(Mutex::new(false)),
        presented: Default::default(),
        presented_view: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Arc::new(Mutex::new(
            crate::chunk_mesh_cache::ChunkMeshCache::default(),
        )),
        lighting: crate::lighting::Table::unlit(),
    };
    let mesh = crate::spectator::build(&scene);
    assert_eq!(mesh.len(), 36);
    assert!(mesh
        .iter()
        .all(|v| v.uv[0] >= slot.start[0] && v.uv[0] <= slot.end[0]));
    assert!(mesh
        .iter()
        .all(|v| v.uv[1] >= slot.start[1] && v.uv[1] <= slot.end[1]));
    assert!(mesh.iter().all(|v| v.atlas_page == slot.page));
}

fn declared_obj_loader_emits_generic_static_mesh_via_gpu_path() {
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
    use std::sync::{Arc, Mutex};
    let fixture = Fixture::new();
    let pack = fixture.path("obj_pack");
    put(
        &pack,
        "assets/example/textures/item/casing.png",
        &png(&[3, 127, 241, 255], 1),
    );
    put(
        &pack,
        "assets/example/blockstates/widget.json",
        br#"{"variants":{"":{"model":"example:block/widget"}}}"#,
    );
    put(
        &pack,
        "assets/example/models/block/widget.json",
        br#"{"loader":"neoforge:obj","model":"example:models/block/widget.obj",
            "textures":{"casing":"example:item/casing"}}"#,
    );
    put(&pack, "assets/example/models/block/widget.obj",
        b"mtllib widget.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl casing\nf 1/1 2/2 3/3 4/4\n");
    put(
        &pack,
        "assets/example/models/block/widget.mtl",
        b"newmtl casing\nmap_Kd #casing\n",
    );
    let bank = Arc::new(TextureBank::open(&pack).unwrap());
    let state = State {
        name: "example:widget".into(),
        properties: vec![],
    };
    let mesh = bank.static_obj_at(&state, (0, 70, 0)).unwrap();
    assert_eq!(mesh.len(), 2);
    assert_eq!(mesh[0].xyz[0], [-0.5, -0.5, -0.5]);
    assert_eq!(mesh[1].xyz[2], [-0.5, 0.5, -0.5]);
    let scene = crate::spectator::SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface: Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 64,
            y_max: 111,
            width: 16,
            depth: 16,
            voxels: vec![SurfaceVoxel::new(
                [0, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: state.into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )],
            missing_columns: 0,
            source_chunks: 1,
            source_tiles: vec![],
        }),
        texture_bank: Some(bank),
        show_textures: true,
        block_layers: Default::default(),
        camera: crate::spectator::Camera::default(),
        quality: crate::gpu_mesh::Quality::Balanced,
        cache: Arc::new(Mutex::new(None)),
        pending: Arc::new(Mutex::new(false)),
        presented: Default::default(),
        presented_view: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Arc::new(Mutex::new(
            crate::chunk_mesh_cache::ChunkMeshCache::default(),
        )),
        lighting: crate::lighting::Table::unlit(),
    };
    let vertices = crate::spectator::build(&scene);
    assert_eq!(vertices.len(), 12);
    assert!(vertices.iter().all(|v| v.atlas_page == mesh[0].page));
    assert!(vertices.iter().all(|v| v.uv.iter().all(|x| x.is_finite())));
}

fn obj_mtl_and_gltf_renderers_follow_resource_model_without_block_id_tables() {
    use mcedit2_world::sections::State;
    let fixture = Fixture::new();
    let path = fixture.path("special_renderers");
    put(
        &path,
        "assets/example/textures/item/casing.png",
        &png(&[40, 80, 120, 255], 1),
    );
    put(
        &path,
        "assets/example/blockstates/casing.json",
        br#"{"variants":{"":{"model":"example:block/casing"}}}"#,
    );
    put(
        &path,
        "assets/example/models/block/casing.json",
        br#"{"loader":"neoforge:obj","model":"example:models/block/casing.obj"}"#,
    );
    put(
        &path,
        "assets/example/models/block/casing.obj",
        b"mtllib casing.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 3 0\nvt 0 2\nusemtl shell\nf 1/1 2/2 3/3\n",
    );
    put(
        &path,
        "assets/example/models/block/casing.mtl",
        b"newmtl shell\nKd 0.25 0.5 1.0 0.5\nd 0.8\nmap_Kd example:item/casing\n",
    );
    let bank = TextureBank::open(&path).unwrap();
    let state = State {
        name: "example:casing".into(),
        properties: vec![],
    };
    let mesh = bank.static_obj_at(&state, (0, 70, 0)).unwrap();
    assert!(mesh.len() > 1, "UV repetition must split OBJ triangles");
    assert!(mesh.iter().all(|m| m.rgba == [[0.25, 0.5, 1.0, 0.5]; 3]));
    let sprite = bank
        .atlas()
        .unwrap()
        .entries
        .get("example:item/casing")
        .unwrap();
    assert!(mesh
        .iter()
        .all(|m| m.uv.iter().all(|uv| uv[0] >= sprite.start[0] - 1e-6
            && uv[0] <= sprite.end[0] + 1e-6
            && uv[1] >= sprite.start[1] - 1e-6
            && uv[1] <= sprite.end[1] + 1e-6)));

    // Replace the blockstate's model reference with a glTF/GLB backend.
    let path2 = fixture.path("gltf_renderers");
    put(
        &path2,
        "assets/example/textures/item/casing.png",
        &png(&[40, 80, 120, 255], 1),
    );
    put(
        &path2,
        "assets/example/blockstates/casing.json",
        br#"{"variants":{"":{"model":"example:block/casing"}}}"#,
    );
    put(&path2,"assets/example/models/block/casing.json",
        br#"{"loader":"mcedit2:glb","model":"example:models/block/casing.glb","textures":{"particle":"example:item/casing"}}"#);
    let mut bin = Vec::new();
    for v in [0f32, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1.] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    let doc = serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":60}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},
            {"buffer":0,"byteOffset":36,"byteLength":24}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1}}]}]});
    let mut json = serde_json::to_vec(&doc).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ')
    }
    let size = (12 + 8 + json.len() + 8 + bin.len()) as u32;
    let mut glb = Vec::new();
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&size.to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");
    glb.extend_from_slice(&bin);
    put(&path2, "assets/example/models/block/casing.glb", &glb);
    let bank = TextureBank::open(&path2).unwrap();
    assert_eq!(bank.static_obj_at(&state, (0, 70, 0)).unwrap().len(), 1);
}

fn custom_mod_loader_static_renderer_adapter_is_declarative_and_id_independent() {
    use mcedit2_world::sections::State;
    let fixture = Fixture::new();
    let root = fixture.path("mod_adapter");
    put(
        &root,
        "assets/foreign.mod/textures/item/skin.png",
        &png(&[25, 75, 150, 255], 1),
    );
    put(
        &root,
        "assets/foreign.mod/blockstates/sculpture.json",
        br#"{"variants":{"":{"model":"foreign.mod:custom/sculpture"}}}"#,
    );
    put(
        &root,
        "assets/foreign.mod/models/custom/sculpture.json",
        br#"{"loader":"foreign.mod:procedural_renderer",
            "mcedit2:static_renderer":{"format":"obj",
              "model":"foreign.mod:models/custom/sculpture.obj",
              "textures":{"shell":"foreign.mod:item/skin"}}}"#,
    );
    put(
        &root,
        "assets/foreign.mod/models/custom/sculpture.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl shell\nf 1/1 2/2 3/3\n",
    );
    let bank = TextureBank::open(&root).unwrap();
    let state = State {
        name: "foreign.mod:sculpture".into(),
        properties: vec![],
    };
    assert_eq!(bank.static_obj_at(&state, (0, 64, 0)).unwrap().len(), 1);
    let no_adapter_root = fixture.path("no_adapter");
    put(
        &no_adapter_root,
        "assets/foreign.mod/blockstates/sculpture.json",
        br#"{"variants":{"":{"model":"foreign.mod:custom/sculpture"}}}"#,
    );
    put(
        &no_adapter_root,
        "assets/foreign.mod/models/custom/sculpture.json",
        br#"{"loader":"foreign.mod:procedural_renderer"}"#,
    );
    assert!(
        TextureBank::open(&no_adapter_root).is_err()
            || TextureBank::open(&no_adapter_root)
                .unwrap()
                .static_obj_at(&state, (0, 64, 0))
                .is_none()
    );
}

fn nested_gltf_scene_uses_external_buffers_in_unordered_zip_pack() {
    let temp = Fixture::new();
    let archive = temp.path("gltf_buffers.zip");
    let mut positions = Vec::new();
    for n in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
        positions.extend_from_slice(&n.to_le_bytes());
    }
    let mut tex = Vec::new();
    for n in [0f32, 0., 1., 0., 0., 1.] {
        tex.extend_from_slice(&n.to_le_bytes());
    }
    let doc = serde_json::json!({
        "asset":{"version":"2.0"},
        "buffers":[{"uri":"position.bin","byteLength":36},{"uri":"uv.bin","byteLength":24}],
        "bufferViews":[{"buffer":0,"byteLength":36},{"buffer":1,"byteLength":24}],
        "accessors":[
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1}}]}],
        "nodes":[{"translation":[0.25,0,0],"children":[1]},{"mesh":0,"translation":[0,0.5,0]}],
        "scenes":[{"nodes":[0]}],"scene":0
    });
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    for (name, data) in [
        (
            "assets/example/models/block/assembly.gltf",
            serde_json::to_vec(&doc).unwrap(),
        ),
        ("assets/example/models/block/uv.bin", tex),
        ("assets/example/models/block/position.bin", positions),
        (
            "assets/example/blockstates/assembly.json",
            br#"{"variants":{"":{"model":"example:block/assembly"}}}"#.to_vec(),
        ),
        (
            "assets/example/models/block/assembly.json",
            br#"{"loader":"mcedit2:gltf","model":"example:models/block/assembly.gltf",
                "textures":{"particle":"example:item/assembly"}}"#
                .to_vec(),
        ),
        (
            "assets/example/textures/item/assembly.png",
            png(&[95, 175, 241, 255], 1),
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&data).unwrap();
    }
    zip.finish().unwrap();
    let bank = TextureBank::open(&archive).unwrap();
    let state = mcedit2_world::sections::State {
        name: "example:assembly".into(),
        properties: vec![],
    };
    let selected = bank.static_obj_at(&state, (0, 70, 0)).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].xyz[0], [-0.25, 0., -0.5]);
    assert_eq!(selected[0].xyz[1], [0.75, 0., -0.5]);
    assert!(!bank.warnings.iter().any(|x| x.contains("glTF")));
}

fn obj_material_map_d_is_baked_into_actual_atlas_sprite_alpha() {
    let f = Fixture::new();
    let dir = f.path("material_map");
    put(
        &dir,
        "assets/example/textures/item/base.png",
        &png(&[125, 180, 230, 255, 10, 50, 90, 128], 2),
    );
    put(
        &dir,
        "assets/example/textures/item/mask.png",
        &png(&[0, 0, 0, 255, 255, 255, 255, 255], 2),
    );
    put(
        &dir,
        "assets/example/blockstates/widget.json",
        br#"{"variants":{"":{"model":"example:block/assembly"}}}"#,
    );
    put(
        &dir,
        "assets/example/models/block/assembly.json",
        br#"{"loader":"example:static_adapter","mcedit2:static_renderer":{"format":"obj","model":"example:models/block/assembly.obj"},
            "textures":{"particle":"example:item/base"}}"#,
    );
    put(
        &dir,
        "assets/example/models/block/assembly.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl casing\nf 1/1 2/2 3/3\n",
    );
    put(
        &dir,
        "assets/example/models/block/assembly.mtl",
        b"newmtl casing\nKd 1 1 1\nmap_Kd example:item/base\nmap_d example:item/mask\n",
    );
    let bank = TextureBank::open(&dir).unwrap();
    let source = mcedit2_world::sections::State {
        name: "example:widget".into(),
        properties: vec![],
    };
    let mesh = bank.models.obj_at(&source, (0, 64, 0)).unwrap();
    let composed = bank.get(&mesh[0].texture).unwrap();
    assert_eq!(composed.pixel(0.5, 0.25), [125, 180, 230, 0]);
    assert_eq!(composed.pixel(0.5, 0.75), [10, 50, 90, 128]);
    let vertices = bank.static_obj_at(&source, (0, 64, 0)).unwrap();
    assert_eq!(vertices.len(), 1);
    assert_eq!(
        crate::material_mode::kind(vertices[0].mode),
        AlphaMode::Blend as u32
    );
}

fn fullbright_gltf_alpha_rules_flow_from_pack_material_into_gpu_mesh_mode() {
    let fixture = Fixture::new();
    for (index, alpha_mode, expected_kind) in [
        (0, "OPAQUE", 0_u32),
        (1, "MASK", 1_u32),
        (2, "BLEND", 2_u32),
    ] {
        let root = fixture.path(&format!("fullbright_gltf_{index}"));
        let mut positions = Vec::new();
        for n in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.] {
            positions.extend_from_slice(&n.to_le_bytes());
        }
        let mut tex = Vec::new();
        for n in [0f32, 0., 1., 0., 0., 1.] {
            tex.extend_from_slice(&n.to_le_bytes());
        }
        let gltf = serde_json::json!({
            "asset": {"version":"2.0"},
            "buffers": [
                {"uri":"positions.bin","byteLength":36},
                {"uri":"uv.bin","byteLength":24}
            ],
            "bufferViews":[{"buffer":0,"byteLength":36},{"buffer":1,"byteLength":24}],
            "accessors":[
                {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
                {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}
            ],
            "materials":[{"name":"panel","alphaMode":alpha_mode,
                "alphaCutoff":0.75,"doubleSided":false,
                "pbrMetallicRoughness":{"baseColorFactor":[0.2,0.4,0.6,0.25]}
            }],
            "meshes":[{"primitives":[{"attributes":
                {"POSITION":0,"TEXCOORD_0":1},"material":0}]}]
        });
        put(
            &root,
            "assets/example/models/block/panel.gltf",
            &serde_json::to_vec(&gltf).unwrap(),
        );
        put(
            &root,
            "assets/example/models/block/positions.bin",
            &positions,
        );
        put(&root, "assets/example/models/block/uv.bin", &tex);
        put(
            &root,
            "assets/example/textures/item/panel.png",
            &png(&[100, 170, 200, 80], 1),
        );
        put(
            &root,
            "assets/example/blockstates/panel.json",
            br#"{"variants":{"":{"model":"example:block/panel"}}}"#,
        );
        put(
            &root,
            "assets/example/models/block/panel.json",
            br#"{"loader":"mcedit2:gltf","model":"example:models/block/panel.gltf",
                "textures":{"panel":"example:item/panel"}}"#,
        );
        let bank = TextureBank::open(&root).unwrap();
        let state = mcedit2_world::sections::State {
            name: "example:panel".into(),
            properties: vec![],
        };
        let faces = bank.static_obj_at(&state, (0, 70, 0)).unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].mode & 3, expected_kind);
        assert_ne!(
            faces[0].mode & 4,
            0,
            "Single-sided glTF material must discard backfaces"
        );
        assert_eq!(faces[0].rgba, [[0.2, 0.4, 0.6, 0.25]; 3]);
        match expected_kind {
            0 => assert_ne!(faces[0].mode & 8, 0),
            1 => {
                assert_ne!(faces[0].mode & 16, 0);
                assert_eq!((faces[0].mode >> 8) & 255, 191);
            }
            _ => assert_eq!(faces[0].mode & 8, 0),
        }
    }
}

fn gltf_base_color_texture_autobinds_and_spectator_instances_are_distinct() {
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
    use std::sync::{Arc, Mutex};
    let fixture = Fixture::new();
    let root = fixture.path("gltf_auto_texture");
    let mut binary = Vec::new();
    for value in [0f32, 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 1.] {
        binary.extend_from_slice(&value.to_le_bytes());
    }
    let doc = serde_json::json!({
        "asset":{"version":"2.0"},
        "buffers":[{"uri":"geometry.bin","byteLength":60}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},
                       {"buffer":0,"byteOffset":36,"byteLength":24}],
        "accessors":[
           {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
           {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
        // Real glTF/GLB material image references, no Java-side 'textures'.
        "images":[{"uri":"panel.png"}],
        "textures":[{"source":0}],
        "materials":[{"pbrMetallicRoughness":{
            "baseColorTexture":{"index":0},
            "baseColorFactor":[0.75,0.5,0.25,1.0]
        }}],
        "meshes":[{"primitives":[{"attributes":{
            "POSITION":0,"TEXCOORD_0":1
        },"material":0}]}],
        "nodes":[{"mesh":0,"translation":[0.,0.,0.]},
                 {"mesh":0,"translation":[0.5,0.,0.]}],
        "scenes":[{"nodes":[0,1]}],"scene":0
    });
    put(
        &root,
        "assets/example/models/block/assembly.gltf",
        &serde_json::to_vec(&doc).unwrap(),
    );
    put(&root, "assets/example/models/block/geometry.bin", &binary);
    put(
        &root,
        "assets/example/models/block/panel.png",
        &png(&[20, 130, 220, 255], 1),
    );
    put(
        &root,
        "assets/example/blockstates/assembly.json",
        br#"{"variants":{"":{"model":"example:block/assembly"}}}"#,
    );
    put(
        &root,
        "assets/example/models/block/assembly.json",
        br#"{"loader":"mcedit2:gltf","model":"example:models/block/assembly.gltf"}"#,
    );
    let bank = Arc::new(TextureBank::open(&root).unwrap());
    let state = State {
        name: "example:assembly".into(),
        properties: vec![],
    };
    assert_eq!(
        color(&bank, "example:__gltf/block/panel"),
        [20, 130, 220, 255]
    );
    let selected = bank.models.obj_at(&state, (0, 70, 0)).unwrap();
    assert_eq!(selected.len(), 2);
    assert!(selected
        .iter()
        .all(|t| t.texture == "example:__gltf/block/panel"));
    assert_ne!(selected[0].xyz[0], selected[1].xyz[0]);
    let surfaces = bank.static_obj_at(&state, (0, 70, 0)).unwrap();
    assert_eq!(surfaces.len(), 2);
    let page = bank.atlas().unwrap().entries["example:__gltf/block/panel"].page;
    assert!(surfaces.iter().all(|tri| tri.page == page));
    let scene = crate::spectator::SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface: Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 64,
            y_max: 111,
            width: 16,
            depth: 16,
            voxels: vec![SurfaceVoxel::new(
                [0, 70, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: state.into(),
                    biome: None,
                    staged: false,
                    original_block_entity: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )],
            source_tiles: vec![],
            source_chunks: 1,
            missing_columns: 0,
        }),
        texture_bank: Some(bank),
        show_textures: true,
        block_layers: Default::default(),
        camera: crate::spectator::Camera::default(),
        quality: crate::gpu_mesh::Quality::Balanced,
        cache: Arc::new(Mutex::new(None)),
        pending: Arc::new(Mutex::new(false)),
        presented: Default::default(),
        presented_view: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Arc::new(Mutex::new(
            crate::chunk_mesh_cache::ChunkMeshCache::default(),
        )),
        lighting: crate::lighting::Table::unlit(),
    };
    let vertices = crate::spectator::build(&scene);
    assert_eq!(vertices.len(), 12);
    assert!(vertices.iter().all(|v| v.atlas_page == page));
    assert!(vertices.iter().all(|v| v.uv.iter().all(|x| x.is_finite())));
}

fn core_java_resource_reader_does_not_import_custom_mod_renderers() {
    let fixture = Fixture::new();
    let directory = fixture.path("standard_only");
    let zip_path = fixture.path("standard_only.zip");
    let inputs: Vec<(&str, Vec<u8>)> = vec![
        (
            "assets/example/textures/block/casing.png",
            png(&[12, 110, 208, 255], 1),
        ),
        (
            "assets/example/textures/block/unresolved.png",
            png(&[220, 30, 11, 255], 1),
        ),
        (
            "assets/example/blockstates/casing.json",
            br#"{"variants":{"":{"model":"example:block/casing"}}}"#.to_vec(),
        ),
        (
            "assets/example/models/block/casing.json",
            br##"{"parent":"minecraft:block/cube_all","textures":{"all":"example:block/casing"}}"##
                .to_vec(),
        ),
        (
            "assets/example/blockstates/custom.json",
            br#"{"variants":{"":{"model":"example:block/custom"}}}"#.to_vec(),
        ),
        (
            "assets/example/models/block/custom.json",
            br##"{"loader":"example:unknown","model":"example:models/block/custom.obj",
                "parent":"minecraft:block/cube_all","textures":{"all":"example:block/casing"}}"##
                .to_vec(),
        ),
        (
            "assets/example/models/block/custom.obj",
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec(),
        ),
        (
            "assets/example/models/block/custom.mtl",
            b"newmtl custom\n".to_vec(),
        ),
        (
            "assets/example/models/block/custom.gltf",
            br#"{"asset":{"version":"2.0"}}"#.to_vec(),
        ),
        (
            "assets/example/models/block/custom.bin",
            b"not a vanilla resource".to_vec(),
        ),
        (
            "assets/example/models/block/sidecar.png",
            png(&[200, 200, 0, 255], 1),
        ),
    ];
    for (name, bytes) in &inputs {
        put(&directory, name, bytes);
    }
    {
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        for (name, bytes) in inputs.iter().rev() {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
    for source in [&directory, &zip_path] {
        let bank = TextureBank::open_stack_java(&[source.to_path_buf()]).unwrap();
        let [casing, custom, unresolved] =
            ["example:casing", "example:custom", "example:unresolved"].map(|name| State {
                name: name.into(),
                properties: vec![],
            });
        assert_eq!(bank.model_count(), 4);
        assert_eq!(color(&bank, "example:casing"), [12, 110, 208, 255]);
        // Sidecar images under models/ are only used by non-vanilla formats.
        assert!(!bank
            .atlas()
            .unwrap()
            .entries
            .contains_key("example:__gltf/block/sidecar"));
        let (uv, _, page) = bank
            .atlas_face_style_at(&casing, crate::isometric::Face::Top, 0, (0, 64, 0))
            .unwrap();
        assert_eq!(
            bank.atlas_pixel(
                [(uv[0][0] + uv[2][0]) * 0.5, (uv[0][1] + uv[2][1]) * 0.5,],
                page
            ),
            Some([12, 110, 208, 255])
        );
        assert!(bank.geometry_at(&casing, (0, 64, 0)).is_some());
        assert_eq!(
            bank.geometry_at(&custom, (0, 64, 0)).unwrap()[0].bounds,
            crate::models::Box3::FULL
        );
        assert!(bank.static_obj_at(&custom, (0, 64, 0)).is_none());
        for missing in [&custom, &unresolved] {
            assert!(bank
                .resolve_id_at(missing, crate::isometric::Face::Top, (0, 64, 0))
                .is_none());
            let (uv, _, page) = bank
                .atlas_face_style_at(missing, crate::isometric::Face::Top, 0, (0, 64, 0))
                .unwrap();
            let pixel = bank.atlas_pixel(
                [(uv[0][0] + uv[2][0]) * 0.5, (uv[0][1] + uv[2][1]) * 0.5],
                page,
            );
            assert!(matches!(pixel, Some([248, 0, 248, 255] | [0, 0, 0, 255])));
        }
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Materials {
        native_animation_and_resource_boundaries_keep_pixels_and_source_identity();
        numeric_biome_profiles_follow_real_resource_overlays_and_structure_preview();
        java_composite_children_keep_texture_uv_geometry_and_declared_alpha();
        vertical_static_png_preserves_every_row_in_directory_zip_and_atlas();
        vertical_static_overlay_replaces_animated_strip_without_inheriting_metadata();
        zip_overlay_preserves_lower_models_and_sprites_without_stale_animation();
        metadata_only_overlay_changes_schedule_without_replacing_png();
        corrupt_modern_png_does_not_shadow_valid_legacy_texture_in_either_zip_order();
        valid_modern_png_wins_over_legacy_regardless_of_archive_order();
        corrupt_upper_layer_sprite_preserves_lower_png_and_animation_metadata();
        zip_and_directory_use_the_same_asset_classifier_and_overlay_rules();
        spectator_emits_real_extended_modded_model_vertices_with_valid_atlas_uv();
        generic_mod_model_sprites_resolve_in_both_zip_and_directory();
        original_json_repeating_uvs_emit_real_atlas_safe_spectator_faces();
        declared_obj_loader_emits_generic_static_mesh_via_gpu_path();
        obj_mtl_and_gltf_renderers_follow_resource_model_without_block_id_tables();
        custom_mod_loader_static_renderer_adapter_is_declarative_and_id_independent();
        nested_gltf_scene_uses_external_buffers_in_unordered_zip_pack();
        obj_material_map_d_is_baked_into_actual_atlas_sprite_alpha();
        fullbright_gltf_alpha_rules_flow_from_pack_material_into_gpu_mesh_mode();
        gltf_base_color_texture_autobinds_and_spectator_instances_are_distinct();
        core_java_resource_reader_does_not_import_custom_mod_renderers();
    }
}

fn numeric_biome_profiles_follow_real_resource_overlays_and_structure_preview() {
    use mcedit2_world::{
        clipboard::CaptureOptions,
        editing::{BiomeTarget, Bounds},
    };
    let fixture = Fixture::new();
    let base = fixture.path("1.8.jar");
    let definition = serde_json::json!({
        "format":1,"source":"显式颜色验证表","versions":{"minimum":0,"maximum":1343,"unversioned":true},"blend_radius":0,
        "entries":[
            {"id":1,"name":"红色草","temperature":0.8,"rainfall":0.4,"water_color":255,"grass_color":16711680,"foliage_color":65280},
            {"id":2,"name":"绿色草","temperature":2.0,"rainfall":0.0,"water_color":16711680,"grass_color":65280,"foliage_color":255}
        ]
    });
    let mut zip = zip::ZipWriter::new(File::create(&base).unwrap());
    let profile = serde_json::to_vec(&definition).unwrap();
    let image = png(&[255, 255, 255, 255], 1);
    let blockstates = br#"{"variants":{"snowy=false":{"model":"minecraft:block/test_grass"}}}"#;
    let mut faces = serde_json::Map::new();
    for side in ["up", "down", "east", "west", "north", "south"] {
        faces.insert(
            side.into(),
            serde_json::json!({"texture":"#all","tintindex":0}),
        );
    }
    let model=serde_json::to_vec(&serde_json::json!({"textures":{"all":"minecraft:block/test_grass"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":faces}]})).unwrap();
    for (name, bytes) in [
        (numeric_biome_colors::ASSET, profile.as_slice()),
        (
            "assets/minecraft/blockstates/grass.json",
            blockstates.as_slice(),
        ),
        (
            "assets/minecraft/models/block/test_grass.json",
            model.as_slice(),
        ),
        (
            "assets/minecraft/textures/block/test_grass.png",
            image.as_slice(),
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
    let vanilla = fixture.path("minecraft-1.8-client.jar");
    let mut archive = zip::ZipWriter::new(File::create(&vanilla).unwrap());
    let mut color_image = Vec::new();
    image::codecs::png::PngEncoder::new(&mut color_image)
        .write_image(
            &[31, 61, 91, 255].repeat(256 * 256),
            256,
            256,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    for (name, bytes) in [
        (
            "assets/minecraft/blockstates/grass.json",
            blockstates.as_slice(),
        ),
        (
            "assets/minecraft/models/block/test_grass.json",
            model.as_slice(),
        ),
        (
            "assets/minecraft/textures/block/test_grass.png",
            image.as_slice(),
        ),
        (
            "assets/minecraft/textures/colormap/grass.png",
            color_image.as_slice(),
        ),
    ] {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(bytes).unwrap();
    }
    archive.finish().unwrap();
    let builtin = TextureBank::open_stack_java_controlled(
        std::slice::from_ref(&vanilla),
        &LoadControl::default(),
    )
    .unwrap();
    assert_eq!(builtin.numeric_biome_choices().unwrap().1.len(), 61);
    let volume = mcedit2_world::numeric_biome::Volume {
        size: [1, 1, 1],
        cells: vec![2],
        data_version: None,
    };
    let source = mcedit2_world::biome_source::Region::new(None)
        .with_numeric_biomes([0, 64, 0], &volume)
        .unwrap();
    assert_eq!(
        builtin.tint_context(&source).block(
            &mcedit2_world::editing::parse_state("numeric:2[data=0]").unwrap(),
            0,
            None,
            (0, 64, 0)
        ),
        [31. / 255., 61. / 255., 91. / 255.]
    );
    let bank = TextureBank::open_stack_java_controlled(
        std::slice::from_ref(&base),
        &LoadControl::default(),
    )
    .unwrap();
    let (source, choices) = bank.numeric_biome_choices().unwrap();
    assert_eq!(source, "显式颜色验证表");
    assert_eq!(choices.len(), 2);
    let upper = fixture.path("upper");
    let mut replacement = definition.clone();
    replacement["source"] = "高层整份替换".into();
    replacement["entries"] = serde_json::json!([
        {"id":70000,"name":"自定义数字","temperature":0.1,"rainfall":0.2,"water_color":1193046,"grass_color":6636321,"foliage_color":11259375}
    ]);
    put(
        &upper,
        numeric_biome_colors::ASSET,
        &serde_json::to_vec(&replacement).unwrap(),
    );
    let top = TextureBank::open_stack_java_controlled(
        &[base.clone(), upper.clone()],
        &LoadControl::default(),
    )
    .unwrap();
    assert_eq!(top.numeric_biome_choices().unwrap().1[0].id, 70000);
    assert_eq!(top.numeric_biome_choices().unwrap().1.len(), 1);
    let generic =
        TextureBank::open_stack_with_policy(&[base.clone(), upper.clone()], false).unwrap();
    assert_eq!(generic.numeric_biome_choices().unwrap().0, "高层整份替换");
    put(&upper, numeric_biome_colors::ASSET, b"{broken");
    assert!(TextureBank::open_stack_java_controlled(
        &[base.clone(), upper.clone()],
        &LoadControl::default()
    )
    .is_err());
    assert!(TextureBank::open_stack_with_policy(&[base.clone(), upper], false).is_err());
    // 使用真实旧世界、实际暂存和捕获；比较独立预览实际发出的面颜色。
    let (mut app, mut scene, _, request) = crate::picking_integration_tests::fixture();
    let [x, y, z] = request.hit.owner.as_ref().unwrap().position;
    let key = app.current_key().unwrap();
    let world = &app.worlds[key.0];
    let raw = mcedit2_world::block_lookup::read_world_chunk_bytes(
        world,
        &world.dimensions[key.1].name,
        x.div_euclid(16),
        z.div_euclid(16),
    )
    .unwrap()
    .unwrap();
    let session = app.current_session(key).unwrap();
    let bounds = Bounds::new((x, y, z), (x, y, z)).unwrap();
    session
        .commit(
            session
                .prepare(
                    bounds,
                    &mcedit2_world::editing::parse_state("numeric:2[data=0]").unwrap(),
                    None,
                )
                .unwrap(),
        )
        .unwrap();
    // 样本本来就是 1 时，先设置 2 再设置 1，以保留明确的真实事务前后值。
    let original_biome = mcedit2_world::sections::decode_sections(&raw)
        .unwrap()
        .numeric_biome(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
        .unwrap();
    if original_biome == 1 {
        let p = session
            .prepare_biome_brush_cancellable(
                &[bounds],
                mcedit2_world::editing::BrushMask {
                    shape: None,
                    hollow: false,
                },
                &BiomeTarget::Numeric(2),
                || false,
            )
            .unwrap();
        session.commit_structure(p.into_structure()).unwrap();
    }
    let p = session
        .prepare_biome_brush_cancellable(
            &[bounds],
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false,
            },
            &BiomeTarget::Numeric(1),
            || false,
        )
        .unwrap();
    session.commit_structure(p.into_structure()).unwrap();
    scene.texture_bank = Some(Arc::new(bank));
    scene.show_textures = true;
    let capture = CaptureOptions {
        entities: false,
        biomes: true,
    };
    let clip = session
        .capture_structure_cancellable(bounds, capture, || false)
        .unwrap();
    let frame = crate::pending_import::build(
        session,
        &clip,
        [100, 64, 100],
        &scene,
        false,
        crate::language::Language::Chinese,
        &|| false,
    )
    .unwrap();
    let quads = frame
        .mesh
        .chunks
        .iter()
        .flat_map(|c| c.quads().iter())
        .collect::<Vec<_>>();
    assert!(!quads.is_empty());
    assert!(quads
        .iter()
        .flat_map(|q| q.rgba)
        .all(|p| p[0] > 0. && p[1] == 0. && p[2] == 0.));
    let p = session
        .prepare_biome_brush_cancellable(
            &[bounds],
            mcedit2_world::editing::BrushMask {
                shape: None,
                hollow: false,
            },
            &BiomeTarget::Numeric(2),
            || false,
        )
        .unwrap();
    session.commit_structure(p.into_structure()).unwrap();
    let clip = session
        .capture_structure_cancellable(bounds, capture, || false)
        .unwrap();
    let frame = crate::pending_import::build(
        session,
        &clip,
        [100, 64, 100],
        &scene,
        false,
        crate::language::Language::Chinese,
        &|| false,
    )
    .unwrap();
    assert!(frame
        .mesh
        .chunks
        .iter()
        .flat_map(|c| c.quads().iter())
        .flat_map(|q| q.rgba)
        .all(|p| p[0] == 0. && p[1] > 0. && p[2] == 0.));
    session.undo().unwrap();
    let clip = session
        .capture_structure_cancellable(bounds, capture, || false)
        .unwrap();
    let frame = crate::pending_import::build(
        session,
        &clip,
        [100, 64, 100],
        &scene,
        false,
        crate::language::Language::Chinese,
        &|| false,
    )
    .unwrap();
    assert!(frame
        .mesh
        .chunks
        .iter()
        .flat_map(|c| c.quads().iter())
        .flat_map(|q| q.rgba)
        .all(|p| p[0] > 0. && p[1] == 0. && p[2] == 0.));
    let world = &app.worlds[key.0];
    assert_eq!(
        mcedit2_world::block_lookup::read_world_chunk_bytes(
            world,
            &world.dimensions[key.1].name,
            x.div_euclid(16),
            z.div_euclid(16)
        )
        .unwrap()
        .unwrap(),
        raw
    );
}
