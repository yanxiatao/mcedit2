//! 验证实际像素、深度和材质；另由硬件测试与生产 GPU 回读结果对照。
use super::*;
use crate::{isometric::Face, textures::java_obj_tests::Fixture};
use image::ImageEncoder;
use mcedit2_world::sections::State;

pub(crate) fn fixture(width: u32, height: u32, pixels: &[u8]) -> Fixture {
    let f = Fixture::new();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(pixels, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    f.put("assets/test/textures/block/tile.png", &png);
    f.put(
        "assets/test/blockstates/tile.json",
        br#"{"variants":{"":{"model":"test:block/tile"}}}"#,
    );
    f.put("assets/test/models/block/tile.json",br#"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"test:block/tile","uv":[0,0,16,16]}}}]}"#);
    f
}
pub(crate) fn texture_quad(bank: &crate::textures::TextureBank) -> [Vertex; 6] {
    let state = State {
        name: "test:tile".into(),
        properties: vec![],
    };
    let (uv, _, page) = bank
        .atlas_face_style_at(&state, Face::Top, 0, (0, 0, 0))
        .unwrap();
    let mut vertices = quad([1.; 4], 0.5, 0);
    for (v, i) in vertices.iter_mut().zip([0, 1, 2, 0, 2, 3]) {
        v.uv = uv[i];
        v.atlas_page = page;
    }
    vertices
}
pub(crate) fn quad(rgba: [f32; 4], depth: f32, mode: u32) -> [Vertex; 6] {
    let xy = [[-1., 1.], [1., 1.], [1., -1.], [-1., -1.]];
    [0, 1, 2, 0, 2, 3].map(|i| Vertex {
        xy: xy[i],
        depth,
        rgba,
        uv: [0.5; 2],
        mode,
        xyz: [0.; 3],
        world_mode: 0,
        atlas_page: 0,
        light: crate::lighting::FULLBRIGHT,
    })
}
fn raster(vertices: &[Vertex], size: u32) -> Raster {
    render(
        vertices,
        None,
        Size::new(size as f32, size as f32),
        Quality::Fast,
        &|| false,
    )
    .unwrap()
    .unwrap()
}
fn pixel(raster: &Raster, x: u32, y: u32) -> [u8; 4] {
    raster.rgba[((y * raster.width + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

fn canvas_preserves_all_source_texels_when_visible_face_count_exceeds_old_sampling_threshold() {
    let pixels = image::RgbaImage::from_fn(16, 16, |x, y| {
        image::Rgba([x as u8 * 15, y as u8 * 15, (x ^ y) as u8 * 15, 255])
    });
    let f = fixture(16, 16, &pixels);
    let bank = f.open();
    let mut quad = texture_quad(&bank);
    for v in &mut quad {
        v.depth = 0.1;
    }
    let size = Size::new(16., 16.);
    let expected = render(
        &quad,
        bank.atlas().map(Arc::as_ref),
        size,
        Quality::Fast,
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        expected.rgba,
        pixels.into_raw(),
        "必须保留 256 个源 texel，不能把每面变成几个均色块"
    );
    let mut many = quad.to_vec();
    let mut behind = quad;
    for v in &mut behind {
        v.depth = 0.9;
    }
    for _ in 0..12_001 {
        many.extend(behind);
    }
    let actual = render(
        &many,
        bank.atlas().map(Arc::as_ref),
        size,
        Quality::Fast,
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(actual.rgba, expected.rgba);
}

fn transparent_quad_shared_diagonal_is_blended_once_and_image_alpha_is_straight() {
    let raster = raster(&quad([1., 0., 0., 0.5], 0.5, 2), 16);
    assert!(
        raster.rgba.chunks_exact(4).all(|p| p == [128, 0, 0, 128]),
        "共享三角边不能出现双重透明度或缺口"
    );
    let Handle::Rgba { pixels, .. } = raster.into_image() else {
        panic!("应直接上传 RGBA，无图片编码往返")
    };
    assert!(pixels.chunks_exact(4).all(|p| p == [255, 0, 0, 128]));
}

fn per_pixel_depth_handles_intersections_opaque_occlusion_and_transparency() {
    let mut vertices = quad([0., 0., 1., 1.], 0.5, 0).to_vec();
    let mut sloped = quad([1., 0., 0., 1.], 0.5, 0);
    for (v, z) in sloped.iter_mut().zip([0.1, 0.9, 0.9, 0.1, 0.9, 0.1]) {
        v.depth = z;
    }
    vertices.extend(sloped);
    vertices.extend(quad([0., 1., 0., 0.5], 0.3, 2));
    let raster = raster(&vertices, 16);
    assert_eq!(pixel(&raster, 1, 8), [255, 0, 0, 255]);
    assert_eq!(pixel(&raster, 14, 8), [0, 128, 128, 255]);
    let mut reversed = vertices[6..12].to_vec();
    reversed.extend(&vertices[..6]);
    reversed.extend(&vertices[12..]);
    assert_eq!(
        raster.rgba,
        self::raster(&reversed, 16).rgba,
        "不透明交叠不依赖面中心排序"
    );
}

fn constant_cutoff_and_far_depth_do_not_develop_interpolation_holes() {
    for size in [7, 17, 31] {
        for depth in [0., 0.5, 1.] {
            let raster = raster(&quad([1., 0., 0., 0.5], depth, 1), size);
            assert!(
                raster.rgba.chunks_exact(4).all(|p| p == [255, 0, 0, 255]),
                "常量属性插值应保持 Alpha=0.5 和原深度，size={size},depth={depth}"
            );
        }
    }
}

fn filtering_cutoff_authored_opaque_and_single_sided_flags_use_actual_pixels() {
    let f = fixture(
        2,
        2,
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    );
    let bank = f.open();
    let quad = texture_quad(&bank);
    let uv = std::array::from_fn(|i| (quad[0].uv[i] + quad[2].uv[i]) * 0.5);
    let filtered = sample(
        bank.atlas().map(Arc::as_ref),
        uv,
        quad[0].atlas_page,
        Quality::Detailed,
    )
    .unwrap();
    for c in &filtered[..3] {
        assert!((*c - 0.5).abs() < 0.000_01);
    }
    assert_ne!(
        sample(
            bank.atlas().map(Arc::as_ref),
            uv,
            quad[0].atlas_page,
            Quality::Fast
        )
        .unwrap(),
        filtered
    );
    let mask = crate::material_mode::encode(0, Some("MASK"), 0.75, true);
    assert!(raster(&self::quad([1., 1., 1., 0.74], 0.5, mask), 16)
        .rgba
        .iter()
        .all(|&b| b == 0));
    assert_eq!(
        pixel(
            &raster(&self::quad([1., 1., 1., 0.76], 0.5, mask), 16),
            8,
            8
        ),
        [255; 4]
    );
    assert_eq!(
        pixel(&raster(&self::quad([1., 0., 0., 0.], 0.5, 8), 16), 8, 8),
        [255, 0, 0, 255]
    );
    let mut back = self::quad([1.; 4], 0.5, crate::material_mode::SINGLE_SIDED);
    assert!(raster(&back, 16).rgba.iter().all(|&b| b == 0));
    back.swap(1, 2);
    back.swap(4, 5);
    assert!(raster(&back, 16)
        .rgba
        .chunks_exact(4)
        .all(|p| p == [255; 4]));
    for cutoff in 0..=255 {
        let mode = 1 | crate::material_mode::CUSTOM_ALPHA_CUTOFF | (cutoff << 8);
        assert_eq!(
            crate::material_mode::covered_alpha_f32(mode, cutoff as f32 / 255.),
            Some(1.)
        );
    }
}

fn cancelled_invalid_and_resized_canvas_frames_never_publish_partial_images() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = AtomicUsize::new(0);
    assert!(render(
        &quad([1.; 4], 0.5, 0),
        None,
        Size::new(256., 256.),
        Quality::Fast,
        &|| calls.fetch_add(1, Ordering::Relaxed) > 5
    )
    .unwrap()
    .is_none());
    for size in [
        Size::ZERO,
        Size::new(f32::NAN, 16.),
        Size::new(f32::INFINITY, 16.),
    ] {
        assert!(render(&[], None, size, Quality::Fast, &|| false).is_err());
    }
    let mut invalid = quad([1.; 4], 0.5, 0);
    invalid[0].xy[0] = f32::NAN;
    assert!(
        render(&invalid, None, Size::new(16., 16.), Quality::Fast, &|| {
            false
        })
        .is_err()
    );
    let f = crate::textures::java_obj_tests::fixture();
    let view = crate::textures::java_obj_tests::isometric_scene(f.open());
    let size = Size::new(64., 64.);
    assert!(view.canvas_image_cancellable(size, &|| true).is_none());
    assert!(view.canvas_image(size).is_none());
    let a = view
        .canvas_image_cancellable(size, &|| false)
        .unwrap()
        .unwrap();
    let b = view
        .clone()
        .canvas_image_cancellable(size, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(a.id(), b.id());
    let mut filtered = view.clone();
    filtered.gpu_quality = if view.gpu_quality == Quality::Detailed {
        Quality::Fast
    } else {
        Quality::Detailed
    };
    assert!(
        filtered.canvas_image(size).is_none(),
        "采样器改变不能命中旧图片"
    );
    let filtered = filtered
        .canvas_image_cancellable(size, &|| false)
        .unwrap()
        .unwrap();
    assert_ne!(a.id(), filtered.id());
    assert!(
        view.canvas_image(size).is_none(),
        "不同采样器的图片不能混用"
    );
    let c = view
        .canvas_image_cancellable(Size::new(65., 64.), &|| false)
        .unwrap()
        .unwrap();
    assert_ne!(a.id(), c.id());
}

fn canvas_worker_schedules_redraw_and_commits_a_complete_frame_without_mouse_input() {
    use iced::{widget::canvas, Rectangle};
    let f = crate::textures::java_obj_tests::fixture();
    let mut view = crate::textures::java_obj_tests::isometric_scene(f.open());
    view.gpu_faces = false;
    let mut state = crate::isometric::CanvasState::default();
    let size = Size::new(64., 64.);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let event = canvas::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let action = canvas::Program::<crate::Message>::update(
            &view,
            &mut state,
            &event,
            Rectangle::with_size(size),
            iced::mouse::Cursor::Unavailable,
        );
        if let Some(result) = view.canvas_image(size) {
            let Handle::Rgba {
                width,
                height,
                pixels,
                ..
            } = result.unwrap()
            else {
                panic!("应为完整 RGBA 图片")
            };
            assert_eq!((width, height), (64, 64));
            assert_eq!(pixels.len(), 64 * 64 * 4);
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 0));
            break;
        }
        assert!(action.is_some(), "未就绪时必须安排后续帧，不能等鼠标唤醒");
        assert!(
            std::time::Instant::now() < deadline,
            "后台 Canvas 任务未完成"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn filtered_and_repeated_texture_picking_matches_the_visible_source() {
    use crate::{isometric::IsometricSurface, textures::java_obj_tests};
    use iced::Point;
    fn point(view: &IsometricSurface, u: f32, v: f32) -> Point {
        let faces = view.faces_cached(Size::new(256., 256.));
        let face = faces.iter().find(|f| f.face == Face::Top).unwrap();
        let w = [(1. - u) * (1. - v), u * (1. - v), u * v, (1. - u) * v];
        Point::new(
            (0..4).map(|i| face.corners[i].x * w[i]).sum(),
            (0..4).map(|i| face.corners[i].y * w[i]).sum(),
        )
    }
    let pixels = image::RgbaImage::from_fn(16, 16, |_, y| {
        image::Rgba([255, 255, 255, if y < 8 { 0 } else { 255 }])
    });
    let f = fixture(16, 16, &pixels);
    f.put("assets/test/models/block/tile.json",br#"{"render_type":"cutout","elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"test:block/tile"}}}]}"#);
    let mut view = java_obj_tests::isometric_scene(f.open());
    Arc::make_mut(&mut view.surface).voxels[0].state = State {
        name: "test:tile".into(),
        properties: vec![],
    }
    .into();
    let p = point(&view, 0.5, 0.49);
    view.gpu_quality = Quality::Fast;
    assert!(view.pick_face(p, Size::new(256., 256.)).is_none());
    view.gpu_quality = Quality::Detailed;
    assert_eq!(
        view.pick_face(p, Size::new(256., 256.)).unwrap().0,
        (0, 0, 0)
    );
    let pixels = image::RgbaImage::from_fn(16, 16, |x, _| {
        image::Rgba([255, 255, 255, if x < 8 { 0 } else { 255 }])
    });
    let f = fixture(16, 16, &pixels);
    f.put("assets/test/models/block/tile.json",br#"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"test:block/tile","uv":[0,0,32,16]}}}]}"#);
    let mut view = java_obj_tests::isometric_scene(f.open());
    Arc::make_mut(&mut view.surface).voxels[0].state = State {
        name: "test:tile".into(),
        properties: vec![],
    }
    .into();
    assert!(
        view.pick_face(point(&view, 0.6, 0.5), Size::new(256., 256.))
            .is_none(),
        "重复后的透明区不能按 clamp 后的不透明边缘拾取"
    );
    assert_eq!(
        view.pick_face(point(&view, 0.85, 0.5), Size::new(256., 256.))
            .unwrap()
            .0,
        (0, 0, 0)
    );
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Rendering {
        canvas_preserves_all_source_texels_when_visible_face_count_exceeds_old_sampling_threshold();
        transparent_quad_shared_diagonal_is_blended_once_and_image_alpha_is_straight();
        per_pixel_depth_handles_intersections_opaque_occlusion_and_transparency();
        constant_cutoff_and_far_depth_do_not_develop_interpolation_holes();
        filtering_cutoff_authored_opaque_and_single_sided_flags_use_actual_pixels();
        cancelled_invalid_and_resized_canvas_frames_never_publish_partial_images();
        canvas_worker_schedules_redraw_and_commits_a_complete_frame_without_mouse_input();
        filtered_and_repeated_texture_picking_matches_the_visible_source();
    }
}
