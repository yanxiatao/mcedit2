//! 真实存档的只读生产网格基准；显式运行，不用时间阈值代替几何验证。
use super::*;
use crate::textures::{LoadControl, TextureBank};
use mcedit2_world::{editing::Session, surface::SurfaceView};
use std::{path::PathBuf, time::Instant};

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn real_source_mesh_phase_profile() {
    let gpu_profile = std::env::var_os("MCEDIT2_GPU_PROFILE").is_some();
    let ui_profile = std::env::var_os("MCEDIT2_UI_PROFILE").is_some();
    let real_view = gpu_profile || ui_profile || std::env::var_os("MCEDIT2_APP_PROFILE").is_some();
    let world = mcedit2_world::inspect_world(PathBuf::from(
        std::env::var_os("MCEDIT2_TEST_WORLD").unwrap(),
    ))
    .unwrap();
    let mut assets = crate::auto_assets::discover_stack(
        &world.root,
        world.version_name.as_deref().unwrap(),
        &[],
    )
    .unwrap()
    .unwrap();
    if let Some(framework) = std::env::var_os("MCEDIT2_FRAMEWORK_JAR") {
        assets
            .paths
            .insert(usize::from(assets.vanilla), PathBuf::from(framework));
    }
    let bank = Arc::new(
        TextureBank::open_stack_java_controlled(
            &assets.paths,
            &LoadControl {
                all_png: real_view,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let radius = std::env::var("MCEDIT2_MESH_RADIUS")
        .ok()
        .map(|v| v.parse::<i32>().unwrap())
        .unwrap_or(1);
    assert!((1..=128).contains(&radius));
    if std::env::var_os("MCEDIT2_APP_PROFILE").is_some() {
        crate::application_probe::run(world, bank, radius as u16);
        return;
    }
    let session = Session::new(world.clone(), world.dimensions[0].name.clone()).unwrap();
    let mut reader =
        mcedit2_world::block_lookup::ReadOnlyRegionCache::new(&world, session.dimension()).unwrap();
    let read_started = Instant::now();
    let mut voxels = Vec::new();
    let mut tiles = Vec::new();
    let mut columns = Vec::new();
    let mut visibility = crate::source_visibility::Reader::new(Some(Arc::clone(&bank)), true);
    let center_z = if real_view { -2 } else { 0 };
    for cx in -radius..=radius {
        for cz in center_z - radius..=center_z + radius {
            let patch = if real_view {
                mcedit2_world::chunk_surface::inspect_full_chunk_with_visibility_cancellable(
                    &session,
                    &mut reader,
                    cx,
                    cz,
                    &mut visibility,
                    &|| false,
                )
                .unwrap()
            } else {
                mcedit2_world::chunk_surface::inspect_full_chunk_cached(
                    &session,
                    &mut reader,
                    cx,
                    cz,
                )
                .unwrap()
            };
            if let Some(sections) = &patch.sections {
                columns.push(crate::section_culling::SourceColumn {
                    coordinate: (cx, cz),
                    sections: Arc::clone(sections),
                });
            }
            voxels.extend(patch.voxels);
            tiles.push(patch.tile);
        }
    }
    let read = read_started.elapsed();
    let surface = Arc::new(SurfaceView {
        biomes: Default::default(),
        x: -radius * 16,
        z: (center_z - radius) * 16,
        y_min: -64,
        y_max: 319,
        width: (radius * 2 + 1) as u16 * 16,
        depth: (radius * 2 + 1) as u16 * 16,
        voxels,
        missing_columns: 0,
        source_chunks: tiles.len(),
        source_tiles: tiles,
    });
    let scene = SpectatorScene {
        entity_frame: None,
        entity_enabled: false,
        entity_cache: Default::default(),
        surface: Arc::clone(&surface),
        texture_bank: Some(Arc::clone(&bank)),
        show_textures: true,
        block_layers: Default::default(),
        camera: Default::default(),
        quality: Quality::Fast,
        cache: Default::default(),
        presented: Default::default(),
        presented_view: Default::default(),
        pending: Default::default(),
        fallback: None,
        snapshot: None,
        chunk_cache: Default::default(),
        lighting: crate::lighting::Table::unlit(),
    };
    let sources = [surface.voxels.as_slice()];
    let neighbors = crate::spectator_stream::ExactNeighbors::new(&sources);
    let start = Instant::now();
    let chunks = build_sources(
        &scene,
        &sources,
        SourceFrame {
            biomes: &surface.biomes,
            origin: [surface.x, surface.y_min, surface.z],
            y_max: surface.y_max,
            reference: false,
        },
        Some(&neighbors),
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let elapsed = start.elapsed();
    let faces = chunks
        .iter()
        .map(|chunk| chunk.quads().len())
        .sum::<usize>();
    assert!(faces > 0);
    if real_view {
        let mut reader =
            mcedit2_world::scene_annotations::Reader::new(&world, &world.dimensions[0].name)
                .unwrap();
        let mut parts = Vec::new();
        for x in -radius..=radius {
            for z in center_z - radius..=center_z + radius {
                parts.push(Arc::new(reader.read(x, z).unwrap()));
            }
        }
        let snapshot = Arc::new(crate::scene_layers::Snapshot::build(
            (0, 0),
            1,
            parts.into_iter(),
            None,
        ));
        let frame = Arc::new(
            crate::entity_geometry::Frame::build(
                snapshot,
                &bank,
                world.data_version,
                true,
                &|| false,
            )
            .unwrap(),
        );
        let entity_counts = (frame.rendered, frame.missing, frame.invisible);
        let terrain = Arc::new(StreamMesh {
            terrain: None,
            visibility: crate::section_culling::Topology::build_columns(columns, &|| false),
            entities: None,
            complete: true,
            chunks: chunks.into(),
            origin: [surface.x, surface.y_min, surface.z],
            atlas: bank.atlas().cloned(),
        });
        let mesh = crate::entity_geometry::compose(&terrain, Some(&frame), &Default::default());
        let camera = Camera {
            position: [2.5, 215.5, -17.5],
            yaw: 0.,
            pitch: 23.,
            zoom: 0.25,
            perspective: true,
        };
        if ui_profile {
            let events = super::controls::profile_controls(Arc::clone(&mesh), camera);
            let report = serde_json::json!({"world":world.root,"radius":radius,"terrain_faces":faces,"events":events});
            std::fs::write(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/ui-profile-world.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            eprintln!("真实世界交互事件基准：{report}");
            if !gpu_profile {
                return;
            }
        }
        let mut reference = (*mesh).clone();
        reference.visibility = None;
        let baseline = crate::gpu_scene::hardware_tests::profile_world(
            Arc::new(reference),
            camera,
            crate::gpu_scene::hardware_tests::WorldProfile {
                y32: false,
                multiple_views: true,
                material_hints: false,
                ..Default::default()
            },
        );
        let unhinted = crate::gpu_scene::hardware_tests::profile_world(
            Arc::clone(&mesh),
            camera,
            crate::gpu_scene::hardware_tests::WorldProfile {
                multiple_views: true,
                material_hints: false,
                ..Default::default()
            },
        );
        let y32 = crate::gpu_scene::hardware_tests::profile_world(
            mesh,
            camera,
            crate::gpu_scene::hardware_tests::WorldProfile {
                multiple_views: true,
                ..Default::default()
            },
        );
        let report = serde_json::json!({"world":world.root,"data_version":world.data_version,"radius":radius,"center_chunk":[0,center_z],"terrain_faces":faces,"entities":{"modeled":entity_counts.0,"missing":entity_counts.1,"invisible":entity_counts.2},"resource_png":bank.statistics.atlas_png,"baseline":baseline,"unhinted":unhinted,"y32":y32});
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/gpu-profile-world.json");
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        eprintln!("真实世界 GPU 离屏基准：{report}");
        return;
    }
    let mut reference_seconds = None;
    if std::env::var("MCEDIT2_MESH_REFERENCE").as_deref() == Ok("1") {
        let mut reference = scene.clone();
        reference.chunk_cache = Default::default();
        let start = Instant::now();
        let expected = build_sources(
            &reference,
            &sources,
            SourceFrame {
                biomes: &surface.biomes,
                origin: [surface.x, surface.y_min, surface.z],
                y_max: surface.y_max,
                reference: true,
            },
            Some(&neighbors),
            None,
            &|| false,
            &mut |_| {},
        )
        .unwrap();
        reference_seconds = Some(start.elapsed().as_secs_f64());
        assert_eq!(chunks.len(), expected.len());
        for (a, b) in chunks.iter().zip(&expected) {
            assert_eq!(a.key, b.key);
            assert_eq!(a.origin, b.origin);
            assert_eq!(a.quads().len(), b.quads().len());
            assert!(
                bytemuck::cast_slice::<_, u8>(&a.quads())
                    == bytemuck::cast_slice::<_, u8>(&b.quads()),
                "真实区块 {:?} 的网格字节不同",
                a.key
            );
            assert_eq!(a.picking.source_groups(), b.picking.source_groups());
        }
        eprintln!(
            "实际场景对照：{faces} 面 / {} 字节及全部源方块归属逐项一致",
            faces * std::mem::size_of::<WorldQuad>()
        );
    }
    let warm_started = Instant::now();
    let warm = build_sources(
        &scene,
        &sources,
        SourceFrame {
            biomes: &surface.biomes,
            origin: [surface.x + 16, surface.y_min, surface.z],
            y_max: surface.y_max,
            reference: false,
        },
        Some(&neighbors),
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap();
    let warm_seconds = warm_started.elapsed().as_secs_f64();
    assert_eq!(chunks.len(), warm.len());
    for (cold, warm) in chunks.iter().zip(&warm) {
        assert!(Arc::ptr_eq(&cold.quads(), &warm.quads()));
        assert!(Arc::ptr_eq(&cold.picking, &warm.picking));
    }
    let variant_started = Instant::now();
    for voxel in &surface.voxels {
        std::hint::black_box(bank.model_variant_key(&voxel.state, (voxel.x, voxel.y, voxel.z)));
    }
    let variant_time = variant_started.elapsed();
    let mut variants = templates::Variants::default();
    let prepared_started = Instant::now();
    for voxel in &surface.voxels {
        std::hint::black_box(variants.at(Some(&bank), &voxel.state, (voxel.x, voxel.y, voxel.z)));
    }
    let prepared_variant_seconds = prepared_started.elapsed().as_secs_f64();
    let mut payload = mcedit2_world::source_memory::Footprint::default();
    payload.include(&surface.voxels, surface.voxels.capacity());
    let source_heap_bytes = payload.usage().shared_bytes();
    let mut cache_inline_bytes = 0;
    for (source, bytes) in scene.chunk_cache.lock().unwrap().source_allocations() {
        cache_inline_bytes += bytes;
        payload.include(
            source,
            bytes / std::mem::size_of::<mcedit2_world::surface::SurfaceVoxel>(),
        );
    }
    let cache_heap_bytes = payload.usage().shared_bytes() - source_heap_bytes;
    for chunk in &chunks {
        for group in chunk.picking.source_groups() {
            if let Some(owner) = &group.owner {
                payload.shared_state(&owner.state);
            }
        }
    }
    let picking_heap_bytes = payload.usage().shared_bytes() - source_heap_bytes - cache_heap_bytes;
    let quad_bytes: usize = chunks
        .iter()
        .map(|c| c.quads().capacity() * std::mem::size_of::<WorldQuad>())
        .sum();
    let picking_bytes: usize = chunks.iter().map(|c| c.picking.storage_bytes()).sum();
    let stats = serde_json::json!({"radius":radius,"voxels":surface.voxels.len(),"faces":faces,"chunks":chunks.len(),
        "read_seconds":read.as_secs_f64(),"mesh_seconds":elapsed.as_secs_f64(),"variant_only_seconds":variant_time.as_secs_f64(),
        "reference_seconds":reference_seconds,"warm_seconds":warm_seconds,"prepared_variant_seconds":prepared_variant_seconds,
        "source_inline_bytes":surface.voxels.capacity()*std::mem::size_of::<mcedit2_world::surface::SurfaceVoxel>(),
        "source_and_cache_memory":payload.usage(),"source_heap_bytes":source_heap_bytes,"cache_inline_bytes":cache_inline_bytes,"cache_heap_bytes":cache_heap_bytes,
        "picking_index_bytes":picking_bytes,"picking_heap_bytes":picking_heap_bytes,"quad_capacity_bytes":quad_bytes});
    eprintln!("实际生产网格基准 {stats}");
    let tag = std::env::var("MCEDIT2_MESH_TAG").unwrap_or_else(|_| "current".into());
    assert!(tag.bytes().all(|v| v.is_ascii_alphanumeric() || v == b'-'));
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("mesh-profile-{tag}.json"));
    std::fs::write(out, serde_json::to_vec_pretty(&stats).unwrap()).unwrap();
}
