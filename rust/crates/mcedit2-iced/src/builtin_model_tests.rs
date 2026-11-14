//! 特殊方块经生产资源索引、真实实体贴图依赖及三条渲染路径的逻辑回归。
use super::*;
use crate::{isometric::Face, textures::java_obj_tests::Fixture};
use image::ImageEncoder;
use mcedit2_world::editing::parse_state;
use std::{io::Write, path::PathBuf};

const COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

#[test]
fn unidentified_legacy_chest_sheets_are_diagnosed_instead_of_misinterpreted() {
    let f = fixture();
    for side in ["left", "right"] {
        fs::remove_file(f.path.join(format!(
            "assets/minecraft/textures/entity/chest/normal_{side}.png"
        )))
        .unwrap();
    }
    let bank = f.open();
    let state = parse_state("minecraft:chest[facing=south,type=single]").unwrap();
    assert!(bank.geometry_at(&state, (0, 0, 0)).unwrap().is_empty());
    assert!(bank
        .warnings
        .iter()
        .any(|s| s.contains("minecraft:chest") && s.contains("旧实体贴图")));
    let bed = parse_state("minecraft:red_bed[facing=north,part=head]").unwrap();
    assert_eq!(bank.geometry_at(&bed, (0, 0, 0)).unwrap().len(), 3);
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn real_special_source_positions() {
    let world = mcedit2_world::inspect_world(PathBuf::from(
        std::env::var_os("MCEDIT2_TEST_WORLD").expect("设置 MCEDIT2_TEST_WORLD"),
    ))
    .unwrap();
    let dimension = &world.dimensions[0];
    let mut reader =
        mcedit2_world::block_lookup::ReadOnlyRegionCache::new(&world, &dimension.name).unwrap();
    let mut found: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for cz in -8..=8 {
        for cx in -8..=8 {
            let Some(chunk) = reader.read_chunk(cx, cz).unwrap() else {
                continue;
            };
            for section in &chunk.sections {
                let selected: Vec<_> = section
                    .palette
                    .iter()
                    .map(|state| {
                        matches!(
                            state.name.as_str(),
                            "minecraft:chest" | "minecraft:trapped_chest" | "minecraft:ender_chest"
                        ) || crate::textures::shulker_tests::COLORS.iter().any(|c| {
                            state.name
                                == format!(
                                    "minecraft:{}",
                                    crate::textures::shulker_tests::names(c).0
                                )
                        }) || state
                            .name
                            .strip_prefix("minecraft:")
                            .and_then(|s| s.strip_suffix("_bed"))
                            .is_some_and(|c| COLORS.contains(&c))
                    })
                    .collect();
                if !selected.iter().any(|&selected| selected) {
                    continue;
                }
                for (i, index) in section.indices.iter().enumerate() {
                    if !selected[*index as usize] {
                        continue;
                    }
                    let state = &section.palette[*index as usize];
                    let entries = found.entry(state.name.clone()).or_default();
                    if entries.len() < 4 {
                        let x = cx * 16 + (i % 16) as i32;
                        let y = i32::from(section.y) * 16 + (i / 256) as i32;
                        let z = cz * 16 + ((i / 16) % 16) as i32;
                        entries.push(serde_json::json!({"pos":[x,y,z],"name":state.name,"properties":state.properties}));
                    }
                }
            }
        }
    }
    assert!(!found.is_empty(), "实际观察区至少应找到一类特殊方块");
    let text = serde_json::to_string_pretty(&found).unwrap();
    eprintln!("{text}");
    fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/builtin-source-positions.json"),
        text,
    )
    .unwrap();
}
pub(crate) fn fixture() -> Fixture {
    let f = Fixture::new();
    for (block, model) in [
        ("chest", "chest"),
        ("trapped_chest", "chest"),
        ("ender_chest", "ender_chest"),
    ] {
        f.put(
            &format!("assets/minecraft/blockstates/{block}.json"),
            serde_json::to_vec(
                &serde_json::json!({"variants":{"":{"model":format!("minecraft:block/{model}")}}}),
            )
            .unwrap()
            .as_slice(),
        );
    }
    for model in ["chest", "ender_chest", "bed"] {
        f.put(
            &format!("assets/minecraft/models/block/{model}.json"),
            br#"{"textures":{"particle":"minecraft:block/oak_planks"}}"#,
        );
    }
    let mut textures: Vec<String> = [
        "normal",
        "normal_left",
        "normal_right",
        "trapped",
        "trapped_left",
        "trapped_right",
        "ender",
    ]
    .into_iter()
    .map(|n| format!("entity/chest/{n}"))
    .collect();
    for color in COLORS {
        f.put(
            &format!("assets/minecraft/blockstates/{color}_bed.json"),
            br#"{"variants":{"":{"model":"minecraft:block/bed"}}}"#,
        );
        textures.push(format!("entity/bed/{color}"));
    }
    for (i, id) in textures.iter().enumerate() {
        let pixels = image::RgbaImage::from_fn(64, 64, |x, y| {
            image::Rgba([x as u8 * 3, y as u8 * 3, (i * 9 + 3) as u8, 255])
        });
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&pixels, 64, 64, image::ExtendedColorType::Rgba8)
            .unwrap();
        f.put(&format!("assets/minecraft/textures/{id}.png"), &bytes);
    }
    f
}
fn center(part: &crate::models::Part) -> [f32; 3] {
    part.point(std::array::from_fn(|i| {
        (part.bounds.min[i] + part.bounds.max[i]) * 0.5
    }))
}

