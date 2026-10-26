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
    assert!(TextureBank::open_stack(&vec![temp.path("base"); 1025]).is_err());
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

#[test]
fn spectator_emits_real_extended_modded_model_vertices_with_valid_atlas_uv() {
    use std::sync::{Arc, Mutex};
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
    let temp = Fixture::new();
    let path = temp.path("mod_assets");
    put(&path,"assets/example/textures/item/large.png",&png(&[31,102,202,255],1));
    put(&path,"assets/example/blockstates/large.json",
        br#"{"variants":{"":{"model":"example:item/large"}}}"#);
    put(&path,"assets/example/models/item/large.json",
        br##"{"textures":{"all":"example:item/large"},"elements":[
        {"from":[-16,0,-8],"to":[32,16,24],"faces":{
          "up":{"texture":"#all","uv":[0,0,16,16]},
          "north":{"texture":"#all","uv":[0,0,16,16]}}}]}"##);
    let bank = Arc::new(TextureBank::open(&path).unwrap());
    let state = State { name:"example:large".into(), properties:vec![] };
    let scene = crate::spectator::SpectatorScene {
        surface:Arc::new(SurfaceView {
            x:0,z:0,y_min:64,y_max:111,width:16,depth:16,
            voxels:vec![SurfaceVoxel {
                x:0,y:70,z:0,span:1,state,biome:None,
                original_block_entity:false,staged:false,
                faces:ALL_FACES,kind:VoxelMaterial::Solid,
            }],
            source_tiles:vec![],source_chunks:1,missing_columns:0,
        }),
        texture_bank:Some(bank),
        show_textures:true,
        camera:crate::spectator::Camera::default(),
        quality:crate::gpu_mesh::Quality::Balanced,
        cache:Arc::new(Mutex::new(None)),
        pending:Arc::new(Mutex::new(false)),
        fallback: None,
        chunk_cache:Arc::new(Mutex::new(
            crate::chunk_mesh_cache::ChunkMeshCache::default()
        )),
    };
    let mesh = crate::spectator::build(&scene);
    assert_eq!(mesh.len(),12);
    let extents = |axis:usize| (
        mesh.iter().map(|v|v.xyz[axis]).fold(f32::INFINITY,f32::min),
        mesh.iter().map(|v|v.xyz[axis]).fold(f32::NEG_INFINITY,f32::max),
    );
    assert_eq!(extents(0),(-1.5,1.5));
    assert_eq!(extents(1),(5.5,6.5));
    assert_eq!(extents(2),(-1.0,1.0));
    assert!(mesh.iter().all(|v|v.uv.iter().all(|component|component.is_finite())));
}

