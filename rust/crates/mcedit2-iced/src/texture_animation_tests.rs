//! 通过生产目录资源、真实帧像素及不可变显示快照验证动画；接入已有类别。
use super::*;
use image::ImageEncoder;
pub(crate) fn fixture() -> (java_obj_tests::Fixture, Arc<TextureBank>) {
    let fixture = java_obj_tests::Fixture::new();
    for (id, a, b) in [
        ("live", [255, 0, 0, 255], [0, 0, 255, 0]),
        ("other", [0, 255, 0, 255], [20, 40, 60, 255]),
    ] {
        let pixels: Vec<u8> = [a, b]
            .into_iter()
            .flat_map(|p| std::iter::repeat_n(p, 16).flatten())
            .collect();
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&pixels, 4, 8, image::ExtendedColorType::Rgba8)
            .unwrap();
        fixture.put(&format!("assets/demo/textures/block/{id}.png"), &png);
        fixture.put(
            &format!("assets/demo/textures/block/{id}.png.mcmeta"),
            if id == "live" {
                br#"{"animation":{"frametime":2,"interpolate":"TRUE"}}"#
            } else {
                br#"{"animation":{"frames":[1,{"index":0,"time":3}]}}"#
            },
        );
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[9, 19, 29, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    fixture.put("assets/demo/textures/block/still.png", &png);
    fixture.put(
        "assets/demo/blockstates/assembly.json",
        br#"{"variants":{"":{"model":"demo:block/assembly"}}}"#,
    );
    let faces = ["up","down","north","south","east","west"].into_iter().map(|side| (side.to_owned(), serde_json::json!({"texture":match side {"up"=>"#still","down"=>"#other",_=>"#all"}}))).collect::<serde_json::Map<_,_>>();
    fixture.put("assets/demo/models/block/assembly.json", &serde_json::to_vec(&serde_json::json!({"render_type":"translucent","textures":{"all":"demo:block/live","other":"demo:block/other","still":"demo:block/still"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":faces}]})).unwrap());
    let bank = fixture.open();
    (fixture, bank)
}
pub(crate) fn verify_materials() {
    let (fixture, bank) = fixture();
    let source = std::fs::read(fixture.path.join("assets/demo/textures/block/live.png")).unwrap();
    let atlas = bank.atlas().unwrap();
    let rect = &atlas.entries["demo:live"];
    let uv = rect.sample(0.5, 0.5);
    assert_eq!(bank.animation_count(), 2);
    assert_eq!(bank.sprite_alpha("demo:live"), Some(AlphaMode::Cutout));
    let mut held = Vec::new();
    for (tick, expected) in [
        (0, [255, 0, 0, 255]),
        (1, [127, 0, 127, 255]),
        (2, [0, 0, 255, 0]),
        (3, [127, 0, 127, 0]),
        (4, [255, 0, 0, 255]),
        (u64::MAX, [127, 0, 127, 0]),
    ] {
        let snapshot = atlas.snapshot_at(tick).unwrap();
        assert!(snapshot
            .pages
            .iter()
            .zip(&atlas.pages)
            .all(|(a, b)| Arc::ptr_eq(a, b)));
        assert!(snapshot
            .mip_pages
            .iter()
            .zip(&atlas.mip_pages)
            .all(|(a, b)| Arc::ptr_eq(a, b)));
        assert!(snapshot.extends_exactly(atlas));
        assert!(
            Arc::ptr_eq(&snapshot.entries, &atlas.entries),
            "动画显示快照必须共享整包 UV 索引"
        );
        for lod in [0., 0.5, 1., 2.] {
            let p = snapshot
                .sample_lod(uv, rect.page, lod, true, 2)
                .unwrap()
                .map(|c| (c * 255.).round() as u8);
            assert_eq!(p, expected, "tick {tick}, lod {lod}");
        }
        let still = &snapshot.entries["demo:still"];
        assert_eq!(
            snapshot
                .sample_lod(still.sample(0.5, 0.5), still.page, 0., false, 0)
                .unwrap()
                .map(|c| (c * 255.).round() as u8),
            [9, 19, 29, 255]
        );
        held.push(snapshot);
    }
    assert_eq!(
        held[0].sample_lod(uv, rect.page, 0., false, 0).unwrap(),
        [1., 0., 0., 1.]
    );
    assert_eq!(
        std::fs::read(fixture.path.join("assets/demo/textures/block/live.png")).unwrap(),
        source
    );
    let one = atlas.snapshot_at(1).unwrap();
    let two = atlas.snapshot_at(2).unwrap();
    let set = atlas.animation.as_ref().unwrap();
    let key = *set
        .bindings
        .iter()
        .find(|(_, b)| b.uv == atlas.entries["demo:other"])
        .unwrap()
        .0;
    assert!(
        Arc::ptr_eq(
            &one.animation_frame.as_ref().unwrap().samples[&key],
            &two.animation_frame.as_ref().unwrap().samples[&key]
        ),
        "未换帧的素材不重建 mip"
    );
    let mut limited = (**atlas).clone();
    limited.budget_bytes = limited.allocated_bytes;
    limited.animation = None;
    limited.animation_frame = None;
    assert!(atlas_animation::Set::install(&mut limited, &bank.live_animations, &|| false).is_err());
    let sequence = texture_animation::Sequence {
        width: 1,
        height: 1,
        columns: 1,
        count: 2,
        duration: 1,
        steps: vec![
            texture_animation::Step {
                index: 1,
                duration: i32::MAX as u32,
            },
            texture_animation::Step {
                index: 0,
                duration: i32::MAX as u32,
            },
        ],
        interpolate: false,
    };
    assert_eq!(sequence.stamp(i32::MAX as u64).index, 0);
    assert_eq!(sequence.stamp(i32::MAX as u64 * 2).index, 1);
    fixture.put("assets/demo/textures/root.png", &source);
    fixture.put("assets/minecraft/font/default.json",br#"{"providers":[{"type":"bitmap","file":"demo:block/live.png","ascent":7,"height":8,"chars":["X"]},{"type":"bitmap","file":"demo:root.png","ascent":7,"height":8,"chars":["Y"]}]}"#);
    let dual = fixture.open();
    let image = dual.fonts.image("demo:live").unwrap();
    assert_eq!((image.width, image.height), (4, 8));
    assert_eq!(image.pixel(0.5, 0.25), [255, 0, 0, 255]);
    assert_eq!(image.pixel(0.5, 0.75), [0, 0, 255, 0]);
    assert!(dual.atlas().unwrap().animation.is_some());
    assert_eq!(
        dual.fonts.image("demo:block/live").unwrap().rgba,
        image.rgba
    );
    assert_eq!(dual.fonts.image("demo:root").unwrap().height, 8);
    assert!(dual.statistics.missing_references.is_empty());
}
pub(crate) fn verify_rendering() {
    let (_fixture, bank) = fixture();
    let scene = java_obj_tests::scene(Arc::clone(&bank));
    let mesh = Arc::new(crate::spectator::build_scene_cancellable(&scene, &|| false).unwrap());
    let chunks = mesh.chunks.iter().cloned().collect::<Vec<_>>();
    for (tick, visible) in [(0, true), (1, true), (2, false), (3, false), (4, true)] {
        let frame = crate::spectator::PresentedView {
            camera: scene.camera.into(),
            viewport: crate::render_viewport::RenderViewport::one_to_one(iced::Size::new(
                160., 160.,
            )),
            quality: crate::gpu_mesh::Quality::Fast,
            mesh: Arc::clone(&mesh),
            atlas: Some(Arc::new(
                mesh.atlas.as_ref().unwrap().snapshot_at(tick).unwrap(),
            )),
        };
        let hit = frame.pick(iced::Point::new(80., 80.));
        assert_eq!(
            hit.is_some(),
            visible,
            "tick {tick} 必须按实际显示帧的 Alpha 点选"
        );
        if let Some(hit) = hit {
            assert!(hit.owner.is_some());
        }
        let input = crate::brush_stroke::InputFrame::world(frame, iced::Point::new(80., 80.));
        assert_eq!(input.sample.unwrap().is_some(), visible);
    }
    assert!(chunks
        .iter()
        .zip(mesh.chunks.iter())
        .all(
            |(a, b)| Arc::ptr_eq(a.geometry(), b.geometry()) && Arc::ptr_eq(&a.picking, &b.picking)
        ));
}
pub(crate) fn verify_background() {
    use crate::projected_entities::{GridCache, GridKey, Mapping};
    let (_fixture, bank) = fixture();
    let snapshot = Arc::new(crate::scene_layers::Snapshot::build(
        (0, 0),
        1,
        std::iter::empty(),
        None,
    ));
    let mut frame =
        crate::entity_geometry::Frame::build(snapshot, &bank, Some(3955), true, &|| false).unwrap();
    frame.atlas = Arc::new(frame.atlas.snapshot_at(u64::MAX).unwrap());
    let key = GridKey {
        lighting: crate::lighting::Table::unlit(),
        frame: Arc::new(frame),
        mapping: Mapping::Orthogonal {
            origin: [0.; 3],
            screen: [0.; 2],
            right: [1., 0., 0.],
            down: [0., 1., 0.],
            depth: [0., 0., 1.],
            scale: 16.,
        },
        clip: ([-1.; 3], [1.; 3]),
        size: iced::Size::new(16., 16.),
        quality: crate::gpu_mesh::Quality::Fast,
        source_revision: 1,
    };
    let cache = Arc::new(GridCache::default());
    let wait = || {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while cache.pending() {
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    cache.request(key.clone(), Vec::new());
    wait();
    let (old, _) = cache.ready(&key).unwrap();
    *cache.presented.lock().unwrap() = Some(Arc::clone(&old));
    assert!(cache.needs(&key));
    let (release, wait_for_release) = std::sync::mpsc::channel();
    cache.request_with(key.clone(), move |_| {
        wait_for_release.recv().unwrap();
        Some(Vec::new())
    });
    assert!(!cache.needs(&key), "动画到时不能不断取消尚未完成的同源图片");
    assert!(cache.is_presented(&old));
    assert!(
        Arc::ptr_eq(&old, &cache.ready(&key).unwrap().0),
        "新动画图片完成前继续显示旧的完整帧"
    );
    release.send(()).unwrap();
    wait();
    assert!(!Arc::ptr_eq(&old, &cache.ready(&key).unwrap().0));
}