#[test]
fn all_modern_chests_keep_body_lid_latch_facing_halves_and_entity_sheet() {
    let f = fixture();
    let bank = f.open();
    assert!(bank.warnings.is_empty(), "{:?}", bank.warnings);
    assert_eq!(bank.statistics.atlas_png, 23);
    for (family, sheet) in [
        ("chest", "normal"),
        ("trapped_chest", "trapped"),
        ("ender_chest", "ender"),
    ] {
        let kinds: &[&str] = if family == "ender_chest" {
            &["single"]
        } else {
            &["single", "left", "right"]
        };
        for &kind in kinds {
            for (facing, axis, sign) in [
                ("south", 2, 1.),
                ("west", 0, -1.),
                ("north", 2, -1.),
                ("east", 0, 1.),
            ] {
                let props = if family == "ender_chest" {
                    format!("facing={facing}")
                } else {
                    format!("facing={facing},type={kind},waterlogged=false")
                };
                let state = parse_state(&format!("minecraft:{family}[{props}]")).unwrap();
                let parts = bank.geometry_at(&state, (9, 64, -19)).unwrap();
                assert_eq!(parts.len(), 3);
                assert_eq!(
                    (parts[0].bounds.min[1], parts[0].bounds.max[1]),
                    (-0.5, 0.125)
                );
                assert_eq!(
                    (parts[1].bounds.min[1], parts[1].bounds.max[1]),
                    (0.125, 0.4375)
                );
                let latch = center(&parts[2]);
                assert_eq!(latch[axis], sign * 0.46875, "{family} {kind} {facing}");
                let expected = format!(
                    "minecraft:entity/chest/{sheet}{}",
                    if kind == "single" {
                        String::new()
                    } else {
                        format!("_{kind}")
                    }
                );
                for face in [
                    Face::Top,
                    Face::Bottom,
                    Face::PosX,
                    Face::NegX,
                    Face::PosZ,
                    Face::NegZ,
                ] {
                    for index in 0..3 {
                        if let Some(spec) =
                            bank.models
                                .element_face_at(&state, face, index, (9, 64, -19))
                        {
                            assert_eq!(spec.texture, expected);
                            assert!(bank
                                .atlas_face_style_at(&state, face, index, (9, 64, -19))
                                .is_some());
                        }
                    }
                }
            }
        }
    }
    let state = parse_state("minecraft:chest[facing=south,type=single]").unwrap();
    let lid = bank
        .models
        .element_face_at(&state, Face::Top, 1, (0, 0, 0))
        .unwrap();
    assert_eq!(lid.uv, [7., 0., 10.5, 3.5]);
    assert_eq!(lid.rotation, 180);
    let left = parse_state("minecraft:chest[facing=south,type=left]").unwrap();
    let right = parse_state("minecraft:chest[facing=south,type=right]").unwrap();
    assert!(bank
        .models
        .element_face_at(&left, Face::NegX, 0, (0, 0, 0))
        .is_none());
    assert!(bank
        .models
        .element_face_at(&right, Face::PosX, 0, (0, 0, 0))
        .is_none());
    let latch = bank
        .models
        .element_face_at(&left, Face::Top, 2, (0, 0, 0))
        .unwrap();
    assert_eq!(latch.uv, [0.25, 0., 0.5, 0.25]);
    assert_eq!(latch.rotation, 0);
    let l = bank.geometry_at(&left, (1, 0, 0)).unwrap();
    let r = bank.geometry_at(&right, (0, 0, 0)).unwrap();
    assert_eq!(r[0].bounds.max[0] + 0.5, l[0].bounds.min[0] + 1.5);
    assert_eq!(r[2].bounds.max[0] + 0.5, l[2].bounds.min[0] + 1.5);
}

