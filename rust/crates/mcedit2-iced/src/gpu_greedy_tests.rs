//! 在既有 GPU 大类中验证整套平面合并，不另建测试入口。
use super::*;
use crate::{
    spectator::{CameraUniform, MeshChunk, StreamMesh},
    textures::RuntimeTexture,
    world_mesh::WorldQuad,
};
pub(super) fn run() {
    let mut reference = GpuProbe::with_options(wgpu::Limits::default(), Size::new(192, 160), false);
    let mut merged = GpuProbe::with_size(wgpu::Limits::default(), Size::new(192, 160));
    reference.pipeline.material_hints = false;
    let anchor = [-30_000_000, 64, 30_000_000];
    for lattice in [0.0, -0.5] {
        for width in [16, 4096] {
            for quality in [Quality::Fast, Quality::Detailed] {
                for rotation in [0, 90, 180, 270] {
                    for mirror in [false, true] {
                        if width > 16 && (rotation != 0 || mirror) {
                            continue;
                        }
                        let textures = (0..3).map(|kind| {
                            let rgba = (0..width * 16)
                                .flat_map(|i| {
                                    let x = i % width;
                                    let y = i / width;
                                    [
                                        ((x * 13 + kind * 29) % 256) as u8,
                                        ((y * 13 + kind * 47) % 256) as u8,
                                        ((x * 7 + y * 11) % 256) as u8,
                                        if kind != 0 && (x / 4 + y / 4) % 2 == 0 {
                                            0
                                        } else {
                                            255
                                        },
                                    ]
                                })
                                .collect();
                            RuntimeTexture {
                                sampler: None,
                                id: format!("merge-{kind}"),
                                width,
                                height: 16,
                                rgba,
                            }
                        });
                        let atlas = TextureAtlas::empty_test_atlas()
                            .append_runtime_cancellable(textures, &|| false)
                            .unwrap();
                        let mut parts = Vec::new();
                        let mut input_faces = 0;
                        for part in 0..2 {
                            let origin = [anchor[0] + part * 8, anchor[1], anchor[2]];
                            let mut quads = Vec::new();
                            for y in 0..8 {
                                for x in 0..8 {
                                    let global_x = part * 8 + x;
                                    if y == 2 && (global_x == 6 || global_x == 7) {
                                        continue;
                                    }
                                    let kind = if y < 3 {
                                        0
                                    } else if y < 6 {
                                        1
                                    } else {
                                        2
                                    };
                                    let tile = &atlas.entries[&format!("merge-{kind}")];
                                    let mut uv = tile.corners_for(
                                        if global_x == 11 {
                                            [0., 0., 8., 16.]
                                        } else {
                                            [0., 0., 16., 16.]
                                        },
                                        rotation,
                                    );
                                    if mirror {
                                        uv = [uv[1], uv[0], uv[3], uv[2]];
                                    }
                                    if global_x >= 12 && (y < 2 || y >= 6) {
                                        uv = [tile.sample(0.25, 0.75); 4];
                                    }
                                    let mut quad = WorldQuad {
                                        xyz: [
                                            [x as f32, y as f32, 4.],
                                            [x as f32 + 1., y as f32, 4.],
                                            [x as f32 + 1., y as f32 + 1., 4.],
                                            [x as f32, y as f32 + 1., 4.],
                                        ],
                                        uv,
                                        rgba: [[1.; 4]; 4],
                                        mode: if kind == 0 {
                                            0
                                        } else if kind == 1 {
                                            1
                                        } else {
                                            1 | 32 | crate::material_mode::NO_MIPMAP
                                        },
                                        atlas_page: tile.page,
                                        light: crate::lighting::FULLBRIGHT,
                                    };
                                    quad.mode |= crate::material_mode::SINGLE_SIDED;
                                    quad.classify_axis_facing();
                                    quads.push(quad);
                                    let mut back = quad;
                                    back.reverse_winding();
                                    back.uv.rotate_right(1);
                                    back.classify_axis_facing();
                                    quads.push(back);
                                    if global_x == 3 && y == 3 {
                                        let mut overlay = quad;
                                        overlay.mode |= crate::material_mode::ORDERED_OPAQUE;
                                        overlay.rgba = [[0.3, 0.8, 1., 1.]; 4];
                                        quads.push(overlay);
                                        overlay.rgba = [[0.9, 0.4, 0.6, 1.]; 4];
                                        quads.push(overlay);
                                    }
                                    if global_x == 5 && y == 1 {
                                        let mut overlay = quad;
                                        overlay.mode = crate::material_mode::ORDERED_OPAQUE;
                                        overlay.xyz = overlay.xyz.map(|p| {
                                            [
                                                x as f32 + 0.25 + (p[0] - x as f32) * 0.5,
                                                y as f32 + 0.25 + (p[1] - y as f32) * 0.5,
                                                4.,
                                            ]
                                        });
                                        overlay.rgba = [[1., 0.1, 0.2, 1.]; 4];
                                        quads.push(overlay);
                                    }
                                }
                            }
                            // 大平面和原有孔洞 / 裁剪 / 叠层共同构成一份场景，验证合并的成本门槛与真实覆盖。
                            let tile = &atlas.entries["merge-0"];
                            let mut uv = tile.corners_for([0., 0., 16., 16.], rotation);
                            if mirror {
                                uv = [uv[1], uv[0], uv[3], uv[2]];
                            }
                            for y in 0..8 {
                                for x in 0..8 {
                                    let mut quad = WorldQuad {
                                        xyz: [
                                            [x as f32, y as f32, 8.],
                                            [x as f32 + 1., y as f32, 8.],
                                            [x as f32 + 1., y as f32 + 1., 8.],
                                            [x as f32, y as f32 + 1., 8.],
                                        ],
                                        uv: if part * 8 + x >= 12 {
                                            [tile.sample(0.25, 0.75); 4]
                                        } else {
                                            uv
                                        },
                                        rgba: [[1.; 4]; 4],
                                        mode: crate::material_mode::SINGLE_SIDED,
                                        atlas_page: tile.page,
                                        light: crate::lighting::FULLBRIGHT,
                                    };
                                    quad.classify_axis_facing();
                                    quads.push(quad);
                                    quad.reverse_winding();
                                    quad.uv.rotate_right(1);
                                    quad.classify_axis_facing();
                                    quads.push(quad);
                                }
                            }
                            for quad in &mut quads {
                                for point in &mut quad.xyz {
                                    point[0] += lattice;
                                    point[1] += lattice;
                                }
                            }
                            input_faces += quads
                                .iter()
                                .filter(|q| !q.blended() && !q.ordered_opaque())
                                .count();
                            parts.push(MeshChunk::new(
                                (part, 0, 0, 0, 0),
                                origin,
                                Arc::new(quads),
                                Default::default(),
                            ));
                        }
                        let mesh = Arc::new(StreamMesh {
                            terrain: None,
                            visibility: None,
                            entities: None,
                            complete: true,
                            origin: anchor,
                            atlas: Some(Arc::clone(&atlas)),
                            chunks: parts.into(),
                        });
                        for (reverse, orthographic, pan) in [
                            (false, true, 0.0),
                            (false, false, 0.0),
                            (true, false, 0.0),
                            (false, false, 0.1),
                        ] {
                            let camera = CameraUniform {
                                position: [
                                    8. + pan,
                                    4.,
                                    if reverse { 28. } else { -20. },
                                    f32::from(orthographic),
                                ],
                                right: [if reverse { -1. } else { 1. }, 0., 0., 0.],
                                up: [0., 1., 0., 0.],
                                forward: [0., 0., if reverse { -1. } else { 1. }, 0.],
                                projection: if orthographic {
                                    [0.1, 0.2, 0.05, 100.]
                                } else {
                                    [1.25, 1.5, 0.05, 100.]
                                },
                            };
                            let primitive = Primitive {
                                lighting: crate::lighting::Table::unlit(),
                                light_report: std::sync::Weak::new(),
                                frame_tick: None,
                                vertices: Arc::new(Vec::new()),
                                atlas: Some(Arc::clone(&atlas)),
                                quality,
                                camera: Some(camera),
                                source_origin: Some(anchor),
                                stream_mesh: Some(Arc::clone(&mesh)),
                                presentation: None,
                                iso_presentation: None,
                            };
                            reference.pixel_primitive(primitive.clone());
                            merged.pixel_primitive(primitive);
                            let maximum = reference
                                .pixels
                                .iter()
                                .zip(&merged.pixels)
                                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)))
                                .max()
                                .unwrap();
                            if maximum > 1 {
                                let directory =
                                    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                                        .join("../../target");
                                for (name, pixels) in [
                                    ("greedy-reference.png", &reference.pixels),
                                    ("greedy-actual.png", &merged.pixels),
                                ] {
                                    image::save_buffer(
                                        directory.join(name),
                                        bytemuck::cast_slice(pixels),
                                        192,
                                        160,
                                        image::ColorType::Rgba8,
                                    )
                                    .unwrap();
                                }
                                let points = reference
                                    .pixels
                                    .iter()
                                    .zip(&merged.pixels)
                                    .enumerate()
                                    .filter(|(_, (a, b))| a != b)
                                    .take(12)
                                    .map(|(i, (a, b))| (i % 192, i / 192, *a, *b))
                                    .collect::<Vec<_>>();
                                eprintln!(
                                    "[gpu-greedy] 差异像素 {}，前几个 {points:?}",
                                    reference
                                        .pixels
                                        .iter()
                                        .zip(&merged.pixels)
                                        .filter(|(a, b)| a != b)
                                        .count()
                                );
                            }
                            assert!(maximum<=1,"重复材质、覆盖或顺序改变：{quality:?} / lattice={lattice} / width={width} / {rotation} / mirror={mirror} / reverse={reverse} / ortho={orthographic} / pan={pan} / max={maximum}");
                        }
                        let output_faces = merged
                            .pipeline
                            .stream_regions
                            .values()
                            .flat_map(|r| &r.buffers.segments)
                            .map(|s| s.spans().last().map_or(0, |s| s.end as usize))
                            .sum::<usize>();
                        if super::super::commands::native_indirect(&merged.device) {
                            assert!(
                                output_faces < input_faces,
                                "合并未减少单位平面的 GPU 实例：{input_faces}->{output_faces}"
                            );
                        } else {
                            assert_eq!(output_faces, input_faces, "无原生批量能力时保留单位面");
                        }
                        assert!(mesh.chunks.iter().all(|part| merged
                            .pipeline
                            .stream_chunks
                            .values()
                            .any(|r| Arc::ptr_eq(&r.source, &part.quads()))));
                    }
                }
            }
        }
    }
}
