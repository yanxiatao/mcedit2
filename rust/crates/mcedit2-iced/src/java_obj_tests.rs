//! 通过生产 Java 资源读取、模型图和网格验证 OBJ，不走实验资源入口。
use crate::{
    gpu_mesh::Quality,
    spectator::{Camera, SpectatorScene},
    textures::{LoadControl, TextureBank},
};
use image::ImageEncoder;
use mcedit2_world::{
    sections::State,
    surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
pub(crate) struct Fixture {
    pub path: PathBuf,
    parent: PathBuf,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let path = parent.join(format!(
            "mcedit2-java-obj-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path, parent }
    }
    pub(crate) fn put(&self, name: &str, bytes: &[u8]) {
        let path = self.path.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    pub(crate) fn open(&self) -> Arc<TextureBank> {
        Arc::new(
            TextureBank::open_stack_java_controlled(
                std::slice::from_ref(&self.path),
                &LoadControl {
                    all_png: false,
                    ..Default::default()
                },
            )
            .unwrap(),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert_eq!(self.path.parent(), Some(self.parent.as_path()));
        assert!(self
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("mcedit2-java-obj-"));
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            eprintln!("OBJ 测试目录清理失败：{error}");
        }
    }
}
pub(crate) fn fixture() -> Fixture {
    let f = Fixture::new();
    for (path, color) in [
        ("item/red", [255, 0, 0, 255]),
        ("item/blue", [0, 0, 255, 255]),
        ("entity/direct", [0, 255, 0, 255]),
        ("block/stone", [100, 100, 100, 255]),
    ] {
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&color, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        f.put(&format!("assets/demo/textures/{path}.png"), &bytes);
    }
    f.put(
        "assets/demo/blockstates/assembly.json",
        br#"{"variants":{"":{"model":"demo:block/assembly"}}}"#,
    );
    f.put("assets/demo/models/block/base.json",br##"{"loader":"neoforge:obj","model":"demo:models/mesh/shape.obj","flip_v":true,"visibility":{"Hidden":false},"textures":{"skin":"demo:item/red","alias":"#skin"},"render_type":"cutout"}"##);
    f.put("assets/demo/models/block/assembly.json",br##"{"loader":"neoforge:composite","children":{"left":{"parent":"demo:block/base","transform":{"translation":[-0.55,0,0],"scale":[-0.5,0.5,0.5],"origin":"center"},"render_type":"solid"},"right":{"parent":"demo:block/base","textures":{"skin":"demo:item/blue"},"visibility":{"Body":false,"Hidden":true},"transform":{"translation":[0.55,0,0],"scale":[0.5,0.5,0.5],"origin":"center"},"render_type":"translucent"},"vanilla":{"loader":"neoforge:elements","textures":{"all":"demo:block/stone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all"}}}]}}}"##);
    f.put("assets/demo/models/mesh/shape.obj",b"mtllib source.mtl\nv 0.1 0.1 0\nv 0.9 0.1 0\nv 0.1 0.9 0\nvt 0.1 0.2\nvt 0.9 0.2\nvt 0.1 0.9\nvn 0 0 -1\nvc 1 1 1 1\nvc 1 0.5 1 0.5\nvc 0.5 1 1 0.75\no Body\ng Surface\nusemtl Painted Body\nf 1/1/1/1 3/3/1/3 2/2/1/2\no Hidden\nf 1/1/1/1 3/3/1/3 2/2/1/2\n");
    f.put("assets/demo/models/mesh/source.mtl",b"newmtl Painted Body\nKd 1 1 1 0.5\nd 0.1\nmap_Kd -s 1 1 1 #alias\nneoforge_TintIndex -1\nnewmtl Unused\nmap_Kd demo:entity/direct\n");
    f
}
pub(crate) fn scene(bank: Arc<TextureBank>) -> SpectatorScene {
    SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface: Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 15,
            width: 16,
            depth: 16,
            voxels: vec![SurfaceVoxel::new(
                [0, 0, 0],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: "demo:assembly".into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    kind: VoxelMaterial::Solid,
                    staged: false,
                    original_block_entity: false,
                    faces: ALL_FACES,
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
        camera: Camera {
            position: [0.0, 0.0, -2.0],
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
            perspective: true,
        },
        quality: Quality::Fast,
        cache: Default::default(),
        presented: Default::default(),
        presented_view: Default::default(),
        pending: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Default::default(),
        lighting: crate::lighting::Table::unlit(),
    }
}
pub(crate) fn isometric_scene(bank: Arc<TextureBank>) -> crate::isometric::IsometricSurface {
    let mut source = scene(Arc::clone(&bank)).surface;
    let surface = Arc::make_mut(&mut source);
    surface.x = -1;
    surface.z = -1;
    surface.width = 2;
    surface.depth = 2;
    surface.y_max = 0;
    crate::isometric::IsometricSurface {
        selection_handle: None,
        brush: None,
        brush_key: None,
        surface: source,
        texture_bank: Some(bank),
        show_textures: true,
        block_layers: Default::default(),
        camera_quarters: 0,
        yaw_degrees: 180,
        pitch_degrees: 35,
        zoom: 1.0,
        pan_x: 0.0,
        pan_y: 0.0,
        selected: None,
        hovered: None,
        selection_box: None,
        show_frame: false,
        show_axes: false,
        show_chunk_grid: false,
        show_section_guides: false,
        navigation_target: crate::view_navigation::Target::Isometric,
        gpu_faces: false,
        gpu_quality: Quality::Fast,
        canvas_cache: Default::default(),
        frame_cache: Default::default(),
        framing_cache: Default::default(),
        gpu_frame_cache: Default::default(),
        gpu_pending: Default::default(),
        entity_frame: None,
        entity_clip: None,
        rendered_cache: Default::default(),
        presented: Default::default(),
        lighting: crate::lighting::Table::unlit(),
    }
}

#[test]
fn production_composite_keeps_obj_groups_colors_uv_reflection_and_json_elements() {
    let fixture = fixture();
    let bank = fixture.open();
    assert!(bank.warnings.is_empty(), "{:?}", bank.warnings);
    assert!(bank
        .atlas()
        .unwrap()
        .entries
        .contains_key("demo:entity/direct"));
    let state = State {
        name: "demo:assembly".into(),
        properties: vec![],
    };
    let triangles = bank.models.obj_at(&state, (0, 0, 0)).unwrap();
    assert_eq!(triangles.len(), 2);
    assert_eq!(triangles[0].texture, "demo:item/red");
    assert_eq!(triangles[1].texture, "demo:item/blue");
    assert_eq!(triangles[0].uv[0], [0.1, 0.8]);
    assert_eq!(
        triangles[0].tint[3], 0.5,
        "d/Tr 不能覆盖 NeoForge 的 Kd Alpha"
    );
    assert!(triangles.iter().all(|t| !t.double_sided));
    assert!(triangles[0].xyz.iter().all(|p| p[0] < 0.0));
    assert!(triangles[1].xyz.iter().all(|p| p[0] > 0.0));
    assert_eq!(bank.geometry_at(&state, (0, 0, 0)).unwrap().len(), 1);
    let faces = bank.static_obj_at(&state, (0, 0, 0)).unwrap();
    assert_eq!(faces.len(), 2);
    assert_eq!(faces[0].rgba[0][3], 0.5);
    assert_eq!(faces[0].rgba[1][3], 0.25);
    assert_eq!(crate::material_mode::kind(faces[0].mode), 0);
    assert_ne!(faces[0].mode & crate::material_mode::FORCE_OPAQUE_ALPHA, 0);
    assert!(faces
        .iter()
        .all(|f| f.mode & crate::material_mode::SINGLE_SIDED != 0));
    assert_eq!(crate::material_mode::kind(faces[1].mode), 2);
    let view = scene(bank);
    let mesh = crate::spectator::build_scene_cancellable(&view, &|| false).unwrap();
    assert_eq!(
        mesh.chunks[0].quads().len(),
        3,
        "JSON 面与两个 OBJ 面都应保留"
    );
    let plain = crate::spectator::build_scene_cancellable(
        &SpectatorScene {
            show_textures: false,
            block_layers: Default::default(),
            ..view.clone()
        },
        &|| false,
    )
    .unwrap();
    assert_eq!(
        plain.chunks[0].quads().len(),
        3,
        "关闭纹理不能移除 OBJ 几何"
    );
    for (textured, plain) in mesh.chunks[0]
        .quads()
        .iter()
        .zip(plain.chunks[0].quads().iter())
    {
        assert_eq!(textured.xyz, plain.xyz);
    }
    for t in &triangles {
        let center: [f64; 3] =
            std::array::from_fn(|i| t.xyz.iter().map(|p| p[i] as f64).sum::<f64>() / 3.0);
        let size = iced::Size::new(400.0, 400.0);
        let camera = view.camera.uniform_at_origin([0; 3], size);
        let rel: [f64; 3] = std::array::from_fn(|i| center[i] - view.camera.position[i]);
        let depth = (0..3)
            .map(|i| rel[i] * camera.forward[i] as f64)
            .sum::<f64>();
        let x = (0..3).map(|i| rel[i] * camera.right[i] as f64).sum::<f64>();
        let y = (0..3).map(|i| rel[i] * camera.up[i] as f64).sum::<f64>();
        let point = iced::Point::new(
            (200.0 * (1.0 + x * camera.projection[0] as f64 / depth)) as f32,
            (200.0 * (1.0 - y * camera.projection[1] as f64 / depth)) as f32,
        );
        let hit = mesh
            .pick(
                view.camera,
                point,
                crate::render_viewport::RenderViewport::one_to_one(size),
                view.quality,
            )
            .unwrap();
        assert_eq!(hit.owner.unwrap().state.name, "demo:assembly");
    }
}

#[test]
fn unmatched_valid_multipart_never_invents_a_placeholder_cube() {
    let fixture = fixture();
    fixture.put(
        "assets/demo/blockstates/assembly.json",
        br#"{"multipart":[{"when":{"loaded":true},"apply":{"model":"demo:block/assembly"}}]}"#,
    );
    let bank = fixture.open();
    let mut spectator = scene(Arc::clone(&bank));
    Arc::make_mut(&mut Arc::make_mut(&mut spectator.surface).voxels[0].state).properties =
        vec![("loaded".into(), "false".into())];
    let state = &spectator.surface.voxels[0].state;
    assert!(bank.geometry_at(state, (0, 0, 0)).unwrap().is_empty());
    let mesh = crate::spectator::build_scene_cancellable(&spectator, &|| false).unwrap();
    assert_eq!(
        mesh.chunks.iter().map(|c| c.quads().len()).sum::<usize>(),
        0
    );
    let mut isometric = isometric_scene(bank);
    Arc::make_mut(&mut Arc::make_mut(&mut isometric.surface).voxels[0].state).properties =
        state.properties.clone();
    assert!(isometric.faces(iced::Size::new(400.0, 400.0)).is_empty());
    assert!(isometric
        .pick_face(
            iced::Point::new(200.0, 200.0),
            iced::Size::new(400.0, 400.0)
        )
        .is_none());
}

#[test]
fn production_obj_honors_mtl_override_variant_rotation_and_never_reads_external_paths() {
    let fixture = fixture();
    fixture.put("assets/demo/models/block/assembly.json",br##"{"parent":"demo:block/base","loader":"neoforge:obj","model":"demo:models/mesh/shape.obj","mtl_override":"demo:models/alternate.mtl","flip_v":false,"visibility":{"Hidden":false},"textures":{"alias":"demo:entity/direct"}}"##);
    fixture.put(
        "assets/demo/models/alternate.mtl",
        b"newmtl Painted Body\nKd 0.25 0.5 0.75 0.6\nmap_Kd #alias\n",
    );
    fixture.put(
        "assets/demo/blockstates/assembly.json",
        br#"{"variants":{"":{"model":"demo:block/assembly","y":180}}}"#,
    );
    let bank = fixture.open();
    let state = State {
        name: "demo:assembly".into(),
        properties: vec![],
    };
    let triangles = bank.models.obj_at(&state, (0, 0, 0)).unwrap();
    assert_eq!(triangles.len(), 1);
    assert_eq!(triangles[0].texture, "demo:entity/direct");
    assert_eq!(triangles[0].tint, [0.25, 0.5, 0.75, 0.6]);
    assert!(triangles[0].xyz.iter().all(|p| (p[2] - 0.5).abs() < 1e-5));
    assert_eq!(triangles[0].cull, Some(crate::isometric::Face::PosZ));
    fixture.put("assets/demo/models/block/assembly.json",br#"{"loader":"neoforge:obj","model":"demo:models/mesh/shape.obj","mtl_override":"../private.mtl"}"#);
    let invalid = fixture.open();
    assert!(invalid.models.parts_at(&state, (0, 0, 0)).is_none());
    assert!(invalid
        .warnings
        .iter()
        .any(|w| w.contains("材质库引用无效")));
}

#[test]
fn obj_zip_and_directory_have_identical_compiled_meshes() {
    use std::io::Write;
    let fixture = fixture();
    let zipped = Fixture::new();
    let path = zipped.path.join("models.zip");
    let mut files = Vec::new();
    fn collect(path: &std::path::Path, out: &mut Vec<PathBuf>) {
        for item in std::fs::read_dir(path).unwrap() {
            let item = item.unwrap();
            if item.file_type().unwrap().is_dir() {
                collect(&item.path(), out)
            } else {
                out.push(item.path())
            }
        }
    }
    collect(&fixture.path, &mut files);
    files.sort();
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for file in files.iter().rev() {
        writer
            .start_file(
                file.strip_prefix(&fixture.path)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(&std::fs::read(file).unwrap()).unwrap();
    }
    writer.finish().unwrap();
    let directory = scene(fixture.open());
    let archive = scene(Arc::new(TextureBank::open_stack_java(&[path]).unwrap()));
    let a = crate::spectator::build(&directory);
    let b = crate::spectator::build(&archive);
    assert_eq!(a, b);
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn real_instance_native_obj_models_keep_visible_components_and_materials() {
    let path = std::env::var_os("MCEDIT2_TEST_WORLD").expect("设置 MCEDIT2_TEST_WORLD");
    let world = mcedit2_world::inspect_world(PathBuf::from(path)).unwrap();
    let stack = crate::auto_assets::discover_stack(
        &world.root,
        world.version_name.as_deref().unwrap(),
        &[],
    )
    .unwrap()
    .unwrap();
    let bank = TextureBank::open_stack_java_controlled(
        &stack.paths,
        &LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap();
    for (name, properties) in [
        ("xycraft_world:geyser", vec![]),
        (
            "xycraft_world:lamp_cube_black",
            vec![
                ("direction".into(), "down".into()),
                ("xl".into(), "false".into()),
            ],
        ),
        (
            "xycraft_machines:hydro_pump",
            vec![("direction".into(), "down".into())],
        ),
    ] {
        let state = State {
            name: name.into(),
            properties,
        };
        let triangles = bank
            .models
            .obj_at(&state, (0, 64, 0))
            .unwrap_or_else(|| panic!("真实模型无法解析：{name}"));
        assert!(!triangles.is_empty());
        let textured = bank.static_obj_at(&state, (0, 64, 0)).unwrap();
        assert!(!textured.is_empty());
        let textures = triangles
            .iter()
            .map(|t| t.texture.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        eprintln!(
            "真实 OBJ {name}: {} 三角形 / {} 图集面，材质 {:?}",
            triangles.len(),
            textured.len(),
            textures
        );
        assert!(textured
            .iter()
            .all(|t| t.rgba.iter().flatten().all(|v| v.is_finite())));
    }
    let errors = bank
        .warnings
        .iter()
        .filter(|w| w.contains("MTL") || w.contains("OBJ") || w.contains("静态模型资源"))
        .collect::<Vec<_>>();
    eprintln!("真实 OBJ/MTL 诊断（{} 条）：{:?}", errors.len(), errors);
    assert!(errors.is_empty(), "真实实例 OBJ/MTL 解析仍有缺口");
    let session =
        mcedit2_world::editing::Session::new(world.clone(), world.dimensions[0].name.clone())
            .unwrap();
    let mut reader =
        mcedit2_world::block_lookup::ReadOnlyRegionCache::new(&world, session.dimension()).unwrap();
    let mut positions = std::collections::BTreeMap::new();
    'chunks: for cx in -8..=8 {
        for cz in -8..=8 {
            let patch = mcedit2_world::chunk_surface::inspect_full_chunk_cached(
                &session,
                &mut reader,
                cx,
                cz,
            )
            .unwrap();
            for voxel in patch.voxels {
                if voxel.state.name == "xycraft_world:geyser" {
                    eprintln!(
                        "原始喷泉 {:?}，暴露面标记 {}",
                        [voxel.x, voxel.y, voxel.z],
                        voxel.faces
                    );
                    positions.insert(voxel.state.name.clone(), [voxel.x, voxel.y, voxel.z]);
                    break 'chunks;
                }
            }
        }
    }
    eprintln!("真实存档 OBJ 原始坐标：{positions:?}");
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/obj-source-positions.json"),
        serde_json::to_vec_pretty(&positions).unwrap(),
    )
    .unwrap();
}