#[test]
fn all_bed_colors_parts_and_facings_keep_sheet_crops_and_source_height() {
    let f = fixture();
    let bank = f.open();
    for color in COLORS {
        for part in ["head", "foot"] {
            for (facing, axis, sign) in [
                ("north", 2, 1.),
                ("east", 0, -1.),
                ("south", 2, -1.),
                ("west", 0, 1.),
            ] {
                let state = parse_state(&format!(
                    "minecraft:{color}_bed[part={part},facing={facing},occupied=false]"
                ))
                .unwrap();
                let parts = bank.geometry_at(&state, (-2, 30, 19)).unwrap();
                assert_eq!(parts.len(), 3);
                assert_eq!(
                    (parts[0].bounds.min[1], parts[0].bounds.max[1]),
                    (-0.3125, 0.0625)
                );
                let leg = center(&parts[1]);
                assert_eq!(leg[1], -0.40625);
                assert_eq!(
                    leg[axis],
                    if part == "head" {
                        -sign * 0.40625
                    } else {
                        sign * 0.40625
                    }
                );
                let spec = bank
                    .models
                    .element_face_at(&state, Face::Top, 0, (-2, 30, 19))
                    .unwrap();
                assert_eq!(spec.texture, format!("minecraft:entity/bed/{color}"));
                assert_eq!(
                    spec.uv,
                    if part == "head" {
                        [1.5, 1.5, 5.5, 5.5]
                    } else {
                        [1.5, 7., 5.5, 11.]
                    }
                );
                assert!(bank
                    .atlas_face_style_at(&state, Face::Top, 0, (-2, 30, 19))
                    .is_some());
            }
        }
    }
    let head = parse_state("minecraft:red_bed[part=head,facing=north]").unwrap();
    let foot = parse_state("minecraft:red_bed[part=foot,facing=north]").unwrap();
    assert!(bank
        .models
        .element_face_at(&head, Face::PosZ, 0, (0, 0, 0))
        .is_none());
    assert!(bank
        .models
        .element_face_at(&foot, Face::NegZ, 0, (0, 0, 0))
        .is_none());
}

#[test]
fn explicit_models_empty_geometry_and_foreign_chests_are_not_overwritten() {
    for doc in [
        br#"{"elements":[]}"#.as_slice(),
        br#"{"loader":"neoforge:empty"}"#.as_slice(),
    ] {
        let f = fixture();
        f.put("assets/minecraft/models/block/chest.json", doc);
        let bank = f.open();
        let s = parse_state("minecraft:chest[facing=south,type=single]").unwrap();
        assert!(bank.geometry_at(&s, (0, 0, 0)).unwrap().is_empty());
    }
    let f = fixture();
    f.put(
        "assets/minecraft/blockstates/chest.json",
        br#"{"variants":{"":{"model":"demo:block/custom"}}}"#,
    );
    f.put(
        "assets/demo/blockstates/chest.json",
        br#"{"variants":{"":{"model":"minecraft:block/chest"}}}"#,
    );
    f.put("assets/demo/models/block/custom.json",br##"{"textures":{"all":"minecraft:entity/chest/ender"},"elements":[{"from":[2,4,6],"to":[8,10,12],"faces":{"up":{"texture":"#all"}}}]}"##);
    let bank = f.open();
    let custom = parse_state("minecraft:chest[facing=south,type=single]").unwrap();
    assert_eq!(bank.geometry_at(&custom, (0, 0, 0)).unwrap().len(), 1);
    let foreign = parse_state("demo:chest").unwrap();
    assert!(bank.geometry_at(&foreign, (0, 0, 0)).unwrap().is_empty());
}

#[test]
fn entity_texture_overlay_and_zip_match_directory_for_block_only_loading() {
    let f = fixture();
    let pixels = image::RgbaImage::from_pixel(64, 64, image::Rgba([77, 19, 199, 255]));
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, 64, 64, image::ExtendedColorType::Rgba8)
        .unwrap();
    let overlay = Fixture::new();
    overlay.put("assets/minecraft/textures/entity/chest/normal.png", &png);
    let bank = TextureBank::open_stack_java_controlled(
        &[f.path.clone(), overlay.path.clone()],
        &LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(bank.statistics.requested_png, 23);
    let s = parse_state("minecraft:chest[facing=south,type=single]").unwrap();
    let (uv, _, page) = bank
        .atlas_face_style_at(&s, Face::Top, 1, (0, 0, 0))
        .unwrap();
    let mid = std::array::from_fn(|i| uv.iter().map(|p| p[i]).sum::<f32>() / 4.);
    assert_eq!(bank.atlas_pixel(mid, page), Some([77, 19, 199, 255]));
    fn add(zip: &mut zip::ZipWriter<std::fs::File>, root: &Path, path: &Path) {
        for item in fs::read_dir(path).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                add(zip, root, &path);
            } else {
                zip.start_file(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                zip.write_all(&fs::read(path).unwrap()).unwrap();
            }
        }
    }
    let packed = Fixture::new();
    let path = packed.path.join("all.zip");
    let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
    add(&mut zip, &f.path, &f.path.join("assets"));
    zip.finish().unwrap();
    let zipped = TextureBank::open_stack_java_controlled(
        &[path, overlay.path.clone()],
        &LoadControl {
            all_png: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(zipped.statistics.requested_png, 23);
    let mut a = crate::textures::java_obj_tests::scene(Arc::new(bank));
    Arc::make_mut(&mut a.surface).voxels[0].state = s.into();
    let mut b = a.clone();
    b.texture_bank = Some(Arc::new(zipped));
    b.chunk_cache = Default::default();
    let a = crate::spectator::build_cancellable(&a, &|| false).unwrap();
    let b = crate::spectator::build_cancellable(&b, &|| false).unwrap();
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&a),
        bytemuck::cast_slice::<_, u8>(&b)
    );
}

