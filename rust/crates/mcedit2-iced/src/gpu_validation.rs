//! 使用生产 prepare/draw 路径离屏绘制并回读像素，需要可用的本地 GPU。
use super::*;
use iced::futures::executor::block_on;
use iced::widget::shader::{Pipeline as _, Primitive as _};
use std::time::Duration;

const EDGE: u32 = 16;
const ROW_BYTES: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_animated_materials_keep_geometry_and_shared_atlas() {
    let (_fixture, bank) = crate::textures::animation_tests::fixture();
    let atlas = bank.atlas().unwrap();
    let rect = &atlas.entries["demo:live"];
    let mut vertices = quad(0.9, [0.2, 0.4, 0.6, 1.], 0);
    for v in &mut vertices {
        v.uv = atlas.white.sample(0.5, 0.5);
        v.atlas_page = atlas.white.page;
    }
    let mut foreground = quad(0.5, [1.; 4], 2);
    for v in &mut foreground {
        v.uv = rect.sample(0.5, 0.5);
        v.atlas_page = rect.page;
    }
    vertices.extend(foreground);
    let vertices = Arc::new(vertices);
    let mut gpu = GpuProbe::new();
    let mut first_texture = None;
    for quality in [Quality::Fast, Quality::Balanced, Quality::Detailed] {
        for (tick, color) in [
            (0, [255, 0, 0, 255]),
            (1, [127, 0, 127, 255]),
            (2, [51, 102, 153, 255]),
            (3, [51, 102, 153, 255]),
            (4, [255, 0, 0, 255]),
        ] {
            let snapshot = Arc::new(atlas.snapshot_at(tick).unwrap());
            let primitive = Primitive {
                lighting: crate::lighting::Table::unlit(),
                light_report: std::sync::Weak::new(),
                frame_tick: None,
                vertices: Arc::clone(&vertices),
                atlas: Some(Arc::clone(&snapshot)),
                quality,
                camera: None,
                source_origin: None,
                stream_mesh: None,
                presentation: None,
                iso_presentation: None,
            };
            assert_pixel(gpu.pixel_primitive(primitive), color);
            let active = gpu.pipeline.animation_frame.as_ref().unwrap();
            let owner = snapshot
                .mip_page(rect.page)
                .unwrap()
                .owner_at(rect.sample(0.5, 0.5))
                .unwrap()
                .unwrap();
            assert_eq!(
                active.samples.keys().copied().collect::<Vec<_>>(),
                vec![(rect.page, owner)]
            );
            let cpu = crate::canvas_raster::render(
                &vertices,
                Some(&snapshot),
                iced::Size::new(EDGE as f32, EDGE as f32),
                quality,
                &|| false,
            )
            .unwrap()
            .unwrap();
            let at = ((EDGE / 2 * EDGE + EDGE / 2) * 4) as usize;
            assert_pixel(cpu.rgba[at..at + 4].try_into().unwrap(), color);
            let texture = &gpu.pipeline.atlas_pages[rect.page as usize].0;
            if let Some(previous) = &first_texture {
                assert_eq!(texture, previous, "动画不能重新分配整包 GPU 图集");
            } else {
                first_texture = Some(texture.clone());
            }
            assert!(Arc::ptr_eq(
                gpu.pipeline.last_source_mesh.as_ref().unwrap(),
                &vertices
            ));
        }
    }
    let mut second = <Pipeline as shader::Pipeline>::new(&gpu.device, &gpu.queue, COLOR_FORMAT);
    let snapshot = Arc::new(atlas.snapshot_at(2).unwrap());
    let primitive = Primitive {
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
        frame_tick: None,
        vertices: Arc::clone(&vertices),
        atlas: Some(snapshot),
        quality: Quality::Fast,
        camera: None,
        source_origin: None,
        stream_mesh: None,
        presentation: None,
        iso_presentation: None,
    };
    let viewport = Viewport::with_physical_size(gpu.size, 1.);
    primitive.prepare(
        &mut second,
        &gpu.device,
        &gpu.queue,
        &Rectangle::with_size(viewport.logical_size()),
        &viewport,
    );
    assert!(
        Arc::ptr_eq(
            second.atlas_pages[0].2.shared.as_ref().unwrap(),
            gpu.pipeline.atlas_pages[0].2.shared.as_ref().unwrap()
        ),
        "同设备的两个动画视口必须共享纹理上传"
    );
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
}
// 大型实例的验收可显式选择工作预算，与桌面资源面板的输入语义一致。
fn installed_bank(paths: &[std::path::PathBuf]) -> crate::textures::TextureBank {
    let budget = std::env::var("MCEDIT2_TEST_RESOURCE_MIB")
        .ok()
        .map(|value| {
            crate::resource_budget::ResourceBudget::from_input(&value)
                .expect("测试资源预算必须是有效 MiB")
        });
    crate::textures::TextureBank::open_stack_java_controlled(
        paths,
        &crate::textures::LoadControl {
            budget,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_world_orthographic_reuses_resident_mesh_and_matches_cpu_projection() {
    let fixture = crate::textures::java_obj_tests::fixture();
    let scene = crate::textures::java_obj_tests::scene(fixture.open());
    let mesh = Arc::new(crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    *scene.cache.lock().unwrap() = Some(Arc::clone(&mesh));
    let size = Size::new(160, 128);
    let logical = Size::new(160., 128.);
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), size);
    let framing = crate::orthographic_world::Cache::default();
    for scale in [1., 1.25, 1.5, 2.] {
        gpu.scale_factor = scale;
        let logical = Viewport::with_physical_size(size, scale).logical_size();
        let drawing_framing = if scale == 1. {
            Arc::clone(&framing)
        } else {
            crate::orthographic_world::Cache::default()
        };
        let mut baseline = Vec::new();
        for (yaw, zoom, pan) in [
            (0_f32, 1., iced::Vector::ZERO),
            (45., 2., iced::Vector::new(17., -11.)),
            (0., 1., iced::Vector::ZERO),
        ] {
            let view = crate::orthographic_world::View {
                angles: (yaw.to_radians(), 35_f32.to_radians()),
                navigation: crate::view_navigation::Viewport { zoom, pan },
                framing: Arc::clone(&drawing_framing),
            };
            let projection = view.projection(&mesh, logical).unwrap().unwrap();
            let primitive = shader::Program::draw(
                &GpuScene(SceneSource::WorldOrthographic(scene.clone(), view)),
                &State::default(),
                mouse::Cursor::Unavailable,
                Rectangle::with_size(logical),
            );
            assert!(
                primitive.vertices.is_empty(),
                "世界等轴不得展开整份世界顶点"
            );
            assert!(Arc::ptr_eq(primitive.stream_mesh.as_ref().unwrap(), &mesh));
            let uniform = primitive.camera.unwrap();
            assert_eq!(uniform.position[3], 1.);
            gpu.pixel_primitive(primitive);
            assert!(gpu.pixels.iter().filter(|p| scene_pixel(p)).count() > 100);
            for resident in gpu.pipeline.stream_chunks.values() {
                assert!(mesh
                    .chunks
                    .iter()
                    .any(|c| Arc::ptr_eq(&c.quads(), &resident.source)));
            }
            if baseline.is_empty() {
                baseline = gpu.pixels.clone();
            } else if yaw == 0. {
                assert_eq!(gpu.pixels, baseline);
            }
            let mut quads = mesh
                .chunks
                .iter()
                .flat_map(|c| c.quads().iter().map(move |q| (c.origin, q)))
                .collect::<Vec<_>>();
            quads.sort_by(|(a, x), (b, y)| {
                let depth = |origin: [i32; 3], q: &crate::world_mesh::WorldQuad| {
                    projection
                        .project(std::array::from_fn(|i| {
                            origin[i] as f64 + q.center()[i] as f64
                        }))
                        .1
                };
                depth(*a, x).total_cmp(&depth(*b, y))
            });
            let mut vertices = Vec::new();
            let mut hits = 0;
            for (origin, quad) in quads {
                let point = projection
                    .project(std::array::from_fn(|i| {
                        origin[i] as f64 + quad.center()[i] as f64
                    }))
                    .0;
                if let Some(hit) = mesh.pick(
                    projection,
                    point,
                    crate::render_viewport::RenderViewport::at_scale(
                        logical,
                        scale,
                        gpu.device.limits().max_texture_dimension_2d,
                    ),
                    scene.quality,
                ) {
                    assert_eq!(hit.owner.unwrap().position, [0, 0, 0]);
                    hits += 1;
                }
                for mut vertex in quad.expanded() {
                    let (point, depth) = projection.project(std::array::from_fn(|i| {
                        origin[i] as f64 + vertex.xyz[i] as f64
                    }));
                    vertex.xy = [
                        2. * point.x / logical.width - 1.,
                        1. - 2. * point.y / logical.height,
                    ];
                    vertex.depth = (-depth - uniform.projection[2])
                        / (uniform.projection[3] - uniform.projection[2]);
                    vertex.world_mode = 0;
                    vertices.push(vertex);
                }
            }
            assert!(hits > 0);
            let cpu = crate::canvas_raster::render(
                &vertices,
                mesh.atlas.as_deref(),
                Size::new(size.width as f32, size.height as f32),
                scene.quality,
                &|| false,
            )
            .unwrap()
            .unwrap();
            let mismatches = cpu
                .rgba
                .chunks_exact(4)
                .zip(&gpu.pixels)
                .filter(|(cpu, gpu)| cpu.iter().zip(gpu.iter()).any(|(a, b)| a.abs_diff(*b) > 2))
                .count();
            assert!(
                mismatches <= 16,
                "世界等轴 GPU / CPU 材质或深度不一致：{mismatches} 个像素"
            );
        }
    }
    gpu.scale_factor = 1.;
    // 模拟下一批在原深度区间之外提交几何，沿同一观察方向保持屏幕位置。
    let mut later_scene = scene.clone();
    let mut later_source = (*scene.surface).clone();
    later_source.z += 10;
    later_source.y_min += 7;
    later_source.y_max += 7;
    for voxel in &mut later_source.voxels {
        voxel.y += 7;
        voxel.z += 10;
    }
    later_scene.surface = Arc::new(later_source);
    later_scene.cache = Default::default();
    later_scene.chunk_cache = Default::default();
    let later_mesh =
        Arc::new(crate::spectator::build_scene_cancellable(&later_scene, &|| false).unwrap());
    *later_scene.cache.lock().unwrap() = Some(Arc::clone(&later_mesh));
    let view = crate::orthographic_world::View {
        angles: (0., 35_f32.to_radians()),
        navigation: Default::default(),
        framing,
    };
    let projection = view.projection(&later_mesh, logical).unwrap().unwrap();
    let primitive = shader::Program::draw(
        &GpuScene(SceneSource::WorldOrthographic(
            later_scene.clone(),
            view.clone(),
        )),
        &State::default(),
        mouse::Cursor::Unavailable,
        Rectangle::with_size(logical),
    );
    assert!(Arc::ptr_eq(
        primitive.stream_mesh.as_ref().unwrap(),
        &later_mesh
    ));
    gpu.pixel_primitive(primitive);
    assert!(gpu.pixels.iter().filter(|p| scene_pixel(p)).count() > 100);
    let mut picked = 0;
    for chunk in &later_mesh.chunks {
        for quad in chunk.quads().iter() {
            let point = projection
                .project(std::array::from_fn(|i| {
                    chunk.origin[i] as f64 + quad.center()[i] as f64
                }))
                .0;
            if let Some(hit) = later_mesh.pick(
                projection,
                point,
                crate::render_viewport::RenderViewport::one_to_one(logical),
                scene.quality,
            ) {
                assert_eq!(hit.owner.unwrap().position, [0, 7, 10]);
                picked += 1;
            }
        }
    }
    assert!(picked > 0);
    // 同一 GPU 类别继续验证 f64 焦点重定位与像素平移等价，材质驻留源不变。
    let mut app = crate::App {
        surface_exposed: true,
        surface_pitch: 35,
        world_orthographic_framing: Arc::clone(&view.framing),
        ..Default::default()
    };
    let action = crate::view_navigation::Action::Pan(iced::Vector::new(17., -11.));
    let mut expected_view = view;
    expected_view.navigation.apply(action).unwrap();
    let expected = shader::Program::draw(
        &GpuScene(SceneSource::WorldOrthographic(
            later_scene.clone(),
            expected_view,
        )),
        &State::default(),
        mouse::Cursor::Unavailable,
        Rectangle::with_size(logical),
    );
    gpu.pixel_primitive(expected);
    let expected_pixels = gpu.pixels.clone();
    app.navigate_world_orthographic(action).unwrap();
    let rebased = shader::Program::draw(
        &GpuScene(SceneSource::WorldOrthographic(
            later_scene,
            app.world_orthographic_view(),
        )),
        &State::default(),
        mouse::Cursor::Unavailable,
        Rectangle::with_size(logical),
    );
    assert!(Arc::ptr_eq(
        rebased.stream_mesh.as_ref().unwrap(),
        &later_mesh
    ));
    gpu.pixel_primitive(rebased);
    assert_eq!(gpu.pixels, expected_pixels);
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_perspective_zoom_reuses_resident_source_mesh() {
    let fixture = crate::textures::java_obj_tests::fixture();
    let mut scene = crate::textures::java_obj_tests::scene(fixture.open());
    scene.camera.position = [0., 0., -5.];
    let mesh = Arc::new(crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    *scene.cache.lock().unwrap() = Some(Arc::clone(&mesh));
    let size = Size::new(128, 128);
    let logical = Size::new(128., 128.);
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), size);
    let mut baseline = Vec::new();
    let mut baseline_visible = 0;
    for zoom in [1., 2., 1.] {
        scene.camera.zoom = zoom;
        let primitive = shader::Program::draw(
            &GpuScene(SceneSource::Spectator(scene.clone())),
            &State::default(),
            mouse::Cursor::Unavailable,
            Rectangle::with_size(logical),
        );
        assert!(Arc::ptr_eq(primitive.stream_mesh.as_ref().unwrap(), &mesh));
        gpu.pixel_primitive(primitive);
        let visible = gpu.pixels.iter().filter(|p| scene_pixel(p)).count();
        if baseline.is_empty() {
            baseline = gpu.pixels.clone();
            baseline_visible = visible;
            assert!(visible > 0);
        } else if zoom == 2. {
            assert!(
                visible > baseline_visible * 2,
                "放大后覆盖像素应增加：{baseline_visible} -> {visible}"
            );
        } else {
            assert_eq!(gpu.pixels, baseline, "缩回原倍率后应恢复同一实际画面");
        }
        for resident in gpu.pipeline.stream_chunks.values() {
            assert!(
                mesh.chunks
                    .iter()
                    .any(|c| Arc::ptr_eq(&c.quads(), &resident.source)),
                "镜头缩放不能复制或替换源几何"
            );
        }
    }
    // 夹具是偏离中心的真实组合模型；从实际几何像素取一条已验证的源射线。
    let (pixel, owner) = baseline
        .iter()
        .enumerate()
        .find_map(|(index, rgba)| {
            if !scene_pixel(rgba) {
                return None;
            }
            let pixel = iced::Point::new(
                (index as u32 % size.width) as f32 + 0.5,
                (index as u32 / size.width) as f32 + 0.5,
            );
            let hit = mesh.pick(
                scene.camera,
                pixel,
                crate::render_viewport::RenderViewport::one_to_one(logical),
                scene.quality,
            )?;
            Some((pixel, hit.owner?.position))
        })
        .expect("实际模型像素必须能追溯到源方块");
    for scale in [1.25, 1.5, 2.] {
        gpu.scale_factor = scale;
        let logical = Viewport::with_physical_size(size, scale).logical_size();
        let primitive = shader::Program::draw(
            &GpuScene(SceneSource::Spectator(scene.clone())),
            &State::default(),
            mouse::Cursor::Unavailable,
            Rectangle::with_size(logical),
        );
        gpu.pixel_primitive(primitive);
        assert_eq!(gpu.pixels, baseline, "DPI 改变不能把较小栅格放大成模糊图像");
        assert_eq!(gpu.pipeline.offscreen_size, (size.width, size.height));
        let shown = scene.presented_view.lock().unwrap().clone().unwrap();
        assert_eq!(shown.viewport.logical, logical);
        assert_eq!(shown.viewport.raster, size);
        let hit = mesh
            .pick(
                shown.camera,
                iced::Point::new(pixel.x / scale, pixel.y / scale),
                shown.viewport,
                shown.quality,
            )
            .unwrap();
        assert_eq!(hit.owner.unwrap().position, owner);
    }
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_vanilla_special_models_real_client_pixels() {
    use mcedit2_world::editing::parse_state;
    let path = std::path::PathBuf::from(
        std::env::var_os("MCEDIT2_CLIENT_JAR").expect("设置现代原版 MCEDIT2_CLIENT_JAR"),
    );
    let bank = Arc::new(
        crate::textures::TextureBank::open_stack_java_controlled(
            &[path],
            &crate::textures::LoadControl {
                all_png: false,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let mut states = Vec::new();
    for name in ["chest", "trapped_chest", "ender_chest"] {
        for facing in ["north", "east", "south", "west"] {
            let kinds: &[&str] = if name == "ender_chest" {
                &["single"]
            } else {
                &["single", "left", "right"]
            };
            for kind in kinds {
                let suffix = if name == "ender_chest" {
                    String::new()
                } else {
                    format!(",type={kind}")
                };
                states.push(
                    parse_state(&format!("minecraft:{name}[facing={facing}{suffix}]")).unwrap(),
                );
            }
        }
    }
    for color in [
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
    ] {
        for part in ["head", "foot"] {
            for facing in ["north", "east", "south", "west"] {
                states.push(
                    parse_state(&format!(
                        "minecraft:{color}_bed[part={part},facing={facing}]"
                    ))
                    .unwrap(),
                );
            }
        }
    }
    for color in crate::textures::shulker_tests::COLORS {
        let (block, _) = crate::textures::shulker_tests::names(color);
        for facing in ["up", "down", "north", "south", "west", "east"] {
            states.push(parse_state(&format!("minecraft:{block}[facing={facing}]")).unwrap());
        }
    }
    assert_eq!(states.len(), 258);
    assert_source_models_render(Arc::clone(&bank), &states);
    assert_isometric_models_render(bank, &states);
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_uv_lock_root_models_and_source_picking() {
    let fixture = crate::resource_models::uv_lock_tests::fixture();
    let bank = fixture.open();
    let states = crate::resource_models::uv_lock_tests::states();
    assert_source_models_render(Arc::clone(&bank), &states);
    assert_isometric_models_render(bank, &states);
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_canvas_raster_matches_gpu_material_and_depth_pixels() {
    use crate::canvas_raster::{
        self,
        tests::{fixture, quad, texture_quad},
    };
    let pixels = image::RgbaImage::from_fn(16, 16, |x, y| {
        image::Rgba([
            7 + x as u8 * 13,
            12 + y as u8 * 10,
            17 + ((x + y) % 16) as u8 * 11,
            if y % 4 == 0 { 0 } else { x as u8 * 17 },
        ])
    });
    let fixture = fixture(16, 16, &pixels);
    let bank = fixture.open();
    let textured = texture_quad(&bank);
    let mut gpu = GpuProbe::new();
    for scale in [1., 1.25, 1.5, 2.] {
        gpu.scale_factor = scale;
        for quality in [Quality::Fast, Quality::Balanced, Quality::Detailed] {
            for mode in [
                8,
                1,
                1 | 32,
                2,
                crate::material_mode::encode(2, Some("MASK"), 0.75, true),
            ] {
                let mut vertices = quad([0.13, 0.23, 0.37, 1.], 0.9, 0).to_vec();
                let white = &bank.atlas().unwrap().white;
                for v in &mut vertices {
                    v.uv = white.sample(0.5, 0.5);
                    v.atlas_page = white.page;
                }
                let mut foreground = textured;
                let start = foreground[0].uv;
                for v in &mut foreground {
                    v.mode = mode;
                    v.depth = 0.5;
                    for (coordinate, start) in v.uv.iter_mut().zip(start) {
                        *coordinate = start + (*coordinate - start) * 0.83 + 0.001;
                    }
                }
                vertices.extend(foreground);
                let mut intersecting = quad([0.2, 0.7, 0.3, 0.35], 0.3, 2);
                for (v, z) in intersecting.iter_mut().zip([0.1, 0.9, 0.9, 0.1, 0.9, 0.1]) {
                    v.depth = z;
                    v.uv = white.sample(0.5, 0.5);
                    v.atlas_page = white.page;
                }
                vertices.extend(intersecting);
                let cpu = canvas_raster::render(
                    &vertices,
                    bank.atlas().map(Arc::as_ref),
                    Size::new(EDGE as f32, EDGE as f32),
                    quality,
                    &|| false,
                )
                .unwrap()
                .unwrap();
                gpu.pixel_primitive(Primitive {
                    lighting: crate::lighting::Table::unlit(),
                    light_report: std::sync::Weak::new(),
                    frame_tick: None,
                    vertices: Arc::new(vertices),
                    atlas: bank.atlas().cloned(),
                    quality,
                    camera: None,
                    source_origin: None,
                    stream_mesh: None,
                    presentation: None,
                    iso_presentation: None,
                });
                for (index, (expected, actual)) in
                    cpu.rgba.chunks_exact(4).zip(&gpu.pixels).enumerate()
                {
                    for (a, b) in expected.iter().zip(actual) {
                        assert!(a.abs_diff(*b)<=2,"CPU / GPU 像素差异 quality={quality:?},mode={mode},pixel={index}: {expected:?} != {actual:?}");
                    }
                }
            }
        }
    }
}

/// 绘制、纹理像素差异及等轴源拾取共用验证，适用于实际客户端和诊断资源包。
#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_full_instance_bed_canvas_gpu_pixels() {
    let path = std::env::var_os("MCEDIT2_TEST_WORLD").expect("设置 MCEDIT2_TEST_WORLD");
    let world = mcedit2_world::inspect_world(std::path::PathBuf::from(path)).unwrap();
    let stack = crate::auto_assets::discover_stack(
        &world.root,
        world.version_name.as_deref().unwrap_or(""),
        &[],
    )
    .unwrap()
    .unwrap();
    let bank = Arc::new(installed_bank(&stack.paths));
    assert_eq!(
        bank.block_name("minecraft:red_bed", crate::language::Language::Chinese),
        Some("红色床")
    );
    assert_eq!(
        bank.block_name(
            "modular_machinery_reborn:biome_reader",
            crate::language::Language::Chinese
        ),
        Some("群系读取器"),
        "富文本提示键不能丢弃同一模组文件中的普通方块名称"
    );
    eprintln!(
        "完整实例：{} PNG / {} 模型 / {} 页",
        bank.count(),
        bank.model_count(),
        bank.atlas().unwrap().page_count()
    );
    let size = Size::new(694, 654);
    let logical = Size::new(size.width as f32, size.height as f32);
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), size);
    // 走与桌面相同的消息、实际源读取、窗口相机和 shader Program 交接。
    let mut app = crate::App {
        worlds: vec![world],
        workspace: {
            let mut state = crate::workspace_tabs::State::default();
            state.open(0);
            state
        },
        current: Some(0),
        spectator: true,
        surface_area: "128,13,84;129,13,84".into(),
        ..Default::default()
    };
    let _ = crate::update_impl(&mut app, crate::Message::InspectVolume);
    let job = app.near_job.take().unwrap();
    let result = job.read();
    app.near_loaded(job.nonce, job.key, result);
    app.texture_bank = Some(Arc::clone(&bank));
    app.language = crate::language::Language::Chinese;
    app.query_coordinates = "128,13,84".into();
    let _ = crate::update_impl(&mut app, crate::Message::InspectBlock);
    let description = app
        .block_name_info()
        .expect("实际资源语言表应提供原版红床名称");
    assert!(description.starts_with("名称：红色床\n") && description.contains("minecraft:red_bed"));
    let mut scene = crate::textures::java_obj_tests::scene(Arc::clone(&bank));
    scene.surface = Arc::clone(&app.surface.as_ref().unwrap().2);
    assert_eq!(scene.surface.voxels.len(), 2);
    scene.camera = app.spectator_camera;
    let mesh = Arc::new(crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    assert!(
        mesh.pick(
            scene.camera,
            iced::Point::new(logical.width * 0.5, logical.height * 0.5),
            crate::render_viewport::RenderViewport::one_to_one(logical),
            scene.quality
        )
        .is_some(),
        "自动构图的中心射线应命中真实床"
    );
    *scene.cache.lock().unwrap() = Some(mesh);
    let primitive = shader::Program::draw(
        &GpuScene(SceneSource::Spectator(scene)),
        &State::default(),
        mouse::Cursor::Unavailable,
        Rectangle::with_size(logical),
    );
    gpu.pixel_primitive(primitive);
    let visible = gpu.pixels.iter().filter(|p| scene_pixel(p)).count();
    assert!(
        visible > 100,
        "实际窗口相机只画出天空：{visible} 个非背景像素"
    );
    for part in ["head", "foot"] {
        let mut view = crate::textures::java_obj_tests::isometric_scene(Arc::clone(&bank));
        view.yaw_degrees = 0;
        Arc::make_mut(&mut view.surface).voxels[0].state = mcedit2_world::editing::parse_state(
            &format!("minecraft:red_bed[facing=east,part={part},occupied=false]"),
        )
        .unwrap()
        .into();
        let vertices = view.gpu_vertices_cached(logical);
        for quality in [Quality::Balanced, Quality::Detailed] {
            let cpu = crate::canvas_raster::render(
                &vertices,
                bank.atlas().map(Arc::as_ref),
                logical,
                quality,
                &|| false,
            )
            .unwrap()
            .unwrap();
            gpu.pixel_primitive(Primitive {
                lighting: crate::lighting::Table::unlit(),
                light_report: std::sync::Weak::new(),
                frame_tick: None,
                vertices: Arc::clone(&vertices),
                atlas: bank.atlas().cloned(),
                quality,
                camera: None,
                source_origin: None,
                stream_mesh: None,
                presentation: None,
                iso_presentation: None,
            });
            let differences = cpu
                .rgba
                .chunks_exact(4)
                .zip(&gpu.pixels)
                .filter(|(a, b)| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 2))
                .count();
            if differences > 16 {
                let out = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
                image::save_buffer(
                    out.join(format!("bed-cpu-{part}-{quality:?}.png")),
                    &cpu.rgba,
                    size.width,
                    size.height,
                    image::ColorType::Rgba8,
                )
                .unwrap();
                image::save_buffer(
                    out.join(format!("bed-gpu-{part}-{quality:?}.png")),
                    bytemuck::cast_slice(&gpu.pixels),
                    size.width,
                    size.height,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            assert!(
                differences <= 16,
                "{part} {quality:?}: {differences} 个像素差异超过 2/255"
            );
        }
    }
}

fn assert_isometric_models_render(
    bank: Arc<crate::textures::TextureBank>,
    states: &[mcedit2_world::sections::State],
) {
    let edge = 128;
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), Size::new(edge, edge));
    let size = Size::new(edge as f32, edge as f32);
    let primitive = |vertices, quality| Primitive {
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
        frame_tick: None,
        vertices,
        atlas: bank.atlas().cloned(),
        quality,
        camera: None,
        source_origin: None,
        stream_mesh: None,
        presentation: None,
        iso_presentation: None,
    };
    gpu.pixel_primitive(primitive(Arc::new(Vec::new()), Quality::Fast));
    let background = gpu.pixels.clone();
    for state in states {
        let mut view = crate::textures::java_obj_tests::isometric_scene(Arc::clone(&bank));
        view.gpu_faces = true;
        Arc::make_mut(&mut view.surface).voxels[0].state = state.clone().into();
        let vertices = view.gpu_vertices_cached(size);
        gpu.pixel_primitive(primitive(vertices, Quality::Fast));
        let textured = gpu.pixels.clone();
        let hit = textured
            .iter()
            .zip(&background)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .find_map(|(i, _)| {
                let point = iced::Point::new(
                    (i % edge as usize) as f32 + 0.5,
                    (i / edge as usize) as f32 + 0.5,
                );
                view.pick_face(point, size)
            })
            .unwrap_or_else(|| panic!("等轴 GPU 可见区域没有命中源方块：{state:?}"));
        assert_eq!(hit.0, (0, 0, 0));
        // 与 ToggleTextures 消息一致：新的样式视图使用新的投影 / GPU 缓存。
        let mut plain = crate::textures::java_obj_tests::isometric_scene(Arc::clone(&bank));
        Arc::make_mut(&mut plain.surface).voxels[0].state = state.clone().into();
        plain.show_textures = false;
        gpu.pixel_primitive(primitive(plain.gpu_vertices_cached(size), Quality::Fast));
        assert!(
            textured.iter().zip(&gpu.pixels).any(|(a, b)| a != b),
            "等轴材质没有影响实际像素 {:?}",
            state
        );
    }
    eprintln!("{} 组模型的等轴 GPU 材质和源方块拾取均通过", states.len());
}

/// 复用实际 prepare/draw 和像素回读，核对已读取资源包的模型确实进入 GPU。
pub(crate) fn assert_source_models_render(
    bank: Arc<crate::textures::TextureBank>,
    states: &[mcedit2_world::sections::State],
) {
    fn primitive(
        view: &crate::spectator::SpectatorScene,
        mesh: Arc<crate::spectator::StreamMesh>,
    ) -> Primitive {
        Primitive {
            lighting: crate::lighting::Table::unlit(),
            light_report: std::sync::Weak::new(),
            frame_tick: None,
            vertices: Arc::new(Vec::new()),
            atlas: mesh.atlas.clone(),
            quality: Quality::Fast,
            camera: Some(
                view.camera
                    .uniform_at_origin(mesh.origin, Size::new(EDGE as f32, EDGE as f32)),
            ),
            source_origin: Some(mesh.origin),
            stream_mesh: Some(mesh),
            presentation: None,
            iso_presentation: None,
        }
    }
    let mut gpu = GpuProbe::new();
    for state in states {
        let mut view = crate::textures::java_obj_tests::scene(Arc::clone(&bank));
        Arc::make_mut(&mut view.surface).voxels[0].state = state.clone().into();
        let mesh = Arc::new(crate::spectator::build_scene_cancellable(&view, &|| false).unwrap());
        let faces = mesh
            .chunks
            .iter()
            .map(|chunk| chunk.quads().len())
            .sum::<usize>();
        let shape = mesh_shape(&mesh);
        assert!(faces > 0, "原生模型没有生成可绘制源面：{}", state.name);
        // 半个双箱的拼接侧本来就是敞开的；按实际范围从上方斜看，不能假定正北必有面。
        let (minimum, maximum) = mesh.chunks.world_bounds().unwrap();
        let center = std::array::from_fn::<_, 3, _>(|i| (minimum[i] + maximum[i]) * 0.5);
        let extent = (0..3)
            .map(|i| maximum[i] - minimum[i])
            .fold(0.0_f64, f64::max);
        view.camera.yaw = -35.0;
        view.camera.pitch = 25.0;
        let (_, _, forward) = view.camera.basis();
        view.camera.position =
            std::array::from_fn(|i| center[i] - f64::from(forward[i]) * (extent * 3.0 + 1.0));
        let empty = Arc::new(crate::spectator::StreamMesh {
            terrain: None,
            visibility: None,
            entities: None,
            complete: true,
            origin: mesh.origin,
            atlas: mesh.atlas.clone(),
            chunks: vec![].into(),
        });
        gpu.pixel_primitive(primitive(&view, empty));
        let background = gpu.pixels.clone();
        gpu.pixel_primitive(primitive(&view, mesh));
        let textured = gpu.pixels.clone();
        let visible = textured
            .iter()
            .zip(&background)
            .filter(|(p, bg)| p != bg)
            .count();
        assert!(
            visible > 0,
            "GPU 没有显示实际模型：{} {:?}",
            state.name,
            state.properties
        );
        view.show_textures = false;
        let plain = Arc::new(crate::spectator::build_scene_cancellable(&view, &|| false).unwrap());
        // 跨纹理边界会细分源面，关闭纹理不需要这些切线。比较外形和总面积，
        // 不能把合法细分数不同误报为几何丢失；实际材质仍须改变 GPU 像素。
        let plain_shape = mesh_shape(&plain);
        for (a, b) in shape.0.into_iter().zip(plain_shape.0) {
            assert!(
                (a - b).abs() < 0.000_1,
                "纹理开关改变源范围：{}",
                state.name
            );
        }
        assert!(
            (shape.1 - plain_shape.1).abs() < 0.000_1 * shape.1.max(1.),
            "纹理开关改变源面积：{}",
            state.name
        );
        gpu.pixel_primitive(primitive(&view, plain));
        let material_pixels = textured
            .iter()
            .zip(&gpu.pixels)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            material_pixels > 0,
            "实际纹理应改变 GPU 像素，不能只通过天空或纯色模型：{}",
            state.name
        );
        eprintln!(
            "真实 GPU {} {:?}：{faces} 源面 / {visible} 几何像素 / {material_pixels} 材质差异像素",
            state.name, state.properties
        );
    }
}

fn mesh_shape(mesh: &crate::spectator::StreamMesh) -> ([f64; 6], f64) {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut area = 0.;
    for chunk in &mesh.chunks {
        for quad in chunk.quads().iter() {
            for point in quad.xyz {
                for axis in 0..3 {
                    let p = point[axis] as f64 + chunk.origin[axis] as f64;
                    bounds[axis] = bounds[axis].min(p);
                    bounds[axis + 3] = bounds[axis + 3].max(p);
                }
            }
            for [a, b, c] in crate::world_mesh::TRIANGLES {
                let u: [f64; 3] =
                    std::array::from_fn(|i| quad.xyz[b][i] as f64 - quad.xyz[a][i] as f64);
                let v: [f64; 3] =
                    std::array::from_fn(|i| quad.xyz[c][i] as f64 - quad.xyz[a][i] as f64);
                let cross = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                area += cross.into_iter().map(|v| v * v).sum::<f64>().sqrt() * 0.5;
            }
        }
    }
    (bounds, area)
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_isometric_java_obj_materials_and_picking_pixels() {
    let fixture = crate::textures::java_obj_tests::fixture();
    let bank = fixture.open();
    let mut view = crate::textures::java_obj_tests::isometric_scene(Arc::clone(&bank));
    view.gpu_faces = true;
    let edge = 128;
    let size = Size::new(edge as f32, edge as f32);
    let vertices = view.gpu_vertices_cached(size);
    assert_eq!(vertices.len(), 18);
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), Size::new(edge, edge));
    gpu.pixel_primitive(Primitive {
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
        frame_tick: None,
        vertices: Arc::clone(&vertices),
        atlas: bank.atlas().cloned(),
        quality: Quality::Fast,
        camera: None,
        source_origin: None,
        stream_mesh: None,
        presentation: None,
        iso_presentation: None,
    });
    let mut red = 0;
    let mut picked_red = 0;
    let mut blue = 0;
    for (i, pixel) in gpu.pixels.iter().enumerate() {
        if pixel[0] > 150 && pixel[1] < 10 && pixel[2] < 10 {
            red += 1;
            assert_eq!(pixel[3], 255, "等轴 solid 应忽略材质与顶点 Alpha");
            let point = iced::Point::new(
                (i % edge as usize) as f32 + 0.5,
                (i / edge as usize) as f32 + 0.5,
            );
            if let Some((pos, _)) = view.pick_face(point, size) {
                assert_eq!(pos, (0, 0, 0));
                picked_red += 1;
            }
        }
        if u16::from(pixel[2]) > u16::from(pixel[0]) + 10
            && u16::from(pixel[2]) > u16::from(pixel[1]) + 10
        {
            blue += 1;
            assert!(
                pixel[3] > 0 && pixel[3] < 255,
                "等轴 blend 应保留实际 Alpha"
            );
        }
    }
    assert!(red > 0 && picked_red > 0, "红色 OBJ 应可见且命中源方块");
    assert!(blue > 0, "等轴应显示透明蓝色 OBJ");
    // 同一生产网格交替使用两个独立图集页，避免同页夹具掩盖错误的三角形批次。
    let mut atlas = TextureAtlas::empty_test_atlas();
    atlas.pages = [[255, 0, 0, 255], [0, 0, 255, 255]]
        .map(|rgba| {
            Arc::new(crate::textures::AtlasPage {
                width: 2,
                height: 2,
                rgba: rgba.repeat(4),
            })
        })
        .into();
    atlas.rebuild_raw_test_mips();
    let atlas = Arc::new(atlas);
    let mut remapped = (*vertices).clone();
    for face in remapped.chunks_exact_mut(6) {
        let page = u32::from(face[0].mode & 3 == 2);
        for vertex in face {
            vertex.atlas_page = page;
            vertex.uv = [0.5; 2];
        }
    }
    let remapped = Arc::new(remapped);
    for quality in [Quality::Fast, Quality::Detailed] {
        gpu.pixel_primitive(Primitive {
            lighting: crate::lighting::Table::unlit(),
            light_report: std::sync::Weak::new(),
            frame_tick: None,
            vertices: Arc::clone(&remapped),
            atlas: Some(Arc::clone(&atlas)),
            quality,
            camera: None,
            source_origin: None,
            stream_mesh: None,
            presentation: None,
            iso_presentation: None,
        });
        assert!(gpu
            .pixels
            .iter()
            .any(|p| p[0] > 150 && p[1] < 10 && p[2] < 10));
        assert!(
            gpu.pixels
                .iter()
                .any(|p| u16::from(p[2]) > u16::from(p[0]) + 10),
            "切换图集页后仍应显示蓝色三角形，采样 {:?}",
            quality
        );
    }
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_java_obj_composite_textures_colors_and_picking_pixels() {
    let fixture = crate::textures::java_obj_tests::fixture();
    let bank = fixture.open();
    let view = crate::textures::java_obj_tests::scene(Arc::clone(&bank));
    let mesh = Arc::new(crate::spectator::build_scene_cancellable(&view, &|| false).unwrap());
    assert_eq!(mesh.chunks[0].quads().len(), 3);
    let mut gpu = GpuProbe::new();
    gpu.pixel_primitive(Primitive {
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
        frame_tick: None,
        vertices: Arc::new(Vec::new()),
        atlas: mesh.atlas.clone(),
        quality: Quality::Fast,
        camera: Some(
            view.camera
                .uniform_at_origin(mesh.origin, Size::new(EDGE as f32, EDGE as f32)),
        ),
        source_origin: Some(mesh.origin),
        stream_mesh: Some(Arc::clone(&mesh)),
        presentation: None,
        iso_presentation: None,
    });
    assert!(
        gpu.pixels
            .iter()
            .any(|p| p[0] > 150 && p[1] < 10 && p[2] < 10),
        "反射后的红色 OBJ 组件应可见"
    );
    assert!(
        gpu.pixels
            .iter()
            .any(|p| u16::from(p[2]) > u16::from(p[0]) + 10
                && u16::from(p[2]) > u16::from(p[1]) + 10),
        "透明蓝色 OBJ 组件应可见"
    );
    for (i, pixel) in gpu.pixels.iter().enumerate() {
        if pixel[0] > 150 && pixel[1] < 10 && pixel[2] < 10 {
            assert_eq!(
                pixel[3], 255,
                "solid 声明必须忽略 Kd / 顶点 Alpha 并写入完整覆盖"
            );
            let point = iced::Point::new(
                (i % EDGE as usize) as f32 + 0.5,
                (i / EDGE as usize) as f32 + 0.5,
            );
            let hit = mesh
                .pick(
                    view.camera,
                    point,
                    crate::render_viewport::RenderViewport::one_to_one(Size::new(
                        EDGE as f32,
                        EDGE as f32,
                    )),
                    Quality::Fast,
                )
                .unwrap();
            assert_eq!(hit.owner.unwrap().state.name, "demo:assembly");
        }
    }
}

struct GpuProbe {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: Pipeline,
    target: wgpu::Texture,
    readback: wgpu::Buffer,
    pixels: Vec<[u8; 4]>,
    size: Size<u32>,
    scale_factor: f32,
    row_bytes: u32,
    timestamp_supported: bool,
    adapter_info: String,
}

/// 使用与目标视图相同的尺寸、采样和编码策略测量实际 GPU 通道。
pub(crate) struct WorldProfile {
    pub size: Size<u32>,
    pub quality: Quality,
    pub y32: bool,
    pub multiple_views: bool,
    pub material_hints: bool,
    pub section_culling: bool,
    /// 首个稳定视角的完整回读，供同一材质下的剔除开关作精确像素比较。
    pub reference_image: Option<std::path::PathBuf>,
}
impl Default for WorldProfile {
    fn default() -> Self {
        Self {
            size: Size::new(694, 555),
            quality: Quality::Fast,
            y32: true,
            multiple_views: false,
            material_hints: true,
            section_culling: true,
            reference_image: None,
        }
    }
}
pub(crate) fn profile_world(
    mesh: Arc<crate::spectator::StreamMesh>,
    camera: crate::spectator::Camera,
    settings: WorldProfile,
) -> serde_json::Value {
    let WorldProfile {
        size,
        quality,
        y32,
        multiple_views,
        material_hints,
        section_culling,
        reference_image,
    } = settings;
    let mesh = if section_culling {
        mesh
    } else {
        let mut reference = (*mesh).clone();
        reference.visibility = None;
        Arc::new(reference)
    };
    let logical = Size::new(size.width as f32, size.height as f32);
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), size);
    gpu.pipeline.opaque_y32 = y32;
    gpu.pipeline.material_hints = material_hints;
    if !gpu.timestamp_supported {
        return serde_json::json!({"timestamp_supported":false,"reason":"当前验收设备未启用 TIMESTAMP_QUERY"});
    }
    let primitive = Primitive {
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
        frame_tick: None,
        vertices: Arc::new(Vec::new()),
        atlas: mesh.atlas.clone(),
        quality,
        camera: Some(camera.uniform_at_origin(mesh.origin, logical)),
        source_origin: Some(mesh.origin),
        stream_mesh: Some(mesh),
        presentation: None,
        iso_presentation: None,
    };
    let started = std::time::Instant::now();
    let reference_pixel = gpu.pixel_primitive(primitive.clone());
    let upload = started.elapsed();
    // 完整画面先稳定，再以同一生产渲染路径批量提交，不在每帧等待 GPU。
    let one = gpu.profile_frames(&primitive, camera, 1);
    assert!(
        one["frames"][0]["composite_ms"]
            .as_f64()
            .is_some_and(|value| value > 0.),
        "单通道时间戳无效：{one}"
    );
    let cameras = if multiple_views {
        vec![
            camera,
            crate::spectator::Camera { zoom: 1., ..camera },
            crate::spectator::Camera {
                zoom: 1.,
                pitch: -45.,
                ..camera
            },
            crate::spectator::Camera {
                zoom: 1.,
                pitch: 60.,
                ..camera
            },
        ]
    } else {
        vec![camera]
    };
    let mut views = Vec::new();
    for (index, cam) in cameras.into_iter().enumerate() {
        let mut p = primitive.clone();
        p.camera = Some(cam.uniform_at_origin(p.source_origin.unwrap(), logical));
        // 先等待同一生产通道稳定；像素摘要必须包含已完成的空域剔除，而非任务交接帧。
        let _ = gpu.profile_frames(&p, cam, 180);
        let _ = gpu.profile_frames(&p, cam, 1);
        gpu.pixel_primitive(p.clone());
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        gpu.pixels.hash(&mut hash);
        // 导出本来就已回读的真实世界画面，便于检查材质排布；不增加渲染或测试用例。
        if multiple_views {
            let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/gpu-profile-images");
            std::fs::create_dir_all(&directory).expect("无法创建 GPU 画面目录");
            let path = directory.join(format!(
                "{}-{index}.png",
                if y32 && !material_hints {
                    "unhinted"
                } else if y32 {
                    "y32"
                } else {
                    "baseline"
                }
            ));
            image::save_buffer(
                &path,
                bytemuck::cast_slice(&gpu.pixels),
                gpu.size.width,
                gpu.size.height,
                image::ColorType::Rgba8,
            )
            .expect("无法保存已有 GPU 回读画面");
        }
        // 两种入口都先稳定 GPU 负载与管线，再恢复固定视角；不把冷启动频率混进通道计时。
        let mut view = gpu.profile_frames(&p, cam, 18);
        if index == 0 {
            if let Some(path) = &reference_image {
                image::save_buffer(
                    path,
                    bytemuck::cast_slice(&gpu.pixels),
                    gpu.size.width,
                    gpu.size.height,
                    image::ColorType::Rgba8,
                )
                .expect("无法写出稳定视角回读");
            }
        }
        view["camera"] = serde_json::json!({"position":cam.position,"yaw":cam.yaw,"pitch":cam.pitch,"zoom":cam.zoom});
        view["pixel_hash"] = serde_json::json!(format!("{:016x}", hash.finish()));
        views.push(view);
    }
    let final_pixel = gpu.pixel_primitive(primitive);
    assert_eq!(
        reference_pixel, final_pixel,
        "时间戳采样不得改变同一视角像素"
    );
    let mut result = if multiple_views {
        serde_json::json!({"views":views})
    } else {
        views.remove(0)
    };
    let segments = gpu
        .pipeline
        .stream_regions
        .values()
        .flat_map(|r| r.buffers.segments.iter())
        .chain(
            gpu.pipeline
                .stream_chunks
                .values()
                .flat_map(|r| r.ordered.segments.iter()),
        );
    let (hinted_faces, opaque_faces) =
        segments.fold((0usize, 0usize), |(hinted, total), segment| {
            (
                hinted + segment.hinted_faces,
                total + segment.spans().last().map_or(0, |s| s.end as usize),
            )
        });
    result["material_hinted_faces"] = serde_json::json!(hinted_faces);
    result["resident_opaque_faces"] = serde_json::json!(opaque_faces);
    result["material_hints"] = serde_json::json!(material_hints);
    result["reference_pixel"] = serde_json::json!(reference_pixel);
    result["initial_upload_ms"] = serde_json::json!(upload.as_secs_f64() * 1000.);
    result["resolution"] = serde_json::json!([size.width, size.height]);
    result["quality"] = serde_json::json!(quality.label());
    result["adapter"] = serde_json::json!(gpu.adapter_info);
    result["debug_assertions"] = serde_json::json!(cfg!(debug_assertions));
    result["instance_flags"] = serde_json::json!(format!("{:?}", wgpu::InstanceFlags::default()));
    result
}

impl GpuProbe {
    fn profile_frames(
        &mut self,
        primitive: &Primitive,
        camera: crate::spectator::Camera,
        count: u32,
    ) -> serde_json::Value {
        if !self.timestamp_supported {
            return serde_json::json!({"timestamp_supported":false});
        }
        let query = self.device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("离屏实际绘制时间戳"),
            ty: wgpu::QueryType::Timestamp,
            count: count * 6,
        });
        let bytes = u64::from(count * 6 * 8);
        let resolve = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("时间戳解析"),
            size: bytes,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("时间戳回读"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let viewport = Viewport::with_physical_size(self.size, self.scale_factor);
        let size = viewport.logical_size();
        let mut cpu = Vec::new();
        let mut issued = Vec::new();
        let mut indices = Vec::new();
        let mut used = 0u32;
        for index in 0..count {
            let mut p = primitive.clone();
            let mut cam = camera;
            if (6..12).contains(&index) {
                cam.position[0] += f64::from(index - 5) * 0.1;
            }
            if index >= 12 {
                cam.yaw += ((index - 12) % 6 + 1) as f32 * 0.5;
            }
            p.camera = Some(cam.uniform_at_origin(p.source_origin.unwrap(), size));
            self.pipeline.profile_pass_issued = false;
            self.pipeline.timestamps = Some((query.clone(), used));
            let start = std::time::Instant::now();
            p.prepare(
                &mut self.pipeline,
                &self.device,
                &self.queue,
                &Rectangle::with_size(size),
                &viewport,
            );
            let prepared = start.elapsed();
            issued.push(self.pipeline.profile_pass_issued);
            let world_index = self.pipeline.profile_pass_issued.then_some(used);
            if world_index.is_some() {
                used += 4;
            }
            let composite_index = used;
            used += 2;
            indices.push((world_index, composite_index));
            let view = self
                .target
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("基准生产合成通道"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                        query_set: &query,
                        beginning_of_pass_write_index: Some(composite_index),
                        end_of_pass_write_index: Some(composite_index + 1),
                    }),
                    occlusion_query_set: None,
                });
                assert!(p.draw(&self.pipeline, &mut pass));
            }
            self.queue.submit(Some(encoder.finish()));
            let submit_total_us = start.elapsed().as_micros();
            let mut opaque_faces = 0u64;
            let mut draws = 0usize;
            let mut submitted_draws = 0usize;
            let mut buffers = 0usize;
            let mut patch_count = 0usize;
            let mut group_count = 0usize;
            let view = self.pipeline.profile_view.as_ref();
            for resident in self.pipeline.stream_regions.values() {
                buffers += resident.buffers.segments.len();
                let delta = origin_delta(
                    resident.source.origin,
                    p.stream_mesh.as_ref().unwrap().origin,
                );
                let local = view.map(|view| view.at_offset([delta[0], delta[1], delta[2], 0.]));
                if local.as_ref().is_some_and(|view| {
                    resident
                        .bounds
                        .as_ref()
                        .is_some_and(|bounds| !bounds.visible_in(view))
                }) {
                    continue;
                }
                submitted_draws += resident.commands.draw_count();
                for segment in &resident.buffers.segments {
                    patch_count += segment.patches().len();
                    group_count += segment.visibility_group_count();
                    for span in segment.spans() {
                        for (start, end) in segment.visible_ranges(span, local.as_ref()) {
                            draws += 1;
                            opaque_faces += u64::from(end - start);
                        }
                    }
                }
            }
            let (transparent_faces, tripwire_faces) = self
                .pipeline
                .world_sorted
                .segments
                .iter()
                .flat_map(|segment| segment.visible_spans(view))
                .fold((0u64, 0u64), |(transparent, tripwire), span| {
                    let count = u64::from(span.end - span.start);
                    if span.tripwire {
                        (transparent, tripwire + count)
                    } else {
                        (transparent + count, tripwire)
                    }
                });
            // 首个深度 / 颜色 pass 还包含绊线；分别保留几何计数，避免混称普通透明面。
            let (range_builds, selection_reuses) = self
                .pipeline
                .stream_regions
                .values()
                .map(|r| &r.commands)
                .chain(
                    self.pipeline
                        .stream_chunks
                        .values()
                        .map(|r| &r.ordered_commands),
                )
                .chain(
                    self.pipeline
                        .world_sorted
                        .segments
                        .iter()
                        .flat_map(|s| [&s.commands, &s.tripwire_commands]),
                )
                .fold((0usize, 0usize), |(builds, reused), c| {
                    (builds + c.range_builds, reused + c.selection_reuses)
                });
            cpu.push(serde_json::json!({"prepare_us":prepared.as_micros(),"section_prepare_us":if self.pipeline.profile_pass_issued {self.pipeline.section_visibility.stats.micros} else {0},"section_reused":self.pipeline.section_visibility.stats.reused,"section_candidates":self.pipeline.section_visibility.stats.visible,"section_total":self.pipeline.section_visibility.stats.total,"gap_ranges":self.pipeline.section_visibility.stats.gap_ranges,"submit_total_us":submit_total_us,"command_range_builds_total":range_builds,"command_selection_reuses_total":selection_reuses,"opaque_faces":opaque_faces,"transparent_faces":transparent_faces,"tripwire_faces":tripwire_faces,"opaque_buffers":buffers,"opaque_ranges":draws,"opaque_draws":submitted_draws,"opaque_patches":patch_count,"opaque_culling_groups":group_count}));
        }
        self.pipeline.timestamps = None;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        if used > 0 {
            encoder.resolve_query_set(&query, 0..used, &resolve, 0);
        }
        encoder.copy_buffer_to_buffer(&resolve, 0, &read, 0, bytes);
        self.queue.submit(Some(encoder.finish()));
        let (send, recv) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            send.send(r).unwrap();
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(60)),
            })
            .unwrap();
        recv.recv().unwrap().unwrap();
        let data = read.slice(..).get_mapped_range();
        let timestamps: &[u64] = bytemuck::cast_slice(&data);
        let period = f64::from(self.queue.get_timestamp_period());
        let frames = (0..count as usize).map(|i| { let (world, composite)=indices[i]; let duration = |at: u32| timestamps[at as usize+1].saturating_sub(timestamps[at as usize]) as f64 * period / 1e6; serde_json::json!({"phase":if i < 6 {"fixed"} else if i < 12 {"translation"} else {"rotation"},"world_pass_issued":issued[i],"opaque_ms":world.map(duration),"transparent_ms":world.map(|at| duration(at+2)),"composite_ms":duration(composite),"cpu":cpu[i]}) }).collect::<Vec<_>>();
        drop(data);
        read.unmap();
        serde_json::json!({"timestamp_supported":true,"timestamp_period_ns":period,"frames":frames})
    }
    fn new() -> Self {
        Self::with_limits(wgpu::Limits::default())
    }
    fn with_limits(limits: wgpu::Limits) -> Self {
        Self::with_size(limits, Size::new(EDGE, EDGE))
    }
    fn with_size(limits: wgpu::Limits, size: Size<u32>) -> Self {
        Self::with_options(limits, size, true)
    }
    fn with_options(limits: wgpu::Limits, size: Size<u32>, indirect: bool) -> Self {
        // 与 iced 真实窗口设备一致，不能使用更宽的离屏限制掩盖启动失败。
        let limits = wgpu::Limits {
            max_bind_groups: 2,
            ..limits
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU 验收需要可用的 wgpu 适配器");
        eprintln!("GPU 像素验收适配器：{:?}", adapter.get_info());
        let adapter_info = format!("{:?}", adapter.get_info());
        let timestamp_supported = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let mut requested =
            if std::env::var_os("MCEDIT2_GPU_PROFILE").is_some() && timestamp_supported {
                wgpu::Features::TIMESTAMP_QUERY
            } else {
                wgpu::Features::empty()
            };
        if indirect
            && adapter
                .get_downlevel_capabilities()
                .flags
                .contains(wgpu::DownlevelFlags::INDIRECT_EXECUTION)
        {
            requested |= adapter.features()
                & (wgpu::Features::INDIRECT_FIRST_INSTANCE
                    | wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);
        }
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: limits,
            required_features: requested,
            ..Default::default()
        }))
        .expect("创建 GPU 验收设备失败");
        eprintln!("GPU 验收设备能力：{:?}", device.features());
        let timestamp_supported = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let mut pipeline = Pipeline::new(&device, &queue, COLOR_FORMAT);
        pipeline.opaque_y32 = true;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("GPU 验收像素输出"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let row_bytes = (size.width * 4).div_ceil(ROW_BYTES) * ROW_BYTES;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU 验收像素回读"),
            size: u64::from(row_bytes * size.height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            device,
            queue,
            pipeline,
            target,
            readback,
            pixels: Vec::new(),
            size,
            scale_factor: 1.,
            row_bytes,
            timestamp_supported,
            adapter_info,
        }
    }

    fn pixel(&mut self, vertices: Vec<Vertex>) -> [u8; 4] {
        let primitive = Primitive {
            lighting: crate::lighting::Table::unlit(),
            light_report: std::sync::Weak::new(),
            frame_tick: None,
            vertices: Arc::new(vertices),
            atlas: None,
            quality: Quality::Fast,
            camera: None,
            source_origin: None,
            stream_mesh: None,
            presentation: None,
            iso_presentation: None,
        };
        self.pixel_primitive(primitive)
    }

    fn pixel_primitive(&mut self, primitive: Primitive) -> [u8; 4] {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let pixel = self.pixel_once(&primitive);
            if self.pipeline.pending.is_none() {
                return pixel;
            }
            assert!(std::time::Instant::now() < deadline, "分帧 GPU 上传未完成");
        }
    }
    fn cursor_pixel<const N: usize>(
        &mut self,
        primitive: &crate::brush_cursor::CursorPrimitive<N>,
        pipeline: &mut crate::brush_cursor::CursorPipeline,
    ) -> [u8; 4] {
        let viewport = Viewport::with_physical_size(self.size, self.scale_factor);
        primitive.prepare(
            pipeline,
            &self.device,
            &self.queue,
            &Rectangle::with_size(viewport.logical_size()),
            &viewport,
        );
        // 使用光标正式准备的独立流水线，回读同一生产合成通道。
        let inner = pipeline.inner.as_mut().unwrap();
        std::mem::swap(&mut self.pipeline, inner);
        let pixel = self.pixel_primitive(primitive.0.clone());
        std::mem::swap(&mut self.pipeline, inner);
        pixel
    }
    fn pixel_once(&mut self, primitive: &Primitive) -> [u8; 4] {
        let viewport = Viewport::with_physical_size(self.size, self.scale_factor);
        primitive.prepare(
            &mut self.pipeline,
            &self.device,
            &self.queue,
            &Rectangle::with_size(viewport.logical_size()),
            &viewport,
        );
        let target = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("GPU 验收生产合成通道"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            assert!(primitive.draw(&self.pipeline, &mut pass));
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.row_bytes),
                    rows_per_image: Some(self.size.height),
                },
            },
            wgpu::Extent3d {
                width: self.size.width,
                height: self.size.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        self.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender.send(result).expect("GPU 回读接收端应保持存活");
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(15)),
            })
            .expect("等待 GPU 绘制失败或超时");
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("GPU 未返回映射结果")
            .expect("GPU 像素缓冲区映射失败");
        let bytes = self.readback.slice(..).get_mapped_range();
        let start = (self.size.height / 2 * self.row_bytes + self.size.width / 2 * 4) as usize;
        let pixel = bytes[start..start + 4].try_into().unwrap();
        self.pixels = bytes
            .chunks_exact(self.row_bytes as usize)
            .flat_map(|row| {
                row[..self.size.width as usize * 4]
                    .chunks_exact(4)
                    .map(|pixel| pixel.try_into().unwrap())
            })
            .collect();
        drop(bytes);
        self.readback.unmap();
        pixel
    }
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_compact_instances_match_expanded_pixels_across_segments_and_samplers() {
    use crate::world_mesh::WorldQuad;
    let mut gpu = GpuProbe::with_limits(wgpu::Limits {
        max_buffer_size: 8192,
        ..Default::default()
    });
    let mut atlas = TextureAtlas::empty_test_atlas();
    atlas.pages = vec![
        Arc::new(crate::textures::AtlasPage {
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        }),
        Arc::new(crate::textures::AtlasPage {
            width: 2,
            height: 2,
            rgba: vec![
                255, 255, 255, 255, 40, 120, 240, 255, 240, 120, 40, 255, 120, 240, 40, 255,
            ],
        }),
    ];
    atlas.rebuild_raw_test_mips();
    let atlas = Arc::new(atlas);
    let base = WorldQuad {
        xyz: [
            [-1.3, -1.3, 3.0],
            [1.3, -1.3, 3.0],
            [1.3, 1.3, 3.8],
            [-1.3, 1.3, 3.0],
        ],
        uv: [[0.15, 0.15], [0.85, 0.15], [0.85, 0.85], [0.15, 0.85]],
        rgba: [
            [1.0, 0.5, 0.8, 1.0],
            [0.5, 1.0, 0.8, 1.0],
            [0.8, 0.5, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ],
        mode: 1,
        atlas_page: 0,
        light: crate::lighting::FULLBRIGHT,
    };
    let mut triangle = base;
    triangle.xyz = [
        [-0.8, -0.3, 1.7],
        [0.8, -0.3, 1.7],
        [0.0, 0.9, 1.7],
        [0.0, 0.9, 1.7],
    ];
    triangle.uv[3] = triangle.uv[2];
    triangle.rgba[3] = triangle.rgba[2];
    triangle.mode = 0;
    triangle.atlas_page = 1;
    let mut blend = base;
    blend.xyz = blend.xyz.map(|mut p| {
        p[2] -= 1.0;
        p
    });
    blend.rgba = [
        [0.5, 0.8, 1.0, 0.3],
        [1.0, 0.3, 0.8, 0.5],
        [0.8, 1.0, 0.3, 0.7],
        [1.0, 0.8, 0.5, 0.9],
    ];
    blend.mode = 2;
    blend.atlas_page = 1;
    let mut faces = vec![base, triangle, blend];
    for i in 0..120 {
        let mut face = base.translated([100.0 + i as f32 * 4.0, 0.0, 0.0]);
        face.mode = if i % 2 == 0 { 0 } else { 2 };
        face.atlas_page = (i / 2) % 2;
        faces.push(face);
    }
    let faces = Arc::new(faces);
    let expanded = Arc::new(
        faces
            .iter()
            .flat_map(WorldQuad::expanded)
            .collect::<Vec<_>>(),
    );
    let mesh = Arc::new(crate::spectator::StreamMesh {
        terrain: None,
        visibility: None,
        entities: None,
        complete: true,
        origin: [0; 3],
        atlas: Some(Arc::clone(&atlas)),
        chunks: vec![crate::spectator::MeshChunk::new(
            (0, 0, 1, 0, 0),
            [0; 3],
            Arc::clone(&faces),
            Default::default(),
        )]
        .into(),
    });
    for quality in [Quality::Fast, Quality::Detailed] {
        for reverse in [false, true] {
            let camera = crate::spectator::CameraUniform {
                position: if reverse {
                    [0.0, 0.0, 6.0, 0.0]
                } else {
                    [0.0; 4]
                },
                right: if reverse {
                    [-1.0, 0.0, 0.0, 0.0]
                } else {
                    [1.0, 0.0, 0.0, 0.0]
                },
                up: [0.0, 1.0, 0.0, 0.0],
                forward: if reverse {
                    [0.0, 0.0, -1.0, 0.0]
                } else {
                    [0.0, 0.0, 1.0, 0.0]
                },
                projection: [1.0, 1.0, 0.05, 100.0],
            };
            let primitive = Primitive {
                lighting: crate::lighting::Table::unlit(),
                light_report: std::sync::Weak::new(),
                frame_tick: None,
                vertices: Arc::clone(&expanded),
                atlas: Some(Arc::clone(&atlas)),
                quality,
                camera: Some(camera),
                source_origin: Some([0; 3]),
                stream_mesh: None,
                presentation: None,
                iso_presentation: None,
            };
            gpu.pixel_primitive(primitive);
            let expected = gpu.pixels.clone();
            let primitive = Primitive {
                lighting: crate::lighting::Table::unlit(),
                light_report: std::sync::Weak::new(),
                frame_tick: None,
                vertices: Arc::new(Vec::new()),
                atlas: Some(Arc::clone(&atlas)),
                quality,
                camera: Some(camera),
                source_origin: Some([0; 3]),
                stream_mesh: Some(Arc::clone(&mesh)),
                presentation: None,
                iso_presentation: None,
            };
            gpu.pixel_primitive(primitive);
            assert_eq!(gpu.pixels, expected, "采样 {quality:?}，反向 {reverse}");
            let resident = gpu.pipeline.stream_regions.values().next().unwrap();
            assert!(resident.buffers.segments.len() > 1);
            assert!(gpu.pipeline.world_sources.slab_count() > 1);
            let opaque_count = faces.iter().filter(|q| !q.blended()).count();
            assert_eq!(
                resident
                    .buffers
                    .segments
                    .iter()
                    .map(|s| s.buffer.size())
                    .sum::<u64>(),
                opaque_count as u64 * std::mem::size_of::<encoding::RegionalFull>() as u64
            );
            assert!(resident
                .buffers
                .segments
                .iter()
                .all(|s| s.buffer.size() <= 8192));
            assert!(Arc::ptr_eq(&resident.source.members[0].source, &faces));
        }
    }
    // 同时保活跨物理 bank 的来源，再交替显示其子集，核对帧映射与旧快照生命周期。
    let capacity =
        (gpu.device.limits().max_buffer_size / std::mem::size_of::<WorldQuad>() as u64) as usize;
    let mut pinned = Vec::new();
    for i in 0..9 {
        let mut visible = blend;
        visible.rgba = [[i as f32 / 8., 0.25, 1. - i as f32 / 8., 0.4]; 4];
        visible.xyz = visible.xyz.map(|mut p| {
            p[2] += i as f32 * 0.05;
            p
        });
        let mut quads = vec![visible];
        quads.extend((1..capacity).map(|n| visible.translated([1000. + n as f32 * 4., 0., 0.])));
        *quads.last_mut().unwrap() = visible.translated([0.1, 0.1, -0.3]);
        let source = Arc::new(quads);
        let allocation = gpu
            .pipeline
            .transparent_residency
            .reserve(Arc::clone(&source), source.len())
            .unwrap();
        loop {
            let mut budget = upload::Budget::new(upload::Limits::default());
            let ready = gpu
                .pipeline
                .transparent_residency
                .advance(&gpu.device, &gpu.queue, &mut budget)
                .unwrap();
            assert!(budget.used() <= upload::Limits::default().bytes);
            if ready {
                break;
            }
        }
        gpu.pipeline
            .transparent_residency
            .upload(&allocation, 0, &source, &gpu.device, &gpu.queue)
            .unwrap();
        pinned.push((source, allocation));
    }
    let first = pinned[0].1.address(0).unwrap();
    let last = pinned[8].1.address(0).unwrap();
    assert_ne!(first.bank, last.bank, "受限设备场景必须跨越真实物理 bank");
    let camera = crate::spectator::CameraUniform {
        position: [0.; 4],
        right: [1., 0., 0., 0.],
        up: [0., 1., 0., 0.],
        forward: [0., 0., 1., 0.],
        projection: [1., 1., 0.05, 100.],
    };
    let primitive = |vertices, stream_mesh| Primitive {
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
        frame_tick: None,
        vertices,
        stream_mesh,
        atlas: Some(Arc::clone(&atlas)),
        quality: Quality::Detailed,
        camera: Some(camera),
        source_origin: Some([0; 3]),
        presentation: None,
        iso_presentation: None,
    };
    let mut saved: Option<(transparent::Sources, Vec<[u8; 4]>)> = None;
    let all = (0..pinned.len()).collect::<Vec<_>>();
    for selection in [&[0_usize, 8][..], &[8][..], all.as_slice(), &[0, 8][..]] {
        let expanded = Arc::new(
            selection
                .iter()
                .flat_map(|&i| pinned[i].0.iter().flat_map(WorldQuad::expanded))
                .collect(),
        );
        gpu.pixel_primitive(primitive(expanded, None));
        let expected = gpu.pixels.clone();
        let mesh = Arc::new(crate::spectator::StreamMesh {
            terrain: None,
            visibility: None,
            entities: None,
            complete: true,
            origin: [0; 3],
            atlas: Some(Arc::clone(&atlas)),
            chunks: selection
                .iter()
                .map(|&i| {
                    crate::spectator::MeshChunk::new(
                        (i as i32, 0, 1, 0, 0),
                        [0; 3],
                        Arc::clone(&pinned[i].0),
                        Default::default(),
                    )
                })
                .collect::<Vec<_>>()
                .into(),
        });
        gpu.pixel_primitive(primitive(Arc::new(Vec::new()), Some(mesh)));
        assert_eq!(
            gpu.pixels, expected,
            "跨物理 bank 的逐面顺序和颜色必须保持一致"
        );
        let sources = &gpu.pipeline.world_sources;
        assert_eq!(
            sources.segments.iter().flatten().count(),
            sources
                .slab_count()
                .div_ceil(residency::bank_width(&gpu.device)),
            "活跃 slab 应密集绑定，超过设备槽位容量后才拆组"
        );
        if selection.len() <= 2 {
            assert_eq!(sources.address(last).unwrap().bank, 0);
        } else {
            assert!(sources.segments.len() > 1);
        }
        if let Some((old, pixels)) = &saved {
            if selection.len() == 1 {
                assert_ne!(
                    old.address(last).unwrap(),
                    sources.address(last).unwrap(),
                    "子集使用独立紧凑槽位，不改写旧帧映射"
                );
                assert_eq!(old.address(first).unwrap().bank, 0);
            } else if selection.len() == 2 {
                assert_eq!(&gpu.pixels, pixels, "恢复旧来源后画面必须一致");
            }
        } else {
            saved = Some((sources.clone(), gpu.pixels.clone()));
        }
    }
    // 扩容期间逐帧核验真实复制预算，并继续绘制旧快照；完成后核对新增来源像素。
    let mut gpu = GpuProbe::with_limits(wgpu::Limits {
        max_buffer_size: 8192,
        ..Default::default()
    });
    let a = Arc::new(vec![blend; 4]);
    let mut other = blend;
    other.xyz = other.xyz.map(|mut p| {
        p[2] += 0.4;
        p
    });
    other.rgba = [[0.2, 0.8, 0.3, 0.5]; 4];
    let b = Arc::new(vec![other; 6]);
    let mesh = |parts: &[Arc<Vec<WorldQuad>>]| {
        Arc::new(crate::spectator::StreamMesh {
            terrain: None,
            visibility: None,
            entities: None,
            complete: true,
            origin: [0; 3],
            atlas: Some(Arc::clone(&atlas)),
            chunks: parts
                .iter()
                .enumerate()
                .map(|(i, quads)| {
                    crate::spectator::MeshChunk::new(
                        (i as i32, 0, 1, 0, 0),
                        [0; 3],
                        Arc::clone(quads),
                        Default::default(),
                    )
                })
                .collect::<Vec<_>>()
                .into(),
        })
    };
    let old = primitive(Arc::new(Vec::new()), Some(mesh(&[Arc::clone(&a)])));
    gpu.pixel_primitive(old.clone());
    let expected_old = gpu.pixels.clone();
    let stride = std::mem::size_of::<WorldQuad>();
    let cancelled = gpu
        .pipeline
        .transparent_residency
        .reserve(Arc::clone(&b), b.len())
        .unwrap();
    let mut budget = upload::Budget::new(upload::Limits {
        bytes: stride,
        transfer: stride,
        time: Duration::from_secs(60),
    });
    assert!(!gpu
        .pipeline
        .transparent_residency
        .advance(&gpu.device, &gpu.queue, &mut budget)
        .unwrap());
    assert_eq!(budget.used(), stride);
    drop(cancelled);
    let mut budget = upload::Budget::new(upload::Limits::default());
    assert!(gpu
        .pipeline
        .transparent_residency
        .advance(&gpu.device, &gpu.queue, &mut budget)
        .unwrap());
    assert_eq!(budget.used(), 0, "取消后不能继续复制已无需求的扩容缓冲");
    let allocation = gpu
        .pipeline
        .transparent_residency
        .reserve(Arc::clone(&b), b.len())
        .unwrap();
    for frame in 0..4 {
        let mut budget = upload::Budget::new(upload::Limits {
            bytes: stride,
            transfer: stride,
            time: Duration::from_secs(60),
        });
        let ready = gpu
            .pipeline
            .transparent_residency
            .advance(&gpu.device, &gpu.queue, &mut budget)
            .unwrap();
        assert_eq!(budget.used(), stride, "扩容复制必须计入本帧预算");
        assert_eq!(ready, frame == 3, "四个已写入面分四帧复制");
        gpu.pipeline.last_visibility = None;
        gpu.pipeline.profile_pass_issued = false;
        gpu.pixel_primitive(old.clone());
        assert!(
            gpu.pipeline.profile_pass_issued,
            "必须重新绘制旧来源，不能只比对缓存合成图"
        );
        assert_eq!(gpu.pixels, expected_old, "扩容不能破坏已经显示的快照");
    }
    gpu.pipeline
        .transparent_residency
        .upload(&allocation, 0, &b, &gpu.device, &gpu.queue)
        .unwrap();
    let expanded = Arc::new(
        a.iter()
            .chain(b.iter())
            .flat_map(WorldQuad::expanded)
            .collect(),
    );
    gpu.pixel_primitive(primitive(expanded, None));
    let expected = gpu.pixels.clone();
    gpu.pixel_primitive(primitive(Arc::new(Vec::new()), Some(mesh(&[a, b]))));
    assert_eq!(
        gpu.pixels, expected,
        "分帧扩容后的完整来源必须与原始几何一致"
    );
}

fn quad(depth: f32, rgba: [f32; 4], mode: u32) -> Vec<Vertex> {
    let corners = [[-0.75, -0.75], [0.75, -0.75], [0.75, 0.75], [-0.75, 0.75]];
    [0, 1, 2, 0, 2, 3]
        .map(|index| Vertex {
            xy: corners[index],
            depth,
            rgba,
            uv: [0.5; 2],
            mode,
            xyz: [0.0; 3],
            world_mode: 0,
            atlas_page: 0,
            light: crate::lighting::FULLBRIGHT,
        })
        .to_vec()
}

// 这些非加色场景只回读几何层，普通画布天空由界面合成。
fn scene_pixel(rgba: &[u8; 4]) -> bool {
    rgba[3] != 0
}

#[track_caller]
fn assert_pixel(actual: [u8; 4], expected: [u8; 4]) {
    assert!(
        actual
            .into_iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) <= 1),
        "GPU 像素不匹配：实际 {actual:?}，预期 {expected:?}"
    );
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_composite_root_transform_and_empty_model_pixels() {
    use crate::{
        spectator::{Camera, SpectatorScene},
        textures::TextureBank,
    };
    use mcedit2_world::{
        editing::parse_state,
        surface::{SurfaceView, SurfaceVoxel, VoxelMaterial},
    };
    use std::io::Write;
    let path =
        std::env::temp_dir().join(format!("mcedit2-composite-gpu-{}.zip", std::process::id()));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name, doc) in [
        (
            "assets/demo/blockstates/machine.json",
            r#"{"variants":{"":{"model":"demo:block/machine"}}}"#,
        ),
        (
            "assets/demo/blockstates/empty.json",
            r#"{"variants":{"":{"model":"demo:block/empty"}}}"#,
        ),
        (
            "assets/demo/models/block/machine.json",
            r#"{"loader":"neoforge:composite","children":{"left":{"parent":"demo:block/part","textures":{"all":"demo:block/red"},"render_type":"solid","transform":{"translation":[-0.25,0,0],"scale":[-0.5,1,1],"origin":"center"}},"right":{"parent":"demo:block/part","textures":{"all":"demo:block/blue"},"render_type":"translucent","transform":{"translation":[0.25,0,0],"scale":[0.5,1,1],"origin":"center"}}}}"#,
        ),
        (
            "assets/demo/models/block/part.json",
            r#"{"parent":"block/cube_all"}"#,
        ),
        (
            "assets/demo/models/block/empty.json",
            r#"{"loader":"neoforge:empty"}"#,
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(doc.as_bytes()).unwrap();
    }
    for (name, color) in [("red", [255u8, 0, 0, 128]), ("blue", [0u8, 0, 255, 128])] {
        let mut output = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(1, 1, image::Rgba(color))
            .write_to(&mut output, image::ImageFormat::Png)
            .unwrap();
        zip.start_file(
            format!("assets/demo/textures/block/{name}.png"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(output.get_ref()).unwrap();
    }
    zip.finish().unwrap();
    let bank = Arc::new(TextureBank::open_stack_java(std::slice::from_ref(&path)).unwrap());
    let scene = SpectatorScene {
        entity_enabled: false,
        entity_frame: None,
        entity_cache: Default::default(),
        surface: Arc::new(SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: 0,
            y_max: 15,
            width: 1,
            depth: 1,
            voxels: vec![SurfaceVoxel::new(
                [0, 0, 0],
                mcedit2_world::surface::VoxelData {
                    state: parse_state("demo:machine").unwrap().into(),
                    biome: None,
                    original_block_entity: false,
                    faces: 63,
                    kind: VoxelMaterial::Solid,
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
        texture_bank: Some(Arc::clone(&bank)),
        show_textures: true,
        block_layers: Default::default(),
        camera: Camera {
            position: [-0.25, 0., -2.],
            yaw: 0.,
            pitch: 0.,
            zoom: 1.0,
            perspective: true,
        },
        quality: Quality::Fast,
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
    let committed = Arc::new(crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    let size = Size::new(EDGE as f32, EDGE as f32);
    let picked = committed
        .pick(
            scene.camera,
            iced::Point::new(size.width * 0.5, size.height * 0.5),
            crate::render_viewport::RenderViewport::one_to_one(size),
            Quality::Fast,
        )
        .unwrap();
    assert_eq!(picked.owner.as_ref().unwrap().position, [0, 0, 0]);
    assert!(picked.outline.iter().all(|p| p[0] <= 1e-6));
    let mut gpu = GpuProbe::new();
    assert_pixel(
        gpu.pixel_primitive(Primitive {
            lighting: crate::lighting::Table::unlit(),
            light_report: std::sync::Weak::new(),
            frame_tick: None,
            vertices: Arc::new(Vec::new()),
            atlas: committed.atlas.clone(),
            quality: Quality::Fast,
            camera: Some(scene.camera.uniform_at_origin(committed.origin, size)),
            source_origin: Some(committed.origin),
            stream_mesh: Some(Arc::clone(&committed)),
            presentation: None,
            iso_presentation: None,
        }),
        [255, 0, 0, 255],
    );
    let primitive = |vertices: Vec<Vertex>, camera: Camera| Primitive {
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
        frame_tick: None,
        vertices: Arc::new(vertices),
        atlas: bank.atlas().cloned(),
        quality: Quality::Fast,
        camera: Some(camera.uniform_at_origin([0; 3], Size::new(EDGE as f32, EDGE as f32))),
        source_origin: Some([0; 3]),
        stream_mesh: None,
        presentation: None,
        iso_presentation: None,
    };
    assert_pixel(
        gpu.pixel_primitive(primitive(vertices.clone(), scene.camera)),
        [255, 0, 0, 255],
    );
    let other = Camera {
        position: [0.25, 0., -2.],
        ..scene.camera
    };
    let picked = committed
        .pick(
            other,
            iced::Point::new(size.width * 0.5, size.height * 0.5),
            crate::render_viewport::RenderViewport::one_to_one(size),
            Quality::Fast,
        )
        .unwrap();
    assert_eq!(picked.owner.as_ref().unwrap().position, [0, 0, 0]);
    assert!(picked.outline.iter().all(|p| p[0] >= -1e-6));
    let empty_pixel = gpu.pixel_primitive(primitive(Vec::new(), other));
    assert_pixel(empty_pixel, [0; 4]);
    let blue = gpu.pixel_primitive(primitive(vertices, other));
    assert!(
        blue[0] == 0 && blue[1] == 0 && blue[2] > 0 && (1..255).contains(&blue[3]),
        "应回读蓝色半透明几何：{blue:?}"
    );
    let mut empty = (*scene.surface).clone();
    empty.voxels[0].state = parse_state("demo:empty").unwrap().into();
    let empty = crate::spectator::build(&SpectatorScene {
        surface: Arc::new(empty),
        chunk_cache: Default::default(),
        ..scene
    });
    assert!(empty.is_empty());
    assert_pixel(gpu.pixel_primitive(primitive(empty, other)), empty_pixel);
    std::fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_real_client_materials_reach_gpu_pixels() {
    use crate::isometric::Face;
    use mcedit2_world::editing::parse_state;
    let path =
        std::env::var_os("MCEDIT2_CLIENT_JAR").expect("设置 MCEDIT2_CLIENT_JAR 指向实际客户端 JAR");
    let path = std::path::PathBuf::from(path);
    let numeric = crate::legacy_render::Version::from_client_path(&path).is_some();
    let paths = if let Some(world) = std::env::var_os("MCEDIT2_TEST_WORLD") {
        let world = mcedit2_world::inspect_world(std::path::PathBuf::from(world)).unwrap();
        crate::auto_assets::discover_stack(
            &world.root,
            world.version_name.as_deref().unwrap_or(""),
            &[],
        )
        .unwrap()
        .unwrap()
        .paths
    } else {
        vec![path]
    };
    let bank = installed_bank(&paths);
    eprintln!(
        "实际资源：{} 纹理、{} 模型、{} 页；诊断 {:?}",
        bank.count(),
        bank.model_count(),
        bank.atlas().unwrap().page_count(),
        bank.warnings.iter().take(8).collect::<Vec<_>>()
    );
    assert!(bank.count() > 100);
    let mut gpu = GpuProbe::new();
    let states = if numeric {
        vec![
            "numeric:1[data=0]",
            "numeric:1[data=1]",
            "numeric:5[data=1]",
            "numeric:7[data=0]",
            "numeric:43[data=8]",
            "numeric:53[data=4]",
        ]
    } else {
        vec![
            "minecraft:stone",
            "minecraft:dirt",
            "minecraft:oak_planks",
            "minecraft:mud_bricks",
        ]
    };
    for name in states {
        let state = parse_state(name).unwrap();
        let original_state = state.clone();
        let (uv, _, page) = bank
            .atlas_face_style_at(&state, Face::Top, 0, (0, 64, 0))
            .unwrap();
        let texture = bank
            .resolve_id_at(&state, Face::Top, (0, 64, 0))
            .unwrap_or_else(|| panic!("没有解析到真实纹理：{name}"));
        assert!(!texture.contains("missingno"));
        let mut center = [
            uv.iter().map(|p| p[0]).sum::<f32>() / 4.0,
            uv.iter().map(|p| p[1]).sum::<f32>() / 4.0,
        ];
        let atlas_page = bank.atlas().unwrap().page(page).unwrap();
        center[0] = ((center[0] * atlas_page.width as f32).floor() + 0.5) / atlas_page.width as f32;
        center[1] =
            ((center[1] * atlas_page.height as f32).floor() + 0.5) / atlas_page.height as f32;
        let expected = bank.atlas_pixel(center, page).unwrap();
        let mut vertices = quad(0.25, [1.0; 4], 0);
        for vertex in &mut vertices {
            vertex.uv = center;
            vertex.atlas_page = page;
        }
        let actual = gpu.pixel_primitive(Primitive {
            lighting: crate::lighting::Table::unlit(),
            light_report: std::sync::Weak::new(),
            frame_tick: None,
            vertices: Arc::new(vertices),
            atlas: bank.atlas().cloned(),
            quality: Quality::Fast,
            camera: None,
            source_origin: None,
            stream_mesh: None,
            presentation: None,
            iso_presentation: None,
        });
        assert_pixel(actual, expected);
        assert!(
            bank.geometry_at(&state, (0, 0, 0)).is_some(),
            "原版状态应解析到实际 JSON 模型：{name}"
        );
        if !name.starts_with("numeric:53") {
            use crate::spectator::{Camera, SpectatorScene};
            use mcedit2_world::surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES};
            let face_texture = bank.resolve_id_at(&state, Face::NegZ, (0, 0, 0)).unwrap();
            let source_tile = bank.get(&face_texture).unwrap();
            let view = SpectatorScene {
                entity_enabled: false,
                entity_frame: None,
                entity_cache: Default::default(),
                surface: Arc::new(SurfaceView {
                    biomes: Default::default(),
                    x: 0,
                    z: 0,
                    y_min: 0,
                    y_max: 1,
                    width: 1,
                    depth: 1,
                    voxels: vec![SurfaceVoxel::new(
                        [0, 0, 0],
                        mcedit2_world::surface::VoxelData {
                            state: state.clone().into(),
                            biome: None,
                            original_block_entity: false,
                            faces: ALL_FACES,
                            kind: VoxelMaterial::Solid,
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
                texture_bank: Some(Arc::new(bank.clone())),
                show_textures: true,
                block_layers: Default::default(),
                camera: Camera {
                    position: [0.0, 0.0, -2.0],
                    yaw: 0.0,
                    pitch: 0.0,
                    // 本段核对源 texel；放大到原生像素，缩小时的 mip 混色另有像素对照。
                    zoom: source_tile.width.max(source_tile.height) as f32,
                    perspective: true,
                },
                quality: Quality::Fast,
                cache: Default::default(),
                pending: Default::default(),
                presented: Default::default(),
                presented_view: Default::default(),
                fallback: None,
                snapshot: None,
                chunk_cache: Default::default(),
                lighting: crate::lighting::Table::unlit(),
            };
            let vertices = crate::spectator::build(&view);
            assert_eq!(vertices.len(), 36, "实际方块应生成六个面的完整网格：{name}");
            let actual = gpu.pixel_primitive(Primitive {
                lighting: crate::lighting::Table::unlit(),
                light_report: std::sync::Weak::new(),
                frame_tick: None,
                vertices: Arc::new(vertices),
                atlas: bank.atlas().cloned(),
                quality: Quality::Fast,
                camera: Some(
                    view.camera
                        .uniform_at_origin([0; 3], Size::new(EDGE as f32, EDGE as f32)),
                ),
                source_origin: Some([0; 3]),
                stream_mesh: None,
                presentation: None,
                iso_presentation: None,
            });
            if !bank
                .get(&face_texture)
                .unwrap()
                .rgba
                .chunks_exact(4)
                .any(|p| p == actual)
            {
                for face in [
                    Face::Top,
                    Face::Bottom,
                    Face::PosX,
                    Face::NegX,
                    Face::PosZ,
                    Face::NegZ,
                ] {
                    eprintln!(
                        "{name} {face:?}: {:?} / {:?}",
                        bank.atlas_face_style_at(&state, face, 0, (0, 0, 0)),
                        bank.face_tint_with_biome_at(&state, face, 0, (0, 0, 0), None)
                    );
                }
                eprintln!(
                    "实际网格 {:?}",
                    crate::spectator::build(&view)
                        .chunks_exact(6)
                        .map(|v| (v[0].xyz, v[0].uv, v[0].rgba, v[0].atlas_page))
                        .collect::<Vec<_>>()
                );
            }
            assert!(
                bank.get(&face_texture)
                    .unwrap()
                    .rgba
                    .chunks_exact(4)
                    .any(|p| p == actual),
                "完整网格像素 {actual:?} 必须来自原始纹理 {face_texture}"
            );
        }
        assert_eq!(state, original_state, "渲染绑定不得改动源状态");
        eprintln!("{name} -> {texture} -> 第 {page} 页，实际 GPU 像素 {actual:?}");
    }
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_depth_alpha_and_culling_pixels() {
    let mut gpu = GpuProbe::new();
    // 深度较远的蓝面不能覆盖先提交的近处绿面。
    let tripwire =
        crate::textures::AlphaMode::from_render_type("minecraft:tripwire").unwrap() as u32;
    let low_mask = crate::textures::AlphaMode::from_render_type("minecraft:cutout").unwrap() as u32;
    assert_pixel(
        gpu.pixel(quad(0.5, [1., 0., 0., 25. / 255.], low_mask)),
        [0; 4],
    );
    assert_pixel(
        gpu.pixel(quad(0.5, [1., 0., 0., 26. / 255.], low_mask)),
        [255, 0, 0, 255],
    );
    assert_pixel(
        gpu.pixel(quad(0.5, [1., 0., 0., 25. / 255.], tripwire)),
        [0; 4],
    );
    assert_pixel(
        gpu.pixel(quad(0.5, [1., 0., 0., 26. / 255.], tripwire)),
        [26, 0, 0, 26],
    );
    assert_pixel(
        gpu.pixel(quad(0.5, [1., 0., 0., 0.5], tripwire)),
        [128, 0, 0, 128],
    );
    assert_pixel(
        gpu.pixel(
            [
                quad(0.25, [0.0, 1.0, 0.0, 1.0], 0),
                quad(0.75, [0.0, 0.0, 1.0, 1.0], 0),
            ]
            .concat(),
        ),
        [0, 255, 0, 255],
    );
    // BLEND 只在透明通道绘制，并正确叠加到不透明背景。
    assert_pixel(
        gpu.pixel(
            [
                quad(0.75, [0.0, 0.0, 1.0, 1.0], 0),
                quad(0.25, [1.0, 0.0, 0.0, 0.5], 2),
            ]
            .concat(),
        ),
        [128, 0, 128, 255],
    );
    // 透明通道不写深度；测试特意先提交近面，再提交远面。
    assert_pixel(
        gpu.pixel(
            [
                quad(0.25, [1.0, 0.0, 0.0, 0.5], 2),
                quad(0.75, [0.0, 1.0, 0.0, 0.5], 2),
            ]
            .concat(),
        ),
        [64, 128, 0, 191],
    );
    assert_pixel(gpu.pixel(quad(0.5, [1.0, 0.0, 0.0, 0.4], 1)), [0; 4]);
    let opaque = crate::material_mode::encode(2, Some("OPAQUE"), 0.5, true);
    assert_pixel(
        gpu.pixel(quad(0.5, [1.0, 0.0, 0.0, 0.0], opaque)),
        [255, 0, 0, 255],
    );
    let mask = crate::material_mode::encode(0, Some("MASK"), 0.25, true);
    assert_pixel(
        gpu.pixel(quad(0.5, [1.0, 0.0, 0.0, 0.4], mask)),
        [255, 0, 0, 255],
    );
    let cutoff = crate::material_mode::encode(0, Some("MASK"), 0.75, true);
    assert_pixel(gpu.pixel(quad(0.5, [1.0, 0.0, 0.0, 0.4], cutoff)), [0; 4]);
    let single_sided = crate::material_mode::encode(0, Some("OPAQUE"), 0.5, false);
    let front = quad(0.5, [1.0, 0.0, 0.0, 1.0], single_sided);
    assert_pixel(gpu.pixel(front.clone()), [255, 0, 0, 255]);
    let back = front
        .chunks_exact(3)
        .flat_map(|triangle| [triangle[0], triangle[2], triangle[1]])
        .collect();
    assert_pixel(gpu.pixel(back), [0; 4]);
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_stream_frustum_preserves_resident_mesh_on_camera_change() {
    let mut gpu = GpuProbe::new();
    let vertices = Arc::new(
        quad(0.5, [1.0, 0.0, 0.0, 1.0], 0)
            .into_iter()
            .map(|mut v| {
                v.xyz = [v.xy[0], v.xy[1], 2.0];
                v.world_mode = 1;
                v
            })
            .collect::<Vec<_>>(),
    );
    let vertices = Arc::new(vec![
        crate::world_mesh::WorldQuad::from_vertices(&vertices).unwrap()
    ]);
    let stream = Arc::new(crate::spectator::StreamMesh {
        terrain: None,
        visibility: None,
        entities: None,
        complete: true,
        atlas: None,
        origin: [0; 3],
        chunks: vec![crate::spectator::MeshChunk::new(
            (0, 0, 1, 0, 0),
            [0; 3],
            Arc::clone(&vertices),
            Default::default(),
        )]
        .into(),
    });
    let camera = crate::spectator::CameraUniform {
        position: [0.0; 4],
        right: [1.0, 0.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0, 0.0],
        forward: [0.0, 0.0, 1.0, 0.0],
        projection: [1.0, 1.0, 0.05, 100.0],
    };
    let scene = |camera| Primitive {
        lighting: crate::lighting::Table::unlit(),
        light_report: std::sync::Weak::new(),
        frame_tick: None,
        vertices: Arc::new(Vec::new()),
        atlas: None,
        quality: Quality::Fast,
        camera: Some(camera),
        source_origin: Some([0; 3]),
        stream_mesh: Some(Arc::clone(&stream)),
        presentation: None,
        iso_presentation: None,
    };
    assert_pixel(gpu.pixel_primitive(scene(camera)), [255, 0, 0, 255]);
    let before = {
        let commands = &gpu
            .pipeline
            .stream_regions
            .values()
            .next()
            .unwrap()
            .commands;
        (commands.range_builds, commands.selection_reuses)
    };
    let nearby = crate::spectator::CameraUniform {
        position: [0.125, 0.0, 0.0, 0.0],
        right: [0.9998, 0.0, -0.02, 0.0],
        forward: [0.02, 0.0, 0.9998, 0.0],
        ..camera
    };
    gpu.pipeline.profile_pass_issued = false;
    assert_pixel(gpu.pixel_primitive(scene(nearby)), [255, 0, 0, 255]);
    assert!(gpu.pipeline.profile_pass_issued, "必须以新相机真正重绘世界");
    let commands = &gpu
        .pipeline
        .stream_regions
        .values()
        .next()
        .unwrap()
        .commands;
    assert_eq!(
        commands.range_builds, before.0,
        "可见范围相同不能再遍历高度片拼装指令"
    );
    assert!(commands.selection_reuses > before.1);
    let moved = crate::spectator::CameraUniform {
        position: [0.0, 0.0, 4.0, 0.0],
        ..camera
    };
    assert_pixel(gpu.pixel_primitive(scene(moved)), [0; 4]);
    assert_eq!(gpu.pipeline.stream_chunks.len(), 1);
    assert!(Arc::ptr_eq(
        &gpu.pipeline.stream_chunks.values().next().unwrap().source,
        &vertices
    ));
    let rotated = crate::spectator::CameraUniform {
        forward: [0.0, 0.0, -1.0, 0.0],
        right: [-1.0, 0.0, 0.0, 0.0],
        ..moved
    };
    assert_pixel(gpu.pixel_primitive(scene(rotated)), [255, 0, 0, 255]);
    assert!(Arc::ptr_eq(
        &gpu.pipeline.stream_chunks.values().next().unwrap().source,
        &vertices
    ));
    // 世界边界附近使用独立区块基底，透明面仍按同一帧原点排序。
    let mut mixed = (*vertices).clone();
    mixed.extend(vertices.iter().copied().map(|mut vertex| {
        for point in &mut vertex.xyz {
            point[2] = 1.0;
        }
        vertex.rgba = [[0.0, 0.0, 1.0, 0.5]; 4];
        vertex.mode = 2;
        vertex
    }));
    let mixed = Arc::new(mixed);
    let chunk_origin = [29_999_984, 80, -30_000_000];
    let mut moved_stream = (*stream).clone();
    moved_stream.origin = [29_999_968, 64, -30_000_000];
    let mut moved_chunks = moved_stream.chunks.to_vec();
    moved_chunks[0].origin = chunk_origin;
    moved_chunks[0] = crate::spectator::MeshChunk::new(
        moved_chunks[0].key,
        moved_chunks[0].origin,
        Arc::clone(&mixed),
        Arc::clone(&moved_chunks[0].picking),
    );
    moved_stream.chunks = moved_chunks.into();
    let render = |stream: crate::spectator::StreamMesh| {
        let delta = origin_delta(chunk_origin, stream.origin);
        Primitive {
            lighting: crate::lighting::Table::unlit(),
            light_report: std::sync::Weak::new(),
            frame_tick: None,
            vertices: Arc::new(Vec::new()),
            atlas: None,
            quality: Quality::Fast,
            camera: Some(crate::spectator::CameraUniform {
                position: [delta[0], delta[1], delta[2], 0.0],
                ..camera
            }),
            source_origin: Some(stream.origin),
            stream_mesh: Some(Arc::new(stream)),
            presentation: None,
            iso_presentation: None,
        }
    };
    assert_pixel(
        gpu.pixel_primitive(render(moved_stream.clone())),
        [127, 0, 128, 255],
    );
    let resident_buffer = gpu
        .pipeline
        .stream_regions
        .values()
        .next()
        .unwrap()
        .buffers
        .segments[0]
        .buffer
        .clone();
    moved_stream.origin = [29_999_984, 32, -29_999_984];
    assert_pixel(
        gpu.pixel_primitive(render(moved_stream)),
        [127, 0, 128, 255],
    );
    let retained = gpu.pipeline.stream_regions.values().next().unwrap();
    assert_eq!(resident_buffer, retained.buffers.segments[0].buffer);
    assert!(Arc::ptr_eq(&retained.source.members[0].source, &mixed));
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_unused_atlas_pages_preserve_resident_geometry() {
    use crate::{spectator::StreamMesh, textures::AtlasPage};
    for mode in [0, crate::material_mode::ORDERED_OPAQUE, 2] {
        let (_, mut view, original, _) = crate::picking_integration_tests::fixture();
        let mut atlas = TextureAtlas::empty_test_atlas();
        atlas.pages = vec![
            Arc::new(AtlasPage {
                width: 64,
                height: 64,
                rgba: [255, 0, 0, 255].repeat(64 * 64),
            }),
            Arc::new(AtlasPage {
                width: 32,
                height: 32,
                rgba: [0, 255, 0, 255].repeat(32 * 32),
            }),
        ];
        atlas.rebuild_raw_test_mips();
        let grown = Arc::new(atlas);
        let mut smaller = (*grown).clone();
        smaller.pages.truncate(1);
        smaller.rebuild_raw_test_mips();
        let smaller = Arc::new(smaller);
        let hints = material_hints::Hints::new(Some(&grown), &[0, 1], true, true).unwrap();
        let trimmed_hints = material_hints::Hints::new(Some(&smaller), &[0], true, true).unwrap();
        assert!(hints.compatible(&trimmed_hints, &[0, 1], &[0], [0]));
        assert!(
            !hints.compatible(&trimmed_hints, &[0, 1], &[0], [1]),
            "使用中的页缺失必须拒绝复用"
        );
        let mut mesh = (*original).clone();
        mesh.chunks = original
            .chunks
            .iter()
            .map(|part| {
                let source = part
                    .quads()
                    .iter()
                    .copied()
                    .map(|mut q| {
                        q.uv = [[0.5; 2]; 4];
                        q.rgba = [[1.; 4]; 4];
                        q.mode = mode;
                        q.atlas_page = 0;
                        q
                    })
                    .collect();
                crate::spectator::MeshChunk::new(
                    part.key,
                    part.origin,
                    Arc::new(source),
                    Arc::clone(&part.picking),
                )
            })
            .collect::<Vec<_>>()
            .into();
        mesh.atlas = Some(Arc::clone(&grown));
        let old = Arc::new(mesh);
        let size = Size::new(160., 128.);
        let draw = |view: &crate::spectator::SpectatorScene| {
            let mut primitive = shader::Program::draw(
                &GpuScene(SceneSource::Spectator(view.clone())),
                &State::default(),
                mouse::Cursor::Unavailable,
                Rectangle::with_size(size),
            );
            primitive.presentation.as_mut().unwrap().section_order = true;
            primitive
        };
        let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), Size::new(160, 128));
        *view.cache.lock().unwrap() = Some(Arc::clone(&old));
        assert_pixel(gpu.pixel_primitive(draw(&view)), [255, 0, 0, 255]);
        let region_buffers = gpu
            .pipeline
            .stream_regions
            .values()
            .map(|r| Arc::clone(&r.buffers))
            .collect::<Vec<_>>();
        let ordered_buffers = gpu
            .pipeline
            .stream_chunks
            .iter()
            .map(|(k, c)| (*k, Arc::clone(&c.ordered)))
            .collect::<Vec<_>>();
        let preparations = gpu.pipeline.ordered_preparations;
        let section_orders = gpu.pipeline.section_orders.clone();
        if mode == 2 {
            assert!(
                !section_orders.is_empty(),
                "透明模式必须产生真实 Section 索引"
            );
        }
        let mut next = (*old).clone();
        next.atlas = Some(Arc::clone(&smaller));
        *view.cache.lock().unwrap() = Some(Arc::new(next));
        assert_pixel(gpu.pixel_primitive(draw(&view)), [255, 0, 0, 255]);
        assert_eq!(gpu.pipeline.ordered_preparations, preparations);
        assert_eq!(
            region_buffers.len(),
            gpu.pipeline.stream_regions.values().count()
        );
        assert_eq!(ordered_buffers.len(), gpu.pipeline.stream_chunks.len());
        assert_eq!(section_orders.len(), gpu.pipeline.section_orders.len());
        for (key, old) in &section_orders {
            assert!(
                Arc::ptr_eq(old, &gpu.pipeline.section_orders[key]),
                "无关材质页变化不能重建透明 Section 索引"
            );
        }
        for (old, new) in region_buffers
            .iter()
            .zip(gpu.pipeline.stream_regions.values())
        {
            assert!(
                Arc::ptr_eq(old, &new.buffers),
                "删除未使用的运行时页不能重传地形"
            );
        }
        for (key, buffers) in &ordered_buffers {
            assert!(
                Arc::ptr_eq(buffers, &gpu.pipeline.stream_chunks[key].ordered),
                "有序面只依赖实际使用页"
            );
        }
        let mut changed = (*smaller).clone();
        changed.pages[0] = Arc::new(AtlasPage {
            width: 64,
            height: 64,
            rgba: [0, 0, 255, 255].repeat(64 * 64),
        });
        changed.rebuild_raw_test_mips();
        let changed_hints = material_hints::Hints::new(Some(&changed), &[0], true, true).unwrap();
        assert!(
            !trimmed_hints.compatible(&changed_hints, &[0], &[0], [0]),
            "使用页的 owner / mip 变化必须拒绝复用"
        );
        let mut next: StreamMesh = (*old).clone();
        next.atlas = Some(Arc::new(changed));
        *view.cache.lock().unwrap() = Some(Arc::new(next));
        assert_pixel(gpu.pixel_primitive(draw(&view)), [0, 0, 255, 255]);
    }
}

/// 一个完整交接流程同时核对分帧限额、旧材质/拾取保留、重用和源取消。
#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_staged_world_handoff_keeps_pixels_atlas_and_picking_atomic() {
    super::upload::verify_ready_delivery();
    use crate::{
        spectator::{self, StreamMesh},
        textures::AtlasPage,
    };
    let (_, mut view, original, _) = crate::picking_integration_tests::fixture();
    let mut atlas = TextureAtlas::empty_test_atlas();
    atlas.pages = vec![Arc::new(AtlasPage {
        width: 64,
        height: 64,
        rgba: [255, 0, 0, 255].repeat(64 * 64),
    })];
    atlas.rebuild_raw_test_mips();
    let red = Arc::new(atlas);
    let colorize = |mesh: &mut StreamMesh, page: u32, repetitions: usize| {
        let mut chunks = mesh.chunks.to_vec();
        for part in &mut chunks {
            let source = part.quads().as_ref().clone();
            *part = crate::spectator::MeshChunk::new(
                part.key,
                part.origin,
                Arc::new(
                    (0..repetitions)
                        .flat_map(|repeat| {
                            source.iter().copied().map(move |mut q| {
                                q.uv = [[0.5; 2]; 4];
                                q.rgba = [[1.; 4]; 4];
                                q.mode = if repeat % 3 == 2 {
                                    2
                                } else if repeat % 3 == 1 {
                                    crate::material_mode::ORDERED_OPAQUE
                                } else {
                                    0
                                };
                                if q.mode == 2 {
                                    q.rgba = [[1., 1., 1., 0.3]; 4];
                                }
                                q.atlas_page = page;
                                q
                            })
                        })
                        .collect(),
                ),
                Arc::clone(&part.picking),
            );
        }
        mesh.chunks = chunks.into();
    };
    let mut old = (*original).clone();
    old.atlas = Some(Arc::clone(&red));
    colorize(&mut old, 0, 1);
    let old = Arc::new(old);
    *view.cache.lock().unwrap() = Some(Arc::clone(&old));
    let size = Size::new(200., 200.);
    let draw = |view: &crate::spectator::SpectatorScene| {
        shader::Program::draw(
            &GpuScene(SceneSource::Spectator(view.clone())),
            &State::default(),
            mouse::Cursor::Unavailable,
            Rectangle::with_size(size),
        )
    };
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), Size::new(200, 200));
    assert_pixel(gpu.pixel_primitive(draw(&view)), [255, 0, 0, 255]);
    assert!(Arc::ptr_eq(
        view.presented.lock().unwrap().as_ref().unwrap(),
        &old
    ));
    assert_eq!(
        gpu.pipeline.ordered_preparations, 0,
        "纯普通不透明来源不排队准备有序源"
    );
    let previous_pixels = gpu.pixels.clone();
    // 同一绘制来源更换拾取证明：保留原 GPU 资源，立即显示新的源对象。
    let faces = Arc::clone(&gpu.pipeline.world_faces);
    let texture = gpu.pipeline.atlas_pages[0].0.clone();
    let mut metadata = (*old).clone();
    let mut chunks = metadata.chunks.to_vec();
    for chunk in &mut chunks {
        let mut groups = chunk.picking.source_groups().to_vec();
        for group in &mut groups {
            if let Some(owner) = &mut group.owner {
                Arc::make_mut(&mut owner.state).name = "sample:metadata_only".into();
            }
        }
        chunk.picking = Arc::new(
            crate::picking::Index::build(chunk.origin, Arc::clone(&chunk.quads()), groups).unwrap(),
        );
    }
    metadata.chunks = chunks.into();
    let old = Arc::new(metadata);
    *view.cache.lock().unwrap() = Some(Arc::clone(&old));
    let publications = gpu.pipeline.metadata_publications;
    gpu.pixel_once(&draw(&view));
    assert_eq!(gpu.pipeline.metadata_publications, publications + 1);
    assert!(gpu.pipeline.pending.is_none());
    assert_eq!(gpu.pipeline.upload_bytes, 0);
    assert!(Arc::ptr_eq(&faces, &gpu.pipeline.world_faces));
    assert_eq!(texture, gpu.pipeline.atlas_pages[0].0);
    assert_eq!(gpu.pixels, previous_pixels);
    let shown = view.presented.lock().unwrap().clone().unwrap();
    assert!(Arc::ptr_eq(&shown, &old));
    let old_name = shown
        .pick(
            view.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(size),
            view.quality,
        )
        .unwrap()
        .owner
        .unwrap()
        .state
        .name
        .clone();
    assert_eq!(old_name, "sample:metadata_only");

    // 同一资源补进新 Section，先绘制并点选普通面，透明索引随后独立交接。
    let mut added = (*old).clone();
    let mut added_atlas = (*red).clone();
    added_atlas.pages.push(Arc::new(AtlasPage {
        width: 32,
        height: 32,
        rgba: [0, 255, 0, 255].repeat(32 * 32),
    }));
    added_atlas.rebuild_raw_test_mips();
    added_atlas.mip_pages[..red.mip_pages.len()].clone_from_slice(&red.mip_pages);
    added.atlas = Some(Arc::new(added_atlas));
    let base = &old.chunks[0];
    let mut origin = base.origin;
    origin[0] += 16;
    let mut key = base.key;
    key.0 += 1;
    let mut quads = base.quads().as_ref().clone();
    let solid_end = quads.len();
    quads.extend((0..192).flat_map(|_| {
        base.quads().iter().copied().map(|mut quad| {
            quad.mode = 2;
            quad.rgba = [[1., 1., 1., 0.3]; 4];
            for point in &mut quad.xyz {
                point[2] -= 1.;
            }
            quad
        })
    }));
    let quads = Arc::new(quads);
    let mut solid = base.picking.source_groups()[0].clone();
    solid.quads = 0..solid_end;
    let owner = solid.owner.as_mut().unwrap();
    owner.position[0] += 16;
    Arc::make_mut(&mut owner.state).name = "sample:solid".into();
    let mut water = solid.clone();
    water.quads = solid_end..quads.len();
    let owner = water.owner.as_mut().unwrap();
    owner.position[2] -= 1;
    Arc::make_mut(&mut owner.state).name = "sample:water".into();
    let picking =
        crate::picking::Index::build(origin, Arc::clone(&quads), vec![solid, water]).unwrap();
    let mut parts = old.chunks.to_vec();
    parts.push(crate::spectator::MeshChunk::new(
        key,
        origin,
        quads,
        Arc::new(picking),
    ));
    added.chunks = parts.into();
    let added = Arc::new(added);
    *view.cache.lock().unwrap() = Some(Arc::clone(&added));
    view.camera.position[0] += 16.;
    let limits = gpu.pipeline.upload_limits;
    gpu.pipeline.upload_limits = upload::Limits {
        bytes: 4096,
        transfer: 2048,
        time: Duration::from_secs(1),
    };
    let arriving = draw(&view);
    let mut saw_preview = false;
    for _ in 0..256 {
        let pixel = gpu.pixel_once(&arriving);
        let shown = view.presented.lock().unwrap().clone().unwrap();
        let complete = Arc::ptr_eq(&shown, &added);
        if shown.chunks.len() == added.chunks.len() {
            let hit = shown
                .pick(
                    view.camera,
                    iced::Point::new(100., 100.),
                    crate::render_viewport::RenderViewport::one_to_one(size),
                    view.quality,
                )
                .unwrap();
            let owner = hit.owner.unwrap();
            let name = &owner.state.name;
            if complete {
                assert_eq!(name, "sample:water");
                break;
            }
            assert!(gpu.pipeline.pending.is_some());
            assert!(!shown.complete && gpu.pipeline.world_faces.is_empty());
            assert_pixel(pixel, [255, 0, 0, 255]);
            assert_eq!(name, "sample:solid");
            if !saw_preview {
                // 实际缓冲已就绪时，另一批实体页扩展不应使普通地形预览消失。
                let mut pending = gpu.pipeline.pending.take().unwrap();
                let original_target = pending.target.clone();
                let mut terrain = (*added).clone();
                terrain.atlas = Some(Arc::clone(&red));
                terrain.terrain = None;
                let terrain = Arc::new(terrain);
                let mut target = (*pending.target.stream_mesh.as_ref().unwrap().as_ref()).clone();
                target.terrain = Some(Arc::clone(&terrain));
                pending.target.stream_mesh = Some(Arc::new(target));
                let source = pending.target.atlas.as_ref().unwrap();
                let mut extended = (**source).clone();
                extended.pages.push(Arc::new(AtlasPage {
                    width: 32,
                    height: 32,
                    rgba: [0, 0, 255, 255].repeat(32 * 32),
                }));
                extended.rebuild_raw_test_mips();
                extended.mip_pages[..source.mip_pages.len()].clone_from_slice(&source.mip_pages);
                let extended = Arc::new(extended);
                let mut latest = arriving.clone();
                latest.atlas = Some(Arc::clone(&extended));
                let mut mesh = (*added).clone();
                mesh.atlas = Some(extended);
                mesh.terrain = Some(terrain);
                latest.stream_mesh = Some(Arc::new(mesh));
                pending.update_preview(&gpu.pipeline, &latest);
                assert!(
                    pending
                        .preview
                        .mesh
                        .as_ref()
                        .is_some_and(|mesh| mesh.chunks.len() == added.chunks.len()),
                    "不相关实体页阻塞了已上传的地形"
                );
                pending.target = original_target;
                pending.update_preview(&gpu.pipeline, &arriving);
                gpu.pipeline.pending = Some(pending);
            }
            saw_preview = true;
        }
    }
    assert!(saw_preview, "新 Section 必须在透明索引提交前上屏");
    assert!(gpu.pipeline.pending.is_none());

    // 真实流式世界使用独立的透明源与索引，局部编辑不能替换邻块索引。
    let draw_local = |view: &crate::spectator::SpectatorScene| {
        let mut primitive = draw(view);
        primitive.presentation.as_mut().unwrap().section_order = true;
        primitive
    };
    gpu.pixel_primitive(draw_local(&view));
    commands::verify_binding_dependencies(&gpu.pipeline, &gpu.device, &gpu.queue);
    // 完整准备范围留在 GPU；移动显示范围只切换绘制与拾取，不能重新上传。
    let warm_pixels = gpu.pixels.clone();
    let warm_buffers = gpu
        .pipeline
        .stream_regions
        .values()
        .map(|r| Arc::clone(&r.buffers))
        .collect::<Vec<_>>();
    let mut scoped = draw_local(&view);
    let origin_column = (added.chunks[0].key.0, added.chunks[0].key.1);
    scoped.presentation.as_mut().unwrap().columns =
        Some(Arc::new(std::collections::HashSet::from([origin_column])));
    gpu.pixel_once(&scoped);
    let scoped_shown = view.presented.lock().unwrap().clone().unwrap();
    assert_ne!(
        gpu.pixels, warm_pixels,
        "预备列必须实际退出画面，不能只隐藏拾取"
    );
    assert!(scoped_shown
        .chunks
        .iter()
        .all(|part| part.key.2 != 1 || (part.key.0, part.key.1) == origin_column));
    assert!(crate::spectator::StreamMesh::same_terrain(
        &scoped_shown,
        &added
    ));
    assert_eq!(gpu.pipeline.upload_bytes, 0);
    assert!(gpu.pipeline.pending.is_none());
    let hidden_hit = scoped_shown.pick(
        view.camera,
        iced::Point::new(100., 100.),
        crate::render_viewport::RenderViewport::one_to_one(size),
        view.quality,
    );
    assert!(hidden_hit.and_then(|hit| hit.owner).is_none_or(|owner| (
        owner.position[0].div_euclid(16),
        owner.position[2].div_euclid(16)
    ) == origin_column));
    let all_columns = Arc::new(
        added
            .chunks
            .iter()
            .filter(|p| p.key.2 == 1)
            .map(|p| (p.key.0, p.key.1))
            .collect::<std::collections::HashSet<_>>(),
    );
    let mut waiting = scoped.clone();
    waiting.stream_mesh = Some(Arc::clone(&scoped_shown));
    waiting.presentation.as_mut().unwrap().awaiting_source = true;
    waiting.presentation.as_mut().unwrap().columns = Some(Arc::clone(&all_columns));
    gpu.pixel_once(&waiting);
    assert!(
        gpu.pipeline.pending.is_none(),
        "临时裁剪显示快照不能成为新的上传目标"
    );
    assert_eq!(
        gpu.pipeline
            .active
            .as_ref()
            .unwrap()
            .stream_mesh
            .as_ref()
            .unwrap()
            .chunks
            .len(),
        added.chunks.len(),
        "等待 CPU 快照时必须保留外围 GPU 常驻数据"
    );
    assert_eq!(
        gpu.pixels, warm_pixels,
        "进入视距应立即使用已常驻的外围几何"
    );
    scoped.presentation.as_mut().unwrap().columns = Some(Arc::new(
        added
            .chunks
            .iter()
            .filter(|p| p.key.2 == 1)
            .map(|p| (p.key.0, p.key.1))
            .collect(),
    ));
    gpu.pixel_once(&scoped);
    assert_eq!(gpu.pipeline.upload_bytes, 0);
    assert!(gpu.pipeline.pending.is_none());
    assert_eq!(gpu.pixels, warm_pixels);
    assert!(gpu
        .pipeline
        .stream_regions
        .values()
        .zip(&warm_buffers)
        .all(|(resident, old)| Arc::ptr_eq(&resident.buffers, old)));
    assert_eq!(
        view.presented
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .chunks
            .len(),
        added.chunks.len()
    );
    let fluid_key = (added.chunks[1].origin, true);
    let fluid = Arc::clone(&gpu.pipeline.section_orders[&fluid_key]);
    let fluid_animation = Arc::clone(&gpu.pipeline.section_animations[&fluid_key]);
    let changed_part = |part: &crate::spectator::MeshChunk, name: &str| {
        let mut groups = part.picking.source_groups().to_vec();
        for group in &mut groups {
            Arc::make_mut(&mut group.owner.as_mut().unwrap().state).name = name.into();
        }
        let mut quads = part.quads().as_ref().clone();
        for quad in &mut quads {
            for corner in &mut quad.rgba {
                corner[0] *= 0.5;
            }
        }
        let quads = Arc::new(quads);
        let picking =
            crate::picking::Index::build(part.origin, Arc::clone(&quads), groups).unwrap();
        crate::spectator::MeshChunk::new(part.key, part.origin, quads, Arc::new(picking))
    };
    let mut edit = (*added).clone();
    let mut pieces = edit.chunks.to_vec();
    pieces[0] = changed_part(&pieces[0], "sample:edited_solid");
    let mut extra = pieces[1].clone();
    extra.origin[0] += 16;
    extra.key.0 += 1;
    let mut groups = extra.picking.source_groups().to_vec();
    for group in &mut groups {
        group.owner.as_mut().unwrap().position[0] += 16;
    }
    // 邻段必须是实际尚未上传的新源，不能复用已驻留水体后仍假定它在等待索引。
    let mut neighbor_quads = extra.quads().as_ref().clone();
    for quad in &mut neighbor_quads {
        for corner in &mut quad.rgba {
            corner[1] *= 0.75;
        }
    }
    let neighbor_quads = Arc::new(neighbor_quads);
    let neighbor_picking =
        crate::picking::Index::build(extra.origin, Arc::clone(&neighbor_quads), groups).unwrap();
    extra = crate::spectator::MeshChunk::new(
        extra.key,
        extra.origin,
        neighbor_quads,
        Arc::new(neighbor_picking),
    );
    let neighbor_key = (extra.origin, true);
    pieces.push(extra);
    edit.chunks = pieces.into();
    let edit = Arc::new(edit);
    view.camera.position[0] -= 16.;
    *view.cache.lock().unwrap() = Some(Arc::clone(&edit));
    let edit_draw = draw_local(&view);
    let center = iced::Point::new(100., 100.);
    let viewport = crate::render_viewport::RenderViewport::one_to_one(size);
    let mut edited_early = false;
    for _ in 0..256 {
        let pixel = gpu.pixel_once(&edit_draw);
        let shown = view.presented.lock().unwrap().clone().unwrap();
        let hit = shown
            .pick(view.camera, center, viewport, view.quality)
            .unwrap();
        if hit.owner.unwrap().state.name == "sample:edited_solid" {
            assert!(
                pixel[0] > 0 && pixel[0] < 255,
                "替换后的 GPU 像素与源拾取一同切换"
            );
            edited_early |= gpu.pipeline.pending.is_some();
        }
        if gpu.pipeline.pending.is_none() {
            assert!(Arc::ptr_eq(&shown, &edit));
            break;
        }
    }
    assert!(
        edited_early,
        "已显示 Section 的替换不能等待另一 Section 的透明索引"
    );
    assert!(gpu.pipeline.pending.is_none());
    assert!(Arc::ptr_eq(
        &gpu.pipeline.section_orders[&fluid_key],
        &fluid
    ));
    let neighbor = Arc::clone(&gpu.pipeline.section_orders[&neighbor_key]);
    let neighbor_animation = Arc::clone(&gpu.pipeline.section_animations[&neighbor_key]);
    assert!(Arc::ptr_eq(
        &gpu.pipeline.section_animations[&fluid_key],
        &fluid_animation
    ));

    let mut wet_edit = (*edit).clone();
    let mut pieces = wet_edit.chunks.to_vec();
    pieces[1] = changed_part(&pieces[1], "sample:edited_water");
    wet_edit.chunks = pieces.into();
    *view.cache.lock().unwrap() = Some(Arc::new(wet_edit));
    view.camera.position[0] += 16.;
    gpu.pixel_primitive(draw_local(&view));
    let shown = view.presented.lock().unwrap().clone().unwrap();
    let hit = shown
        .pick(view.camera, center, viewport, view.quality)
        .unwrap();
    assert_eq!(hit.owner.unwrap().state.name, "sample:edited_water");
    assert!(!Arc::ptr_eq(
        &gpu.pipeline.section_orders[&fluid_key],
        &fluid
    ));
    assert!(Arc::ptr_eq(
        &gpu.pipeline.section_orders[&neighbor_key],
        &neighbor
    ));
    assert!(!Arc::ptr_eq(
        &gpu.pipeline.section_animations[&fluid_key],
        &fluid_animation
    ));
    assert!(Arc::ptr_eq(
        &gpu.pipeline.section_animations[&neighbor_key],
        &neighbor_animation
    ));

    gpu.pipeline.upload_limits = limits;
    view.camera.position[0] -= 16.;
    *view.cache.lock().unwrap() = Some(Arc::clone(&old));
    assert_pixel(gpu.pixel_primitive(draw(&view)), [255, 0, 0, 255]);

    let mut changed_atlas = (*red).clone();
    changed_atlas.pages = (0..2)
        .map(|_| {
            Arc::new(AtlasPage {
                width: 64,
                height: 64,
                rgba: [0, 0, 255, 255].repeat(64 * 64),
            })
        })
        .collect();
    changed_atlas.rebuild_raw_test_mips();
    let blue = Arc::new(changed_atlas);
    let replacement_name = if old_name == "minecraft:diamond_block" {
        "minecraft:gold_block"
    } else {
        "minecraft:diamond_block"
    };
    Arc::make_mut(&mut view.surface).voxels[0].state = mcedit2_world::sections::State {
        name: replacement_name.into(),
        properties: Vec::new(),
    }
    .into();
    let mut next = spectator::build_scene_cancellable(&view, &|| false).unwrap();
    next.atlas = Some(Arc::clone(&blue));
    next.origin[0] += 16;
    colorize(&mut next, 1, 12);
    let mut next = Arc::new(next);
    *view.cache.lock().unwrap() = Some(Arc::clone(&next));
    gpu.pipeline.upload_limits = upload::Limits {
        bytes: 4096,
        transfer: 2048,
        time: Duration::from_secs(1),
    };
    let mut primitive = draw(&view);
    let mut frames = 0;
    let mut carried = None;
    let mut followup = None;
    loop {
        gpu.pixel_once(&primitive);
        frames += 1;
        if frames == 1 {
            assert!(gpu.pipeline.pending.is_some());
            let mut latest = (*next).clone();
            let mut chunks = latest.chunks.to_vec();
            for chunk in &mut chunks {
                let mut groups = chunk.picking.source_groups().to_vec();
                for group in &mut groups {
                    if let Some(owner) = &mut group.owner {
                        Arc::make_mut(&mut owner.state)
                            .properties
                            .push(("stage".into(), "latest".into()));
                    }
                }
                chunk.picking = Arc::new(
                    crate::picking::Index::build(chunk.origin, Arc::clone(&chunk.quads()), groups)
                        .unwrap(),
                );
            }
            latest.chunks = chunks.into();
            next = Arc::new(latest);
            *view.cache.lock().unwrap() = Some(Arc::clone(&next));
            primitive = draw(&view);
        }
        // 在途几何变化时仍保留已经上传的源区块，只交接最新的完整场景。
        if carried.is_none() {
            if let Some(buffer) = gpu
                .pipeline
                .pending
                .as_ref()
                .and_then(|p| p.prepared_ordered_buffer())
            {
                let mut latest = (*next).clone();
                let mut chunks = latest.chunks.to_vec();
                let mut extra = chunks[0].clone();
                extra.origin[0] += 16;
                extra.key.0 += 1;
                let mut groups = extra.picking.source_groups().to_vec();
                for group in &mut groups {
                    if let Some(owner) = &mut group.owner {
                        owner.position[0] += 16;
                    }
                }
                extra.picking = Arc::new(
                    crate::picking::Index::build(extra.origin, Arc::clone(&extra.quads()), groups)
                        .unwrap(),
                );
                chunks.push(extra);
                latest.chunks = chunks.into();
                next = Arc::new(latest);
                *view.cache.lock().unwrap() = Some(Arc::clone(&next));
                primitive = draw(&view);
                carried = Some(buffer);
            }
        }
        // 几何目标冻结后继续送入更新，当前完整帧仍必须能够完成交接。
        if carried.is_some()
            && followup.is_none()
            && gpu.pipeline.pending.as_ref().is_some_and(|p| {
                p.target
                    .stream_mesh
                    .as_ref()
                    .is_some_and(|m| Arc::ptr_eq(m, &next))
            })
        {
            let mut later = (*next).clone();
            let mut chunks = later.chunks.to_vec();
            let mut extra = chunks[0].clone();
            extra.origin[0] += 32;
            extra.key.0 += 2;
            let mut groups = extra.picking.source_groups().to_vec();
            for group in &mut groups {
                if let Some(owner) = &mut group.owner {
                    owner.position[0] += 32;
                }
            }
            extra.picking = Arc::new(
                crate::picking::Index::build(extra.origin, Arc::clone(&extra.quads()), groups)
                    .unwrap(),
            );
            chunks.push(extra);
            later.chunks = chunks.into();
            let later = Arc::new(later);
            *view.cache.lock().unwrap() = Some(Arc::clone(&later));
            primitive = draw(&view);
            followup = Some(later);
        }
        assert!(gpu.pipeline.upload_bytes <= 4096);
        let displayed = view.presented.lock().unwrap().clone().unwrap();
        if gpu.pipeline.pending.is_none() {
            assert!(Arc::ptr_eq(&displayed, &next));
            assert!(Arc::ptr_eq(gpu.pipeline.atlas.as_ref().unwrap(), &blue));
            assert_eq!(
                displayed
                    .pick(
                        view.camera,
                        iced::Point::new(100., 100.),
                        crate::render_viewport::RenderViewport::one_to_one(size),
                        view.quality
                    )
                    .unwrap()
                    .owner
                    .unwrap()
                    .state
                    .name,
                replacement_name
            );
            assert!(displayed
                .pick(
                    view.camera,
                    iced::Point::new(100., 100.),
                    crate::render_viewport::RenderViewport::one_to_one(size),
                    view.quality,
                )
                .unwrap()
                .owner
                .unwrap()
                .state
                .properties
                .iter()
                .any(|(k, v)| k == "stage" && v == "latest"));
            break;
        }
        assert!(frames < 100);
        assert!(Arc::ptr_eq(&displayed, &old));
        assert_eq!(gpu.pixels, previous_pixels);
        assert!(Arc::ptr_eq(gpu.pipeline.atlas.as_ref().unwrap(), &red));
        assert_eq!(
            displayed
                .pick(
                    view.camera,
                    iced::Point::new(100., 100.),
                    crate::render_viewport::RenderViewport::one_to_one(size),
                    view.quality
                )
                .unwrap()
                .owner
                .unwrap()
                .state
                .name,
            old_name
        );
    }
    assert!(followup.is_some(), "持续来源更新必须覆盖已冻结的交接阶段");
    *view.cache.lock().unwrap() = Some(Arc::clone(&next));
    primitive = draw(&view);
    gpu.pixel_once(&primitive);
    assert!(gpu.pipeline.pending.is_none());
    let carried = carried.expect("在途来源变化应在已有完成区块时发生");
    assert!(
        gpu.pipeline.stream_chunks.values().any(|chunk| {
            chunk
                .ordered
                .segments
                .iter()
                .any(|segment| segment.buffer == carried)
        }),
        "新目标必须沿用已上传的源缓冲"
    );
    assert_eq!(
        gpu.pipeline.ordered_preparations, 1,
        "同一来源的多个放置只准备一次有序几何"
    );
    assert!(
        gpu.pipeline.stream_chunks.values().all(|chunk| {
            chunk
                .ordered
                .segments
                .first()
                .is_some_and(|segment| segment.buffer == carried)
        }),
        "各放置共享缓冲并保留独立偏移"
    );
    assert!(frames >= 8);
    assert_ne!(gpu.pixels, previous_pixels);
    let mut reference = GpuProbe::with_size(wgpu::Limits::default(), Size::new(200, 200));
    reference.pixel_primitive(primitive.clone());
    assert_eq!(gpu.pixels, reference.pixels);
    // 同源块换帧原点只分配独立 offset，透明上传期间旋转后重排，原缓冲复用。
    gpu.pipeline.upload_limits.bytes = 1536;
    gpu.pipeline.upload_limits.transfer = 1024;
    let resident = gpu
        .pipeline
        .stream_regions
        .values()
        .next()
        .unwrap()
        .buffers
        .segments[0]
        .buffer
        .clone();
    let resident_ordered = gpu
        .pipeline
        .stream_chunks
        .values()
        .next()
        .unwrap()
        .ordered
        .segments[0]
        .buffer
        .clone();
    let texture = gpu.pipeline.atlas_pages[0].0.clone();
    let mut shifted = (*next).clone();
    shifted.origin[0] -= 32;
    let shifted = Arc::new(shifted);
    *view.cache.lock().unwrap() = Some(Arc::clone(&shifted));
    let mut primitive = draw(&view);
    loop {
        gpu.pixel_once(&primitive);
        assert!(gpu.pipeline.upload_bytes <= 1536);
        if gpu
            .pipeline
            .pending
            .as_ref()
            .is_some_and(|p| p.uploading_transparency())
        {
            break;
        }
        assert!(gpu.pipeline.pending.is_some(), "透明上传必须经过分帧阶段");
    }
    view.camera.yaw += 10.;
    primitive = draw(&view);
    gpu.pixel_primitive(primitive.clone());
    assert_eq!(
        gpu.pipeline
            .stream_regions
            .values()
            .next()
            .unwrap()
            .buffers
            .segments[0]
            .buffer,
        resident
    );
    assert_eq!(gpu.pipeline.atlas_pages[0].0, texture);
    assert_eq!(
        gpu.pipeline
            .stream_chunks
            .values()
            .next()
            .unwrap()
            .ordered
            .segments[0]
            .buffer,
        resident_ordered
    );
    reference.pixel_primitive(primitive);
    assert!(
        gpu.pixels == reference.pixels,
        "换基与透明重排后的完整像素必须一致"
    );
    // 切换世界上下文时取消未交接的大网格；不再显示或点选旧世界。
    let mut queued = (*shifted).clone();
    colorize(&mut queued, 1, 48);
    let queued = Arc::new(queued);
    *view.cache.lock().unwrap() = Some(Arc::clone(&queued));
    gpu.pixel_once(&draw(&view));
    assert!(gpu.pipeline.pending.is_some());
    let faces = Arc::clone(&gpu.pipeline.world_faces);
    *view.cache.lock().unwrap() = Some(Arc::clone(&shifted));
    let mut interim = draw(&view);
    interim.presentation.as_mut().unwrap().awaiting_source = true;
    let pending_target = gpu
        .pipeline
        .pending
        .as_ref()
        .unwrap()
        .target
        .stream_mesh
        .clone()
        .unwrap();
    gpu.pixel_once(&interim);
    assert!(
        gpu.pipeline
            .pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(
                pending.target.stream_mesh.as_ref().unwrap(),
                &pending_target
            )),
        "临时沿用旧画面不能取消正在上传的新来源"
    );
    gpu.pixel_once(&draw(&view));
    assert!(
        gpu.pipeline.pending.is_none(),
        "返回显示来源必须取消过期候选上传"
    );
    assert_eq!(gpu.pipeline.upload_bytes, 0);
    assert!(Arc::ptr_eq(&faces, &gpu.pipeline.world_faces));
    assert!(Arc::ptr_eq(
        view.presented.lock().unwrap().as_ref().unwrap(),
        &shifted
    ));
    *view.cache.lock().unwrap() = Some(queued);
    gpu.pixel_once(&draw(&view));
    assert!(gpu.pipeline.pending.is_some());
    let prior_context = Arc::clone(&view.presented);
    view.presented = Default::default();
    *view.cache.lock().unwrap() = Some(Arc::clone(&old));
    gpu.pixel_once(&draw(&view));
    assert!(prior_context.lock().unwrap().is_none());
    assert!(view.presented.lock().unwrap().is_none());
    assert!(gpu.pixels.iter().all(|p| *p == [0; 4]));
    gpu.pixel_primitive(draw(&view));
    assert!(Arc::ptr_eq(
        view.presented.lock().unwrap().as_ref().unwrap(),
        &old
    ));
    assert_pixel(gpu.pixels[(200 * 100 + 100) as usize], [255, 0, 0, 255]);
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]
#[cfg(test)]
fn hardware_static_entity_models_share_depth_alpha_atlas_and_read_only_picking() {
    let (fixture, bank) = crate::entity_geometry::tests::assets();
    let source = crate::entity_geometry::tests::source("minecraft:creeper", [0.5, 0., 3.5]);
    let mut scene = crate::entity_geometry::tests::scene(bank.clone(), source.clone());
    let skin_color = bank
        .get("minecraft:entity/creeper/creeper")
        .unwrap()
        .pixel(0.5, 0.5);
    let size = Size::new(160., 128.);
    let draw = |scene: &crate::spectator::SpectatorScene| {
        shader::Program::draw(
            &GpuScene(SceneSource::Spectator(scene.clone())),
            &State::default(),
            mouse::Cursor::Unavailable,
            Rectangle::with_size(size),
        )
    };
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), Size::new(160, 128));
    assert_pixel(gpu.pixel_primitive(draw(&scene)), skin_color);
    let shown = scene.presented.lock().unwrap().clone().unwrap();
    let center = iced::Point::new(80., 64.);
    let hit = shown
        .pick(
            scene.camera,
            center,
            crate::render_viewport::RenderViewport::one_to_one(size),
            Quality::Fast,
        )
        .unwrap();
    assert!(hit.owner.is_none() && hit.marker.is_some());
    let entity_source = shown.entities.as_ref().unwrap().chunks[0].quads().clone();
    scene.camera.yaw = 4.;
    gpu.pixel_primitive(draw(&scene));
    assert!(gpu
        .pipeline
        .stream_chunks
        .values()
        .any(|c| Arc::ptr_eq(&c.source, &entity_source)));
    scene.camera.yaw = 0.;
    let view = crate::orthographic_world::View {
        angles: (0.3, 0.4),
        navigation: Default::default(),
        framing: Default::default(),
    };
    let primitive = shader::Program::draw(
        &GpuScene(SceneSource::WorldOrthographic(scene.clone(), view)),
        &State::default(),
        mouse::Cursor::Unavailable,
        Rectangle::with_size(size),
    );
    assert_eq!(primitive.camera.unwrap().position[3], 1.);
    gpu.pixel_primitive(primitive);
    assert!(gpu.pixels.iter().filter(|p| scene_pixel(p)).count() > 100);
    // 地形面位于实体之前，显示与拾取必须得到同一个最近源。
    let mut surface = (*scene.surface).clone();
    surface
        .voxels
        .push(mcedit2_world::surface::SurfaceVoxel::new(
            [0, 1, 1],
            mcedit2_world::surface::VoxelData {
                span: 1,
                fluid: None,
                state: mcedit2_world::sections::State {
                    name: "minecraft:stone".into(),
                    properties: Vec::new(),
                }
                .into(),
                biome: None,
                kind: mcedit2_world::surface::VoxelMaterial::Solid,
                staged: false,
                original_block_entity: false,
                faces: mcedit2_world::surface::ALL_FACES,
                light: Default::default(),
            },
        ));
    scene.surface = Arc::new(surface);
    let terrain = Arc::new(crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    *scene.cache.lock().unwrap() = Some(terrain);
    let pixel = gpu.pixel_primitive(draw(&scene));
    assert_ne!(pixel, skin_color);
    let shown = scene.presented.lock().unwrap().clone().unwrap();
    assert!(shown
        .pick(
            scene.camera,
            center,
            crate::render_viewport::RenderViewport::one_to_one(size),
            Quality::Fast
        )
        .unwrap()
        .owner
        .is_some());
    Arc::make_mut(&mut scene.surface).voxels.clear();
    // 同一皮肤被高优先级透明 PNG 替换后，GPU 遮罩和只读射线都穿过透明位置。
    use image::ImageEncoder;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[0; 64 * 64 * 4], 64, 64, image::ExtendedColorType::Rgba8)
        .unwrap();
    fixture.put("assets/minecraft/textures/entity/creeper/creeper.png", &png);
    let transparent = fixture.open();
    scene.texture_bank = Some(transparent.clone());
    scene.entity_frame = Some(Arc::new(
        crate::entity_geometry::Frame::build(
            source.clone(),
            &transparent,
            Some(3955),
            true,
            &|| false,
        )
        .unwrap(),
    ));
    *scene.cache.lock().unwrap() = Some(Arc::new(
        crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap(),
    ));
    assert_pixel(gpu.pixel_primitive(draw(&scene)), [0; 4]);
    let shown = scene.presented.lock().unwrap().clone().unwrap();
    assert!(shown
        .pick(
            scene.camera,
            center,
            crate::render_viewport::RenderViewport::one_to_one(size),
            Quality::Fast
        )
        .is_none());
    scene.show_textures = false;
    scene.entity_frame = Some(Arc::new(
        crate::entity_geometry::Frame::build(source, &transparent, Some(3955), false, &|| false)
            .unwrap(),
    ));
    assert_pixel(gpu.pixel_primitive(draw(&scene)), [255; 4]);
    scene.entity_enabled = false;
    assert_pixel(gpu.pixel_primitive(draw(&scene)), [0; 4]);
    scene.entity_enabled = true;
    assert_pixel(gpu.pixel_primitive(draw(&scene)), [255; 4]);
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_ordered_layers_keep_page_order_and_cutout_holes() {
    use crate::textures::AtlasPage;
    let (_, mut view, original, _) = crate::picking_integration_tests::fixture();
    let mut atlas = TextureAtlas::empty_test_atlas();
    let page = |rgba: [u8; 4]| {
        Arc::new(AtlasPage {
            width: 64,
            height: 64,
            rgba: rgba.repeat(64 * 64),
        })
    };
    atlas.pages = vec![
        page([0, 255, 0, 0]),
        page([0, 0, 255, 255]),
        page([255, 0, 0, 255]),
    ];
    let mut mesh = (*original).clone();
    let mut chunks = mesh.chunks.to_vec();
    for chunk in &mut chunks {
        let source = Arc::clone(&chunk.quads());
        *chunk = crate::spectator::MeshChunk::new(
            chunk.key,
            chunk.origin,
            Arc::new(
                (0..3)
                    .flat_map(|layer| {
                        source.iter().copied().map(move |mut q| {
                            q.mode = crate::material_mode::ORDERED_OPAQUE | 1 | 32;
                            q.uv = [[0.5; 2]; 4];
                            q.rgba = [[1.; 4]; 4];
                            q.atlas_page = 2 - layer;
                            q
                        })
                    })
                    .collect(),
            ),
            Arc::clone(&chunk.picking),
        );
        let owner = chunk.picking.source_groups()[0].owner.clone();
        chunk.picking = Arc::new(
            crate::picking::Index::build(
                chunk.origin,
                Arc::clone(&chunk.quads()),
                vec![crate::picking::Group {
                    quads: 0..chunk.quads().len(),
                    owner,
                }],
            )
            .unwrap(),
        );
    }
    mesh.chunks = chunks.into();
    let mesh = Arc::new(mesh);
    let size = Size::new(200., 200.);
    let draw = |view: &crate::spectator::SpectatorScene| {
        shader::Program::draw(
            &GpuScene(SceneSource::Spectator(view.clone())),
            &State::default(),
            mouse::Cursor::Unavailable,
            Rectangle::with_size(size),
        )
    };
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), Size::new(200, 200));
    let render = |gpu: &mut GpuProbe,
                  view: &mut crate::spectator::SpectatorScene,
                  mesh: &Arc<crate::spectator::StreamMesh>,
                  atlas: &TextureAtlas| {
        let mut next = (**mesh).clone();
        let mut raw_atlas = atlas.clone();
        raw_atlas.rebuild_raw_test_mips();
        next.atlas = Some(Arc::new(raw_atlas));
        *view.cache.lock().unwrap() = Some(Arc::new(next));
        gpu.pixel_primitive(draw(view))
    };
    assert_pixel(render(&mut gpu, &mut view, &mesh, &atlas), [0, 0, 255, 255]);
    assert!(gpu
        .pipeline
        .stream_chunks
        .values()
        .any(|chunk| !chunk.ordered.segments.is_empty() && chunk.bounds.is_some()));
    assert!(gpu.pipeline.world_faces.is_empty());
    let pick = view
        .presented
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .pick(
            view.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(size),
            Quality::Fast,
        )
        .unwrap();
    assert!(pick.owner.is_some());
    atlas.pages[1] = page([0, 0, 255, 0]);
    assert_pixel(render(&mut gpu, &mut view, &mesh, &atlas), [255, 0, 0, 255]);
    let through = view
        .presented
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .pick(
            view.camera,
            iced::Point::new(100., 100.),
            crate::render_viewport::RenderViewport::one_to_one(size),
            Quality::Fast,
        )
        .unwrap();
    assert_eq!(
        through.owner.as_ref().unwrap().position,
        pick.owner.as_ref().unwrap().position
    );
    atlas.pages[0] = page([0, 255, 0, 255]);
    assert_pixel(render(&mut gpu, &mut view, &mesh, &atlas), [0, 255, 0, 255]);
    atlas.pages[0] = page([0, 255, 0, 0]);
    atlas.pages[1] = page([0, 0, 255, 255]);
    atlas.pages.push(page([255, 255, 0, 255]));
    let wall = |delta: f32, blend: bool| {
        let mut next = (*mesh).clone();
        let mut chunks = next.chunks.to_vec();
        for chunk in &mut chunks {
            let mut quads = chunk.quads().as_ref().clone();
            quads.extend(
                original
                    .chunks
                    .iter()
                    .flat_map(|c| c.quads().iter())
                    .copied()
                    .map(|mut q| {
                        for p in &mut q.xyz {
                            p[2] += delta;
                        }
                        q.uv = [[0.5; 2]; 4];
                        q.rgba = [[1., 1., 1., if blend { 0.5 } else { 1. }]; 4];
                        q.mode = if blend { 2 } else { 0 };
                        q.atlas_page = 3;
                        q
                    }),
            );
            *chunk = crate::spectator::MeshChunk::new(
                chunk.key,
                chunk.origin,
                Arc::new(quads),
                Arc::clone(&chunk.picking),
            );
        }
        next.chunks = chunks.into();
        Arc::new(next)
    };
    assert_pixel(
        render(&mut gpu, &mut view, &wall(-2., false), &atlas),
        [255, 255, 0, 255],
    );
    assert_pixel(
        render(&mut gpu, &mut view, &wall(2., false), &atlas),
        [0, 0, 255, 255],
    );
    assert_pixel(
        render(&mut gpu, &mut view, &wall(2., true), &atlas),
        [0, 0, 255, 255],
    );
    assert_pixel(
        render(&mut gpu, &mut view, &wall(-2., true), &atlas),
        [191, 191, 64, 255],
    );
}

#[path = "gpu_greedy_tests.rs"]
mod greedy_tests;
#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]

fn hardware_brush_cursor_isolated_pixels_and_shared_atlas() {
    use crate::brush_cursor::{CursorPipeline, CursorPrimitive};
    let (_fixture, mesh, camera) = crate::brush_cursor::tests::gpu_fixture();
    let mut gpu = GpuProbe::with_size(wgpu::Limits::default(), Size::new(96, 96));
    let size = Size::new(96., 96.);
    let render = |camera: crate::world_camera::WorldCamera| {
        cursor_primitive(
            Some(Arc::clone(&mesh)),
            Some(camera.uniform_at_origin(mesh.origin, size)),
            Arc::new(Vec::new()),
            Quality::Fast,
            None,
        )
    };
    let world = render(camera);
    let center = gpu.pixel_primitive(world.clone());
    let world_pixels = gpu.pixels.clone();
    let mut first = CursorPipeline::new(&gpu.device, &gpu.queue, COLOR_FORMAT);
    let mut second = CursorPipeline::new(&gpu.device, &gpu.queue, COLOR_FORMAT);
    let first_draw = CursorPrimitive::<0>(render(camera));
    assert_pixel(gpu.cursor_pixel(&first_draw, &mut first), center);
    let first_pixels = gpu.pixels.clone();
    assert!(
        first_pixels.iter().any(|p| p[3] == 0),
        "透视光标背景必须透明"
    );
    assert!(
        first_pixels
            .iter()
            .any(|p| p[2] > 100 && p[0] == 0 && p[1] == 0 && p[3] == 255),
        "实际半宽模型必须使用蓝色纹理"
    );
    let crate::world_camera::WorldCamera::Free(mut moved) = camera else {
        unreachable!()
    };
    moved.position[0] += 4.;
    let second_draw = CursorPrimitive::<1>(render(moved.into()));
    assert_pixel(gpu.cursor_pixel(&second_draw, &mut second), [0; 4]);
    assert_ne!(gpu.pixels, first_pixels, "第二视图必须使用独立相机");
    let a = first.inner.as_ref().unwrap();
    let b = second.inner.as_ref().unwrap();
    assert!(!a.atlas_pages.is_empty());
    for ((world, first), second) in gpu
        .pipeline
        .atlas_pages
        .iter()
        .zip(&a.atlas_pages)
        .zip(&b.atlas_pages)
    {
        assert_eq!(world.0, first.0);
        assert_eq!(world.0, second.0);
        assert!(Arc::ptr_eq(
            world.2.shared.as_ref().unwrap(),
            first.2.shared.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            first.2.shared.as_ref().unwrap(),
            second.2.shared.as_ref().unwrap()
        ));
    }
    gpu.cursor_pixel(&first_draw, &mut first);
    assert_eq!(gpu.pixels, first_pixels, "另一视图不得覆盖已准备光标");
    let planar = CursorPrimitive::<2>(crate::brush_cursor::tests::planar_fixture(
        Arc::clone(&mesh),
        size,
    ));
    let mut grid = CursorPipeline::new(&gpu.device, &gpu.queue, COLOR_FORMAT);
    let pixel = gpu.cursor_pixel(&planar, &mut grid);
    assert!(
        pixel[2] > 100 && pixel[0] == 0 && pixel[1] == 0 && pixel[3] == 255,
        "二维投影必须保留实际材质：{pixel:?}"
    );
    assert!(gpu.pixels.iter().any(|p| p[3] == 0));
    assert_eq!(
        grid.inner.as_ref().unwrap().atlas_pages[0].0,
        gpu.pipeline.atlas_pages[0].0
    );
    gpu.pixel_primitive(world);
    assert_eq!(
        gpu.pixels, world_pixels,
        "光标相机和透明背景不得改变主世界画面"
    );
    let lease = Arc::downgrade(
        first.inner.as_ref().unwrap().atlas_pages[0]
            .2
            .shared
            .as_ref()
            .unwrap(),
    );
    first.trim();
    first.trim();
    second.trim();
    second.trim();
    grid.trim();
    grid.trim();
    assert!(first.inner.is_none() && second.inner.is_none() && grid.inner.is_none());
    drop(gpu);
    assert!(
        lease.upgrade().is_none(),
        "关闭视图后共享 GPU 图集不得被全局缓存保活"
    );
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]
fn hardware_behavior() {
    let (_fixture, bank) = crate::entity_geometry::tests::assets();
    let source = crate::entity_geometry::tests::source("minecraft:creeper", [0.5, 0., 3.5]);
    let scene = crate::entity_geometry::tests::scene(bank, source);
    let terrain = scene.cache.lock().unwrap().clone().unwrap();
    let mesh =
        crate::entity_geometry::compose(&terrain, scene.entity_frame.as_ref(), &scene.entity_cache);
    let mut mixed = (*mesh).clone();
    let mut mixed_chunks = mixed.chunks.to_vec();
    let mut quads = mixed_chunks[0].quads().as_ref().clone();
    let half = quads.len() / 2;
    for quad in quads.iter_mut().skip(half) {
        quad.mode = (quad.mode & !3) | 2;
    }
    mixed_chunks[0] = crate::spectator::MeshChunk::new(
        mixed_chunks[0].key,
        mixed_chunks[0].origin,
        Arc::new(quads),
        Arc::clone(&mixed_chunks[0].picking),
    );
    mixed.chunks = mixed_chunks.into();
    let mixed = Arc::new(mixed);
    let result = profile_world(
        Arc::clone(&mixed),
        scene.camera,
        WorldProfile {
            y32: false,
            material_hints: false,
            ..Default::default()
        },
    );
    let y32 = profile_world(mixed, scene.camera, WorldProfile::default());
    assert_eq!(
        result["pixel_hash"], y32["pixel_hash"],
        "高度分桶改变了小场景像素"
    );
    eprintln!("小场景生产通道 GPU 计时：{result}");
    assert_eq!(
        result["timestamp_supported"], true,
        "GPU 时间戳不可用：{result}"
    );
    let frames = result["frames"].as_array().unwrap();
    assert!(frames[..6]
        .iter()
        .all(|frame| frame["world_pass_issued"] == false && frame["opaque_ms"].is_null()));
    assert!(frames[6..]
        .iter()
        .all(|frame| frame["world_pass_issued"] == true
            && frame["opaque_ms"].as_f64().unwrap() > 0.
            && frame["transparent_ms"].as_f64().unwrap() > 0.
            && frame["composite_ms"].as_f64().unwrap() > 0.));
}

#[test]
#[ignore = "requires local GPU / installed Minecraft resources"]
fn test_greedy_tests_run() {
    greedy_tests::run();
}

#[cfg(test)]
pub(crate) fn block_on_for_application<F: std::future::Future>(future: F) -> F::Output {
    block_on(future)
}
