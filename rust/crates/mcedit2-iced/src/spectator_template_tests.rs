use super::*;
use crate::textures::java_obj_tests::{fixture, scene, Fixture};
use mcedit2_world::{
    sections::State,
    surface::{SurfaceView, SurfaceVoxel, VoxelMaterial, ALL_FACES},
};

fn pack() -> Fixture {
    let f = fixture();
    let elements: Vec<_> = (0..70)
        .map(|index| {
            let x = index as f32 * 0.2;
            serde_json::json!({"from":[x,0,0],"to":[x+0.1,1,1],"faces":{
                "north":{"texture":"#all","cullface":"north"},"up":{"texture":"#all"}
            }})
        })
        .collect();
    f.put(
        "assets/demo/blockstates/many_faces.json",
        br#"{"variants":{"":{"model":"demo:block/many_faces"}}}"#,
    );
    f.put(
        "assets/demo/models/block/many_faces.json",
        &serde_json::to_vec(
            &serde_json::json!({"textures":{"all":"demo:item/red"},"elements":elements}),
        )
        .unwrap(),
    );
    f.put(
        "assets/demo/blockstates/details.json",
        br#"{"variants":{"":{"model":"demo:block/details","y":90,"uvlock":true}}}"#,
    );
    f.put("assets/demo/models/block/details.json",br##"{"loader":"neoforge:composite","children":{
        "rotated":{"loader":"neoforge:elements","transform":{"translation":[-0.2,0.1,0.3],"rotation":[15,35,-10],"scale":[-1,0.75,1],"origin":"center"},"textures":{"all":"demo:item/red"},"render_type":"cutout","elements":[
            {"from":[-4,0,0],"to":[20,16,16],"rotation":{"axis":"y","angle":22.5,"origin":[8,8,8],"rescale":true},"faces":{"north":{"texture":"#all","uv":[-16,-16,32,32],"rotation":90},"up":{"texture":"#all","cullface":"up"}}}]},
        "inside":{"loader":"neoforge:elements","textures":{"all":"demo:item/blue"},"render_type":"translucent","elements":[
            {"from":[16,0,0],"to":[0,16,16],"faces":{"east":{"texture":"#all","uv":[0,0,16,16]},"south":{"texture":"#all"}}}]}
    }}"##);
    f.put("assets/demo/blockstates/weighted.json",br#"{"variants":{"": [{"model":"demo:block/details","weight":2},{"model":"demo:block/assembly","weight":3,"y":180}]}}"#);
    f.put(
        "assets/demo/blockstates/hidden.json",
        br#"{"multipart":[{"when":{"loaded":true},"apply":{"model":"demo:block/assembly"}}]}"#,
    );
    f.put(
        "assets/minecraft/blockstates/oak_leaves.json",
        br#"{"variants":{"":{"model":"minecraft:block/oak_leaves"}}}"#,
    );
    f.put("assets/minecraft/models/block/oak_leaves.json",br##"{"textures":{"all":"demo:item/red"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all","tintindex":0,"cullface":"up"},"north":{"texture":"#all","tintindex":0,"cullface":"north"}}}]}"##);
    f
}

fn chunks(view: &SpectatorScene, reference: bool) -> Vec<MeshChunk> {
    let mut view = view.clone();
    view.chunk_cache = Default::default();
    let sources = [view.surface.voxels.as_slice()];
    let neighbors = crate::spectator_stream::ExactNeighbors::new(&sources);
    build_sources(
        &view,
        &sources,
        SourceFrame {
            biomes: &view.surface.biomes,
            origin: [view.surface.x, view.surface.y_min, view.surface.z],
            y_max: view.surface.y_max,
            reference,
        },
        Some(&neighbors),
        None,
        &|| false,
        &mut |_| {},
    )
    .unwrap()
}
fn compare(view: &SpectatorScene) {
    let compiled = chunks(view, false);
    let reference = chunks(view, true);
    assert_eq!(
        compiled.len(),
        reference.len(),
        "模板分组 {:?}；逐面分组 {:?}",
        compiled
            .iter()
            .map(|c| (c.key, c.quads().len()))
            .collect::<Vec<_>>(),
        reference
            .iter()
            .map(|c| (c.key, c.quads().len()))
            .collect::<Vec<_>>()
    );
    for (a, b) in compiled.iter().zip(&reference) {
        assert_eq!(a.key, b.key);
        assert_eq!(a.origin, b.origin);
        assert_eq!(a.quads().len(), b.quads().len(), "区块 {:?}", a.key);
        if let Some((i, (a, b))) = a
            .quads()
            .iter()
            .zip(b.quads().iter())
            .enumerate()
            .find(|(_, (a, b))| bytemuck::bytes_of(*a) != bytemuck::bytes_of(*b))
        {
            panic!("第 {i} 个源面字节不同：模板 {a:?}；逐面 {b:?}");
        }
        assert_eq!(a.picking.source_groups(), b.picking.source_groups());
        assert_eq!(a.picking.world_bounds(), b.picking.world_bounds());
    }
}
fn state(name: &str, properties: &[(&str, &str)]) -> State {
    State {
        name: name.into(),
        properties: properties
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

fn compiled_templates_match_reference_bytes_for_masks_models_biomes_fluids_and_lod() {
    let fixture = pack();
    let bank = fixture.open();
    let cases = [
        (state("demo:many_faces", &[]), VoxelMaterial::Solid),
        (state("demo:assembly", &[]), VoxelMaterial::Solid),
        (state("demo:details", &[]), VoxelMaterial::Solid),
        (state("demo:weighted", &[]), VoxelMaterial::Solid),
        (
            state("demo:hidden", &[("loaded", "true")]),
            VoxelMaterial::Solid,
        ),
        (
            state("demo:hidden", &[("loaded", "false")]),
            VoxelMaterial::Solid,
        ),
        (state("minecraft:oak_leaves", &[]), VoxelMaterial::Foliage),
        (
            state(
                "minecraft:oak_stairs",
                &[("facing", "west"), ("half", "top")],
            ),
            VoxelMaterial::Solid,
        ),
        (
            state("minecraft:water", &[("level", "7")]),
            VoxelMaterial::Liquid,
        ),
        (state("demo:unknown", &[]), VoxelMaterial::Transparent),
    ];
    let mut voxels = Vec::new();
    for (case, (state, kind)) in cases.iter().enumerate() {
        for mask in 0..=ALL_FACES {
            let n = case * 64 + mask as usize;
            voxels.push(SurfaceVoxel::new(
                [
                    (n % 16) as i32 * 2 - 16,
                    64 + (n / 256) as i32,
                    (n / 16) as i32 * 2 - 32,
                ],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: state.clone().into(),
                    kind: *kind,
                    biome: Some(
                        if mask % 2 == 0 {
                            "minecraft:swamp"
                        } else {
                            "minecraft:badlands"
                        }
                        .into(),
                    ),
                    staged: mask % 3 == 0,
                    original_block_entity: mask % 5 == 0,
                    faces: mask,
                    light: Default::default(),
                },
            ));
        }
    }
    let mut border = voxels[1].clone();
    border.x = 29_999_998;
    border.y = -63;
    border.z = -29_999_998;
    voxels.push(border);
    let mut fluid = voxels
        .iter()
        .find(|v| v.kind == VoxelMaterial::Liquid && v.faces == ALL_FACES)
        .unwrap()
        .clone();
    fluid.x = 0;
    fluid.y = 63;
    fluid.z = 0;
    fluid.state = state("minecraft:water", &[("level", "3")]).into();
    voxels.push(fluid.clone());
    fluid.x = 1;
    fluid.y = 64;
    fluid.z = 1;
    voxels.push(fluid);
    for (x, y) in [(48, 65), (52, 60)] {
        let mut lod = voxels[1].clone();
        lod.x = x;
        lod.y = y;
        lod.z = 0;
        lod.span = 4;
        lod.kind = VoxelMaterial::Solid;
        lod.faces = ALL_FACES;
        voxels.push(lod);
    }
    let mut view = scene(bank);
    view.surface = Arc::new(SurfaceView {
        biomes: Default::default(),
        x: -16,
        z: -32,
        y_min: -64,
        y_max: 319,
        width: 96,
        depth: 128,
        voxels,
        missing_columns: 0,
        source_chunks: 1,
        source_tiles: vec![],
    });
    for enabled in [true, false] {
        view.show_textures = enabled;
        compare(&view);
    }
}

fn variant_cache_keeps_actual_coordinate_choices_and_inactive_multipart_branches() {
    let fixture = pack();
    let bank = fixture.open();
    let fixed = state("demo:assembly", &[]);
    let weighted = state("demo:weighted", &[]);
    let hidden = state("demo:hidden", &[("loaded", "false")]);
    assert!(!bank.model_position_dependent(&fixed));
    assert!(bank.model_position_dependent(&weighted));
    assert!(!bank.model_position_dependent(&hidden));
    let mut variants = Variants::default();
    let mut choices = std::collections::BTreeSet::new();
    for state in [&fixed, &weighted, &hidden] {
        for n in -64..64 {
            let pos = (n, -63, n * 17);
            let expected = bank.model_variant_key(state, pos);
            assert_eq!(variants.at(Some(&bank), state, pos), expected);
            if state == &weighted {
                choices.insert(expected);
            }
        }
    }
    assert!(choices.len() > 1);
}

fn cancelling_template_preparation_does_not_return_partial_geometry() {
    let fixture = pack();
    let view = scene(fixture.open());
    let calls = std::cell::Cell::new(0);
    assert!(
        Template::compile(&view, &view.surface.voxels[0], [[0.5; 2]; 4], &|| {
            calls.set(calls.get() + 1);
            calls.get() > 1
        })
        .is_none()
    );
    let template =
        Template::compile(&view, &view.surface.voxels[0], [[0.5; 2]; 4], &|| false).unwrap();
    let mut out = Vec::new();
    assert!(template
        .append(
            &mut out,
            ALL_FACES,
            [0.0; 3],
            &view.surface.voxels[0],
            &mut None,
            &|| true
        )
        .is_none());
    assert!(out.is_empty());
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::WorldGeometry {
        compiled_templates_match_reference_bytes_for_masks_models_biomes_fluids_and_lod();
        variant_cache_keeps_actual_coordinate_choices_and_inactive_multipart_branches();
        cancelling_template_preparation_does_not_return_partial_geometry();
    }
}