#[test]
fn special_models_keep_geometry_and_source_picking_in_perspective_and_projection() {
    use iced::{Point, Size};
    let f = fixture();
    let bank = f.open();
    for name in [
        "minecraft:chest[facing=south,type=single]",
        "minecraft:chest[facing=east,type=left]",
        "minecraft:trapped_chest[facing=west,type=right]",
        "minecraft:ender_chest[facing=north]",
        "minecraft:red_bed[facing=north,part=head]",
        "minecraft:blue_bed[facing=south,part=foot]",
    ] {
        let state = parse_state(name).unwrap();
        let mut perspective = java_obj_tests::scene(Arc::clone(&bank));
        Arc::make_mut(&mut perspective.surface).voxels[0].state = state.clone().into();
        let mesh = crate::spectator::build_scene_cancellable(&perspective, &|| false).unwrap();
        let face_count = mesh.chunks.iter().map(|c| c.quads().len()).sum::<usize>();
        assert!(face_count > 6, "{name} 必须包含实际部件，而不是全方块");
        let hit = mesh
            .pick(
                perspective.camera,
                Point::new(64., 64.),
                crate::render_viewport::RenderViewport::one_to_one(Size::new(128., 128.)),
                perspective.quality,
            )
            .unwrap();
        let owner = hit.owner.unwrap();
        assert_eq!(owner.position, [0, 0, 0]);
        assert_eq!(*owner.state, state);
        perspective.show_textures = false;
        assert_eq!(
            crate::spectator::build_scene_cancellable(&perspective, &|| false)
                .unwrap()
                .chunks
                .iter()
                .map(|c| c.quads().len())
                .sum::<usize>(),
            face_count
        );
        let mut view = java_obj_tests::isometric_scene(Arc::clone(&bank));
        Arc::make_mut(&mut view.surface).voxels[0].state = state.into();
        let size = Size::new(128., 128.);
        let faces = view.faces_cached(size);
        assert!(!faces.is_empty());
        assert!(faces
            .iter()
            .all(|face| face.static_triangle.is_none() && face.element_index < 3));
        let center = Point::new(
            faces[0].corners.iter().map(|p| p.x).sum::<f32>() / 4.,
            faces[0].corners.iter().map(|p| p.y).sum::<f32>() / 4.,
        );
        assert_eq!(view.pick_face(center, size).unwrap().0, (0, 0, 0));
        assert_eq!(view.gpu_vertices_cached(size).len(), faces.len() * 6);
    }
    let mut pair = java_obj_tests::scene(bank);
    let voxels = &mut Arc::make_mut(&mut pair.surface).voxels;
    voxels[0].state = parse_state("minecraft:chest[facing=south,type=right]")
        .unwrap()
        .into();
    let mut left = voxels[0].clone();
    left.x = 1;
    left.state = parse_state("minecraft:chest[facing=south,type=left]")
        .unwrap()
        .into();
    voxels.push(left);
    let mesh = crate::spectator::build_scene_cancellable(&pair, &|| false).unwrap();
    for x in [0, 1] {
        let mut camera = pair.camera;
        camera.position[0] = x as f64;
        let hit = mesh
            .pick(
                camera,
                Point::new(64., 64.),
                crate::render_viewport::RenderViewport::one_to_one(Size::new(128., 128.)),
                pair.quality,
            )
            .unwrap();
        assert_eq!(hit.owner.unwrap().position, [x, 0, 0]);
    }
}
