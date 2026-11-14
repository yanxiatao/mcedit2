//! 使用真实旧世界、正式资源包和生产光标任务，接入既有类别。
use super::*;
use crate::brush_stroke::{Action, Sample};
use crate::textures::java_obj_tests::Fixture;
use image::ImageEncoder;
use std::io::Write;
fn resources() -> (Fixture, Arc<TextureBank>) {
    let fixture = Fixture::new();
    let jar = fixture.path.join("1.8.jar");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
    let mut entries = Vec::<(String, Vec<u8>)>::new();
    for (name, from, to, tinted, color) in [
        ("stone", [0, 0, 0], [8, 16, 16], false, [0, 0, 255, 255]),
        ("grass", [0, 0, 0], [16, 16, 16], true, [255, 255, 255, 255]),
    ] {
        let mut image = Vec::new();
        image::codecs::png::PngEncoder::new(&mut image)
            .write_image(&color, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        entries.push((format!("assets/minecraft/textures/block/{name}.png"), image));
        let mut faces = serde_json::Map::new();
        for side in ["up", "down", "east", "west", "north", "south"] {
            let mut face = serde_json::json!({"texture":"#all","cullface":side});
            if tinted {
                face["tintindex"] = serde_json::json!(0);
            }
            faces.insert(side.into(), face);
        }
        entries.push((format!("assets/minecraft/models/block/{name}.json"),serde_json::to_vec(&serde_json::json!({"textures":{"all":format!("minecraft:block/{name}")},"elements":[{"from":from,"to":to,"faces":faces}]})).unwrap()));
        let variant = if name == "grass" {
            "snowy=false"
        } else {
            "normal"
        };
        entries.push((format!("assets/minecraft/blockstates/{name}.json"),serde_json::to_vec(&serde_json::json!({"variants":{variant:{"model":format!("minecraft:block/{name}")}}})).unwrap()));
    }
    entries.push(("assets/mcedit2/biomes/numeric.json".into(),serde_json::to_vec(&serde_json::json!({"format":1,"source":"光标颜色验证表","versions":{"minimum":0,"maximum":1343,"unversioned":true},"blend_radius":0,"entries":[{"id":1,"name":"红","temperature":0.8,"rainfall":0.4,"water_color":16777215,"grass_color":16711680},{"id":2,"name":"绿","temperature":0.8,"rainfall":0.4,"water_color":16777215,"grass_color":65280}]})).unwrap()));
    for (name, bytes) in entries {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(&bytes).unwrap();
    }
    archive.finish().unwrap();
    let bank = Arc::new(TextureBank::open_stack_java(std::slice::from_ref(&jar)).unwrap());
    (fixture, bank)
}
fn setup() -> (Fixture, App, Source, Sample, WorldCamera) {
    let (fixture, bank) = resources();
    let (mut app, scene, mesh, hit) = crate::picking_integration_tests::fixture();
    app.current_session((0, 0)).unwrap();
    app.manual_textures = true;
    app.show_textures = true;
    app.texture_bank = Some(bank);
    // 场景快照的资源身份也必须属于此真实资源包。
    app.surface_assets = app.texture_bank.clone();
    app.surface_textures = true;
    for action in [
        Action::Enabled(true),
        Action::Size("1,1,1".into()),
        Action::Offset("0".into()),
    ] {
        let _ = crate::brush_stroke::update(&mut app, action);
    }
    app.edit_target = "numeric:1[data=0]".into();
    (
        fixture,
        app,
        Source::World(mesh),
        Sample::from_hit(&hit.hit).unwrap(),
        scene.camera.into(),
    )
}
fn key(app: &App, source: Source, sample: &Sample) -> Key {
    let mut config = crate::brush_stroke::config(app).unwrap();
    let context = config.visual.clone().expect("真实源会话已有光标上下文");
    let bounds = crate::brush_stroke::cursor_sample(&config, sample).unwrap();
    config.visual = None;
    Key {
        config,
        source,
        bounds,
        numeric: Some(sample.owner().unwrap().state.name.starts_with("numeric:")),
        session: context.session,
        bank: context.bank,
        show: context.show,
        quality: context.quality,
        language: context.language,
    }
}
fn wait(cache: &Cache) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while cache.state.lock().unwrap().running {
        assert!(std::time::Instant::now() < deadline, "光标任务未结束");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
pub(super) fn material_cursor() {
    let (_fixture, mut app, source, sample, camera) = setup();
    let disk = mcedit2_world::block_lookup::read_world_chunk_bytes(
        &app.worlds[0],
        app.sessions[&(0, 0)].dimension(),
        sample.owner().unwrap().position[0].div_euclid(16),
        sample.owner().unwrap().position[2].div_euclid(16),
    )
    .unwrap()
    .unwrap();
    let mesh = Arc::new(build(&key(&app, source.clone(), &sample), Some(true), &|| false).unwrap());
    let points = mesh
        .chunks
        .iter()
        .flat_map(|chunk| {
            chunk
                .quads()
                .iter()
                .flat_map(|q| q.xyz)
                .map(move |p| f64::from(chunk.origin[0]) + f64::from(p[0]))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(!points.is_empty());
    assert!(
        (points.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - points.iter().copied().fold(f64::INFINITY, f64::min)
            - 0.5)
            .abs()
            < 1e-6,
        "光标必须使用资源中的半宽模型"
    );
    assert!(mesh.chunks.iter().all(|c| c
        .picking
        .source_groups()
        .iter()
        .all(|g| g.owner.is_none())));
    let _ = crate::brush_stroke::update(&mut app, Action::Size("32,8,32".into()));
    let mut large = key(&app, source.clone(), &sample);
    let _ = crate::brush_stroke::update(&mut app, Action::Size("1,1,1".into()));
    let large_mesh = build(&large, Some(true), &|| false).unwrap();
    assert!(!large_mesh.chunks.is_empty());
    large.config.budget_mib = "1".into();
    assert!(build(&large, Some(true), &|| false)
        .unwrap_err()
        .contains("预算"));
    let before = app.spectator_presented.lock().unwrap().clone().unwrap();
    let size = Size::new(200., 200.);
    let _ = request(
        crate::brush_stroke::config(&app).as_ref(),
        Target::Perspective,
        Some((source.clone(), sample.clone())),
        camera.into(),
        size,
    );
    let cache = Arc::clone(&app.block_edit.cursor.slots[0]);
    wait(&cache);
    {
        let state = cache.state.lock().unwrap();
        assert!(state.ready.is_some(), "{:?}", state.error);
    }
    assert!(Arc::ptr_eq(
        &before,
        app.spectator_presented.lock().unwrap().as_ref().unwrap()
    ));
    let _ = crate::brush_stroke::update(&mut app, Action::Mode(Mode::Biome));
    let old = mcedit2_world::sections::decode_sections(&disk)
        .unwrap()
        .numeric_biome(
            sample.owner().unwrap().position[0].rem_euclid(16) as u8,
            sample.owner().unwrap().position[1],
            sample.owner().unwrap().position[2].rem_euclid(16) as u8,
        )
        .unwrap();
    let target = if old == 1 { 2 } else { 1 };
    app.block_edit.biome_target = target.to_string();
    let painted = build(&key(&app, source.clone(), &sample), Some(true), &|| false).unwrap();
    let expected = if target == 1 {
        [1., 0., 0.]
    } else {
        [0., 1., 0.]
    };
    assert!(painted
        .chunks
        .iter()
        .flat_map(|c| c.quads().iter().map(|q| q.rgba).collect::<Vec<_>>())
        .any(|rgba| rgba.iter().any(|c| c[..3] == expected)));
    app.block_edit.biome_target = old.to_string();
    assert!(
        !build(&key(&app, source, &sample), Some(true), &|| false)
            .unwrap()
            .chunks
            .is_empty(),
        "目标未改变也应显示有效光标"
    );
    assert!(!app.sessions[&(0, 0)].has_pending_changes());
    assert_eq!(
        mcedit2_world::block_lookup::read_world_chunk_bytes(
            &app.worlds[0],
            app.sessions[&(0, 0)].dimension(),
            sample.owner().unwrap().position[0].div_euclid(16),
            sample.owner().unwrap().position[2].div_euclid(16)
        )
        .unwrap()
        .unwrap(),
        disk
    );
}
pub(super) fn request_handoff() {
    let (_fixture, mut app, source, sample, camera) = setup();
    let size = Size::new(200., 200.);
    let config = crate::brush_stroke::config(&app).unwrap();
    let _ = request(
        Some(&config),
        Target::Perspective,
        Some((source.clone(), sample.clone())),
        camera.into(),
        size,
    );
    app.edit_target = "numeric:2[data=0]".into();
    let config = crate::brush_stroke::config(&app).unwrap();
    let _ = request(
        Some(&config),
        Target::Perspective,
        Some((source.clone(), sample.clone())),
        camera.into(),
        size,
    );
    let cache = Arc::clone(&app.block_edit.cursor.slots[0]);
    wait(&cache);
    {
        let s = cache.state.lock().unwrap();
        assert_eq!(s.key.as_ref().unwrap().config.target, "numeric:2[data=0]");
        assert!(s.ready.is_some(), "{:?}", s.error);
    }
    let _ = request(
        Some(&config),
        Target::Perspective,
        None,
        camera.into(),
        size,
    );
    assert!(cache.state.lock().unwrap().ready.is_none());
    app.edit_target = "broken".into();
    let config = crate::brush_stroke::config(&app).unwrap();
    let _ = request(
        Some(&config),
        Target::Perspective,
        Some((source.clone(), sample.clone())),
        camera.into(),
        size,
    );
    wait(&cache);
    {
        let s = cache.state.lock().unwrap();
        assert!(s.error.is_some());
        assert!(s.ready.is_none());
    }
    app.edit_target = "numeric:1[data=0]".into();
    let config = crate::brush_stroke::config(&app).unwrap();
    let _ = request(
        Some(&config),
        Target::Perspective,
        Some((source, sample)),
        camera.into(),
        size,
    );
    app.block_edit.cursor.clear();
    wait(&cache);
    let s = cache.state.lock().unwrap();
    assert!(s.ready.is_none());
    assert!(s.key.is_none());
}
/// GPU 类别使用与生产光标相同的资源解释及模型生成。
pub(crate) fn gpu_fixture() -> (Fixture, Arc<StreamMesh>, WorldCamera) {
    let (fixture, app, source, sample, camera) = setup();
    let mesh = Arc::new(build(&key(&app, source, &sample), Some(true), &|| false).unwrap());
    let WorldCamera::Free(mut camera) = camera else {
        unreachable!()
    };
    camera.position[0] -= 0.25;
    (fixture, mesh, camera.into())
}
pub(crate) fn planar_fixture(mesh: Arc<StreamMesh>, size: Size) -> crate::gpu_scene::Primitive {
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];
    for chunk in mesh.chunks.iter() {
        for quad in chunk.quads().iter() {
            for point in quad.xyz {
                for axis in 0..3 {
                    let value = f64::from(chunk.origin[axis]) + f64::from(point[axis]);
                    minimum[axis] = minimum[axis].min(value);
                    maximum[axis] = maximum[axis].max(value);
                }
            }
        }
    }
    let origin = std::array::from_fn(|i| (minimum[i] + maximum[i]) * 0.5);
    let mapping = Mapping::Orthogonal {
        origin,
        screen: [size.width * 0.5, size.height * 0.5],
        right: [1., 0., 0.],
        down: [0., -1., 0.],
        depth: [0., 0., -1.],
        scale: f64::from(size.height) * 0.6,
    };
    let cache = Arc::new(Cache::default());
    {
        let mut state = cache.state.lock().unwrap();
        state.ready = Some(mesh);
        state.projection = Some((Projection::Planar(mapping), size));
    }
    Layer::<2> {
        cache,
        quality: Quality::Fast,
    }
    .primitive(&LayerState::default(), Rectangle::with_size(size))
}