#[test]
fn generic_mod_model_sprites_resolve_in_both_zip_and_directory() {
    let fixture = Fixture::new();
    let directory = fixture.path("nested_model");
    let archive = fixture.path("nested_model.zip");
    let entries = [
        ("assets/create.addon/textures/item/machine/gizmo.png",
            png(&[22, 66, 204, 255], 1)),
        ("assets/create.addon/textures/entity/machine/side.png",
            png(&[208, 32, 78, 255], 1)),
        ("assets/create.addon/blockstates/gizmo.json",
            br#"{"variants":{"":{"model":"create.addon:item/machine/gizmo"}}}"#.to_vec()),
        ("assets/create.addon/models/item/machine/gizmo.json",
            br##"{"parent":"create.addon:custom/model/base","textures":{
                "all":"create.addon:item/machine/gizmo",
                "side":"create.addon:entity/machine/side"}}"##.to_vec()),
        ("assets/create.addon/models/custom/model/base.json",
            br##"{"elements":[{"from":[-16,0,-8],"to":[32,16,24],"faces":{
                "north":{"texture":"#side","uv":[0,0,16,16]},
                "up":{"texture":"#all","uv":[0,0,16,16]}}}]}"##.to_vec()),
    ];
    for (filename, contents) in &entries { put(&directory,filename,contents); }
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    for (filename, contents) in entries.iter().rev() {
        zip.start_file(*filename,zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap();
    for pack in [directory,archive] {
        let bank = TextureBank::open(&pack).unwrap();
        assert_eq!(color(&bank,"create.addon:item/machine/gizmo"),[22,66,204,255]);
        assert_eq!(color(&bank,"create.addon:entity/machine/side"),[208,32,78,255]);
        let state = mcedit2_world::sections::State {
            name:"create.addon:gizmo".into(),properties:vec![]
        };
        let elements = bank.geometry_at(&state,(1,64,1)).unwrap();
        assert_eq!(elements.len(),1);
        assert_eq!(elements[0].bounds.min,[-1.5,-0.5,-1.0]);
        for face in [crate::isometric::Face::Top, crate::isometric::Face::NegZ] {
            assert!(bank.atlas_face_style_at(&state,face,0,(1,64,1)).is_some());
        }
        assert!(bank.atlas().unwrap().entries.contains_key("create.addon:item/machine/gizmo"));
        assert!(bank.atlas().unwrap().entries.contains_key("create.addon:entity/machine/side"));
    }
}

#[test]
fn original_json_repeating_uvs_emit_real_atlas_safe_spectator_faces() {
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
    use std::sync::{Arc, Mutex};
    let fx = Fixture::new();
    let path = fx.path("uv_pack");
    put(&path, "assets/example/textures/item/tiling.png", &png(&[105, 80, 230, 255], 1));
    put(&path, "assets/example/blockstates/tile.json", br#"{"variants":{"":{"model":"example:block/tiling"}}}"#);
    put(&path, "assets/example/models/block/tiling.json", br##"{
        "textures":{"all":"example:item/tiling"},
        "elements":[{"from":[0,0,0],"to":[16,16,16],
        "faces":{"up":{"texture":"#all","uv":[-16,0,32,32]}}}]
    }"##);
    let bank = Arc::new(TextureBank::open(&path).unwrap());
    let state = State { name: "example:tile".into(), properties: vec![] };
    assert!(bank.atlas_repeating_face_at(&state, crate::isometric::Face::Top, 0, (0,70,0)).is_some());
    let slot = bank.atlas().unwrap().entries.get("example:item/tiling").unwrap().clone();
    let scene = crate::spectator::SpectatorScene {
        surface: Arc::new(SurfaceView {
            x:0,z:0,y_min:64,y_max:111,width:16,depth:16,
            voxels:vec![SurfaceVoxel {
                x:0,y:70,z:0,span:1,state,biome:None,
                original_block_entity:false,staged:false,
                faces:ALL_FACES,kind:VoxelMaterial::Solid,
            }],
            source_tiles:vec![],source_chunks:1,missing_columns:0,
        }),
        texture_bank:Some(bank),show_textures:true,
        camera:crate::spectator::Camera::default(),
        quality:crate::gpu_mesh::Quality::Balanced,
        cache:Arc::new(Mutex::new(None)),pending:Arc::new(Mutex::new(false)),
        fallback: None,
        chunk_cache:Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
    };
    let mesh = crate::spectator::build(&scene);
    assert_eq!(mesh.len(), 36);
    assert!(mesh.iter().all(|v| v.uv[0] >= slot.start[0] && v.uv[0] <= slot.end[0]));
    assert!(mesh.iter().all(|v| v.uv[1] >= slot.start[1] && v.uv[1] <= slot.end[1]));
    assert!(mesh.iter().all(|v| v.atlas_page == slot.page));
}

#[test]
fn declared_obj_loader_emits_generic_static_mesh_via_gpu_path() {
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
    use std::sync::{Arc, Mutex};
    let fixture = Fixture::new();
    let pack = fixture.path("obj_pack");
    put(&pack, "assets/example/textures/item/casing.png", &png(&[3,127,241,255], 1));
    put(&pack, "assets/example/blockstates/widget.json",
        br#"{"variants":{"":{"model":"example:block/widget"}}}"#);
    put(&pack, "assets/example/models/block/widget.json",
        br#"{"loader":"neoforge:obj","model":"example:models/block/widget.obj",
            "textures":{"casing":"example:item/casing"}}"#);
    put(&pack, "assets/example/models/block/widget.obj",
        b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl casing\nf 1/1 2/2 3/3 4/4\n");
    let bank = Arc::new(TextureBank::open(&pack).unwrap());
    let state = State { name:"example:widget".into(), properties:vec![] };
    let mesh = bank.static_obj_at(&state, (0,70,0)).unwrap();
    assert_eq!(mesh.len(), 2);
    assert_eq!(mesh[0].xyz[0], [-0.5,-0.5,-0.5]);
    assert_eq!(mesh[1].xyz[2], [-0.5,0.5,-0.5]);
    let scene = crate::spectator::SpectatorScene {
        surface: Arc::new(SurfaceView {
            x:0,z:0,y_min:64,y_max:111,width:16,depth:16,
            voxels:vec![SurfaceVoxel {
                x:0,y:70,z:0,span:1,state,biome:None,
                original_block_entity:false,staged:false,
                faces:ALL_FACES,kind:VoxelMaterial::Solid,
            }],
            missing_columns:0,source_chunks:1,source_tiles:vec![],
        }),
        texture_bank:Some(bank),show_textures:true,
        camera:crate::spectator::Camera::default(),
        quality:crate::gpu_mesh::Quality::Balanced,
        cache:Arc::new(Mutex::new(None)),pending:Arc::new(Mutex::new(false)),
        fallback: None,
        chunk_cache:Arc::new(Mutex::new(crate::chunk_mesh_cache::ChunkMeshCache::default())),
    };
    let vertices = crate::spectator::build(&scene);
    assert_eq!(vertices.len(),12);
    assert!(vertices.iter().all(|v|v.atlas_page==mesh[0].page));
    assert!(vertices.iter().all(|v|v.uv.iter().all(|x|x.is_finite())));
}

#[test]
fn obj_mtl_and_gltf_renderers_follow_resource_model_without_block_id_tables() {
    use mcedit2_world::sections::State;
    let fixture = Fixture::new();
    let path = fixture.path("special_renderers");
    put(&path,"assets/example/textures/item/casing.png",&png(&[40,80,120,255],1));
    put(&path,"assets/example/blockstates/casing.json",
        br#"{"variants":{"":{"model":"example:block/casing"}}}"#);
    put(&path,"assets/example/models/block/casing.json",
        br#"{"loader":"neoforge:obj","model":"example:models/block/casing.obj"}"#);
    put(&path,"assets/example/models/block/casing.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 3 0\nvt 0 2\nusemtl shell\nf 1/1 2/2 3/3\n");
    put(&path,"assets/example/models/block/casing.mtl",
        b"newmtl shell\nKd 0.25 0.5 1.0\nd 0.5\nmap_Kd example:item/casing\n");
    let bank = TextureBank::open(&path).unwrap();
    let state=State{name:"example:casing".into(),properties:vec![]};
    let mesh=bank.static_obj_at(&state,(0,70,0)).unwrap();
    assert!(mesh.len()>1,"UV repetition must split OBJ triangles");
    assert!(mesh.iter().all(|m|m.tint==[0.25,0.5,1.0,0.5]));
    let sprite=bank.atlas().unwrap().entries.get("example:item/casing").unwrap();
    assert!(mesh.iter().all(|m|m.uv.iter().all(|uv|
        uv[0]>=sprite.start[0]-1e-6&&uv[0]<=sprite.end[0]+1e-6
        && uv[1]>=sprite.start[1]-1e-6&&uv[1]<=sprite.end[1]+1e-6)));

    // Replace the blockstate's model reference with a glTF/GLB backend.
    let path2=fixture.path("gltf_renderers");
    put(&path2,"assets/example/textures/item/casing.png",&png(&[40,80,120,255],1));
    put(&path2,"assets/example/blockstates/casing.json",
        br#"{"variants":{"":{"model":"example:block/casing"}}}"#);
    put(&path2,"assets/example/models/block/casing.json",
        br#"{"loader":"mcedit2:glb","model":"example:models/block/casing.glb","textures":{"particle":"example:item/casing"}}"#);
    let mut bin=Vec::new();
    for v in [0f32,0.,0.,1.,0.,0.,0.,1.,0.,0.,0.,1.,0.,0.,1.] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    let doc=serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":60}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},
            {"buffer":0,"byteOffset":36,"byteLength":24}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1}}]}]});
    let mut json=serde_json::to_vec(&doc).unwrap();
    while json.len()%4!=0 {json.push(b' ')}
    let size=(12+8+json.len()+8+bin.len()) as u32;
    let mut glb=Vec::new();
    glb.extend_from_slice(b"glTF");glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&size.to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");glb.extend_from_slice(&json);
    glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"BIN\0");glb.extend_from_slice(&bin);
    put(&path2,"assets/example/models/block/casing.glb",&glb);
    let bank=TextureBank::open(&path2).unwrap();
    assert_eq!(bank.static_obj_at(&state,(0,70,0)).unwrap().len(),1);
}

#[test]
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

#[test]
fn nested_gltf_scene_uses_external_buffers_in_unordered_zip_pack(){
    let temp=Fixture::new();
    let archive=temp.path("gltf_buffers.zip");
    let mut positions=Vec::new();
    for n in [0f32,0.,0.,1.,0.,0.,0.,1.,0.] {
        positions.extend_from_slice(&n.to_le_bytes());
    }
    let mut tex=Vec::new();
    for n in [0f32,0.,1.,0.,0.,1.] {
        tex.extend_from_slice(&n.to_le_bytes());
    }
    let doc=serde_json::json!({
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
    let mut zip=zip::ZipWriter::new(File::create(&archive).unwrap());
    for (name,data) in [
        ("assets/example/models/block/assembly.gltf",serde_json::to_vec(&doc).unwrap()),
        ("assets/example/models/block/uv.bin",tex),
        ("assets/example/models/block/position.bin",positions),
        ("assets/example/blockstates/assembly.json",
            br#"{"variants":{"":{"model":"example:block/assembly"}}}"#.to_vec()),
        ("assets/example/models/block/assembly.json",
            br#"{"loader":"mcedit2:gltf","model":"example:models/block/assembly.gltf",
                "textures":{"particle":"example:item/assembly"}}"#.to_vec()),
        ("assets/example/textures/item/assembly.png",png(&[95,175,241,255],1))
    ] {
        zip.start_file(name,zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&data).unwrap();
    }
    zip.finish().unwrap();
    let bank=TextureBank::open(&archive).unwrap();
    let state=mcedit2_world::sections::State{
        name:"example:assembly".into(),properties:vec![]
    };
    let selected=bank.static_obj_at(&state,(0,70,0)).unwrap();
    assert_eq!(selected.len(),1);
    assert_eq!(selected[0].xyz[0],[-0.25,0.,-0.5]);
    assert_eq!(selected[0].xyz[1],[0.75,0.,-0.5]);
    assert!(!bank.warnings.iter().any(|x|x.contains("glTF")));
}

#[test]
fn obj_material_map_d_is_baked_into_actual_atlas_sprite_alpha(){
    let f=Fixture::new();
    let dir=f.path("material_map");
    put(&dir,"assets/example/textures/item/base.png",
        &png(&[125,180,230,255, 10,50,90,128],2));
    put(&dir,"assets/example/textures/item/mask.png",
        &png(&[0,0,0,255, 255,255,255,255],2));
    put(&dir,"assets/example/blockstates/widget.json",
        br#"{"variants":{"":{"model":"example:block/assembly"}}}"#);
    put(&dir,"assets/example/models/block/assembly.json",
        br#"{"loader":"neoforge:obj","model":"example:models/block/assembly.obj",
            "textures":{"particle":"example:item/base"}}"#);
    put(&dir,"assets/example/models/block/assembly.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl casing\nf 1/1 2/2 3/3\n");
    put(&dir,"assets/example/models/block/assembly.mtl",
        b"newmtl casing\nKd 1 1 1\nmap_Kd example:item/base\nmap_d example:item/mask\n");
    let bank=TextureBank::open(&dir).unwrap();
    let source=mcedit2_world::sections::State{
        name:"example:widget".into(),properties:vec![]
    };
    let mesh=bank.models.obj_at(&source,(0,64,0)).unwrap();
    let composed=bank.get(&mesh[0].texture).unwrap();
    assert_eq!(composed.pixel(0.5,0.25),[125,180,230,0]);
    assert_eq!(composed.pixel(0.5,0.75),[10,50,90,128]);
    let vertices=bank.static_obj_at(&source,(0,64,0)).unwrap();
    assert_eq!(vertices.len(),1);
    assert_eq!(vertices[0].alpha,AlphaMode::Blend);
}

#[test]
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
        put(&root,"assets/example/models/block/panel.gltf",
            &serde_json::to_vec(&gltf).unwrap());
        put(&root,"assets/example/models/block/positions.bin",&positions);
        put(&root,"assets/example/models/block/uv.bin",&tex);
        put(&root,"assets/example/textures/item/panel.png",
            &png(&[100,170,200,80],1));
        put(&root,"assets/example/blockstates/panel.json",
            br#"{"variants":{"":{"model":"example:block/panel"}}}"#);
        put(&root,"assets/example/models/block/panel.json",
            br#"{"loader":"mcedit2:gltf","model":"example:models/block/panel.gltf",
                "textures":{"panel":"example:item/panel"}}"#);
        let bank=TextureBank::open(&root).unwrap();
        let state=mcedit2_world::sections::State {
            name:"example:panel".into(), properties:vec![],
        };
        let faces=bank.static_obj_at(&state,(0,70,0)).unwrap();
        assert_eq!(faces.len(),1);
        assert_eq!(faces[0].mode & 3,expected_kind);
        assert_ne!(faces[0].mode & 4,0,"Single-sided glTF material must discard backfaces");
        assert_eq!(faces[0].tint,[0.2,0.4,0.6,0.25]);
        match expected_kind {
            0 => assert_ne!(faces[0].mode & 8,0),
            1 => {
                assert_ne!(faces[0].mode & 16,0);
                assert_eq!((faces[0].mode >> 8)&255,191);
            },
            _ => assert_eq!(faces[0].mode & 8,0),
        }
    }
}

#[test]
fn gltf_base_color_texture_autobinds_and_spectator_instances_are_distinct() {
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
    use std::sync::{Arc, Mutex};
    let fixture = Fixture::new();
    let root = fixture.path("gltf_auto_texture");
    let mut binary = Vec::new();
    for value in [0f32, 0., 0., 1., 0., 0., 0., 1., 0.,
                  0., 0., 1., 0., 0., 1.] {
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
    put(&root, "assets/example/models/block/assembly.gltf",
        &serde_json::to_vec(&doc).unwrap());
    put(&root, "assets/example/models/block/geometry.bin", &binary);
    put(&root, "assets/example/models/block/panel.png",
        &png(&[20,130,220,255], 1));
    put(&root, "assets/example/blockstates/assembly.json",
        br#"{"variants":{"":{"model":"example:block/assembly"}}}"#);
    put(&root, "assets/example/models/block/assembly.json",
        br#"{"loader":"mcedit2:gltf","model":"example:models/block/assembly.gltf"}"#);
    let bank = Arc::new(TextureBank::open(&root).unwrap());
    let state = State{name:"example:assembly".into(),properties:vec![]};
    assert_eq!(color(&bank, "example:__gltf/block/panel"), [20,130,220,255]);
    let selected = bank.models.obj_at(&state,(0,70,0)).unwrap();
    assert_eq!(selected.len(),2);
    assert!(selected.iter().all(|t|t.texture=="example:__gltf/block/panel"));
    assert_ne!(selected[0].xyz[0],selected[1].xyz[0]);
    let surfaces = bank.static_obj_at(&state,(0,70,0)).unwrap();
    assert_eq!(surfaces.len(),2);
    let page = bank.atlas().unwrap().entries["example:__gltf/block/panel"].page;
    assert!(surfaces.iter().all(|tri|tri.page==page));
    let scene=crate::spectator::SpectatorScene {
        surface:Arc::new(SurfaceView {
            x:0,z:0,y_min:64,y_max:111,width:16,depth:16,
            voxels:vec![SurfaceVoxel {
                x:0,y:70,z:0,span:1,state,biome:None,
                staged:false,original_block_entity:false,
                faces:ALL_FACES,kind:VoxelMaterial::Solid,
            }],
            source_tiles:vec![],source_chunks:1,missing_columns:0,
        }),
        texture_bank:Some(bank),show_textures:true,
        camera:crate::spectator::Camera::default(),
        quality:crate::gpu_mesh::Quality::Balanced,
        cache:Arc::new(Mutex::new(None)),pending:Arc::new(Mutex::new(false)),
        fallback: None,
        chunk_cache:Arc::new(Mutex::new(
            crate::chunk_mesh_cache::ChunkMeshCache::default()
        )),
    };
    let vertices=crate::spectator::build(&scene);
    assert_eq!(vertices.len(),12);
    assert!(vertices.iter().all(|v|v.atlas_page==page));
    assert!(vertices.iter().all(|v|v.uv.iter().all(|x|x.is_finite())));
}


#[test]
fn core_java_resource_reader_does_not_import_custom_mod_renderers() {
    let fixture = Fixture::new();
    let directory = fixture.path("standard_only");
    let zip_path = fixture.path("standard_only.zip");
    let inputs: Vec<(&str, Vec<u8>)> = vec![
        ("assets/example/textures/block/casing.png", png(&[12, 110, 208, 255], 1)),
        ("assets/example/textures/block/unresolved.png", png(&[220, 30, 11, 255], 1)),
        ("assets/example/blockstates/casing.json",
            br#"{"variants":{"":{"model":"example:block/casing"}}}"#.to_vec()),
        ("assets/example/models/block/casing.json",
            br##"{"parent":"minecraft:block/cube_all","textures":{"all":"example:block/casing"}}"##.to_vec()),
        ("assets/example/blockstates/custom.json",
            br#"{"variants":{"":{"model":"example:block/custom"}}}"#.to_vec()),
        ("assets/example/models/block/custom.json",
            br##"{"loader":"neoforge:obj","model":"example:models/block/custom.obj",
                "parent":"minecraft:block/cube_all","textures":{"all":"example:block/casing"}}"##.to_vec()),
        ("assets/example/models/block/custom.obj",
            b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()),
        ("assets/example/models/block/custom.mtl", b"newmtl custom\n".to_vec()),
        ("assets/example/models/block/custom.gltf", br#"{"asset":{"version":"2.0"}}"#.to_vec()),
        ("assets/example/models/block/custom.bin", b"not a vanilla resource".to_vec()),
        ("assets/example/models/block/sidecar.png", png(&[200, 200, 0, 255], 1)),
    ];
    for (name, bytes) in &inputs { put(&directory, name, bytes); }
    {
        let mut writer = zip::ZipWriter::new(File::create(&zip_path).unwrap());
        for (name, bytes) in inputs.iter().rev() {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();
    }
    for source in [&directory, &zip_path] {
        let bank = TextureBank::open_stack_java(&[source.to_path_buf()]).unwrap();
        let casing = state("example:casing", vec![]);
        let custom = state("example:custom", vec![]);
        let unresolved = state("example:unresolved", vec![]);
        assert_eq!(bank.model_count(), 4);
        assert_eq!(color(&bank, "example:casing"), [12, 110, 208, 255]);
        // Sidecar images under models/ are only used by non-vanilla formats.
        assert!(!bank.atlas().unwrap().entries.contains_key("example:__gltf/block/sidecar"));
        let (uv, _, page) = bank.atlas_face_style_at(
            &casing, crate::isometric::Face::Top, 0, (0, 64, 0)).unwrap();
        assert_eq!(bank.atlas_pixel([
            (uv[0][0] + uv[2][0]) * 0.5,
            (uv[0][1] + uv[2][1]) * 0.5,
        ], page), Some([12, 110, 208, 255]));
        assert!(bank.geometry_at(&casing, (0, 64, 0)).is_some());
        assert!(bank.geometry_at(&custom, (0, 64, 0)).is_none());
        assert!(bank.static_obj_at(&custom, (0, 64, 0)).is_none());
        for missing in [&custom, &unresolved] {
            assert!(bank.resolve_id_at(missing, crate::isometric::Face::Top, (0, 64, 0)).is_none());
            let (uv, _, page) = bank.atlas_face_style_at(
                missing, crate::isometric::Face::Top, 0, (0, 64, 0)).unwrap();
            let pixel = bank.atlas_pixel([
                (uv[0][0] + uv[2][0]) * 0.5,
                (uv[0][1] + uv[2][1]) * 0.5,
            ], page);
            assert!(matches!(pixel, Some([248, 0, 248, 255] | [0, 0, 0, 255])));
        }
    }
}
