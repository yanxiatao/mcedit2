//! 女巫的零时刻鼻部、真实持有物条件及分版本物品层矩阵。
use super::*;
use crate::root_transform::Affine;
use mcedit2_world::entity_equipment::{Slot, Source, Stack};

pub(super) fn poses(
    visual: &EntityVisual,
    pitch: f32,
) -> Result<BTreeMap<String, PartPose>, String> {
    let source = Source::read(visual, false)?;
    let holding = source.items.contains_key(&Slot::MainHand);
    Ok(BTreeMap::from([
        (
            "/head".into(),
            PartPose {
                rotation: Some([pitch.to_radians(), 0., 0.]),
                ..Default::default()
            },
        ),
        (
            "/head/nose".into(),
            PartPose {
                // 旧版保存为 render offset，现代直接改 pivot；身体的组合坐标相同。
                pivot: Some(if holding {
                    [0., 1., -1.5]
                } else {
                    [0., -2., 0.]
                }),
                rotation: Some([if holding { -0.9 } else { 0. }, 0., 2.5 * 0.017453292]),
                ..Default::default()
            },
        ),
    ]))
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    let rig = Rig::read(catalog, Kind::Entity, Some("minecraft:witch"), visual, None)?;
    npc::render(
        catalog,
        bank,
        rig.layer,
        "minecraft:entity/witch",
        &rig.poses,
        rig.style,
    )
}

fn rotation(angles: [f32; 3]) -> Result<Affine, String> {
    // Affine 的坐标原点约定在这里抵消，使本函数直接表示原生 PoseStack 旋转。
    Affine::parse(&serde_json::json!({"rotation": angles, "origin": [0.5, 0.5, 0.5]}))
}
fn scaling(size: [f32; 3]) -> Affine {
    Affine([
        [size[0], 0., 0., 0.],
        [0., size[1], 0., 0.],
        [0., 0., size[2], 0.],
    ])
}
fn nose_transform(catalog: &Catalog, rig: &Rig) -> Result<Affine, String> {
    let legacy = catalog.version == "1.12.2";
    let mut poses = rig.poses.clone();
    if legacy {
        // ModelRenderer.postRender 不使用 render offset，也不自动应用父部件。
        poses
            .get_mut("/head/nose")
            .expect("女巫鼻部姿态已建立")
            .pivot = Some([0., -2., 0.]);
    }
    let point = |point| {
        if legacy {
            catalog.local_attachment_point(rig.layer, "/head/nose", &poses, point)
        } else {
            catalog.attachment_point(rig.layer, "/head/nose", &poses, point)
        }
    };
    let origin = point([0.; 3])?;
    let mut matrix = Affine::translation(origin);
    for axis in 0..3 {
        let mut basis = [0.; 3];
        basis[axis] = 1.;
        let basis = point(basis)?;
        for row in 0..3 {
            matrix.0[row][axis] = basis[row] - origin[row];
        }
    }
    Ok(matrix)
}

pub(super) fn held_transform(
    catalog: &Catalog,
    rig: &Rig,
    stack: &Stack,
) -> Result<(&'static str, Affine), String> {
    if catalog.version != "1.12.2" {
        let transform = if stack.id == "minecraft:potion" {
            nose_transform(catalog, rig)?
                .compose(Affine::translation([0.0625, 0.25, 0.]))?
                .compose(rotation([0., 0., 180.])?)?
                .compose(rotation([140., 0., 0.])?)?
                .compose(rotation([0., 0., 10.])?)?
                .compose(Affine::translation([0., -0.4, 0.4]))?
                .compose(rig::crossed_transform())?
        } else {
            rig::crossed_transform()
        };
        return Ok(("ground", transform));
    }
    let properties = crate::item_catalog::get(Some(1343))
        .and_then(|catalog| catalog.items.get(&stack.id))
        .ok_or_else(|| format!("旧女巫持有物 {} 不在准确物品注册表中", stack.id))?
        .legacy_held
        .as_ref()
        .ok_or("旧物品持有层属性尚未由准确客户端生成")?;
    let mut transform =
        nose_transform(catalog, rig)?.compose(Affine::translation([-0.0625, 0.53125, 0.21875]))?;
    if properties.block_entity {
        transform = transform
            .compose(Affine::translation([0., 0.0625, -0.25]))?
            .compose(rotation([30., 0., 0.])?)?
            .compose(rotation([0., -5., 0.])?)?
            .compose(scaling([0.375, -0.375, 0.375]))?;
    } else if stack.id == "minecraft:bow" {
        transform = transform
            .compose(Affine::translation([0., 0.125, -0.125]))?
            .compose(rotation([0., -45., 0.])?)?
            .compose(scaling([0.625, -0.625, 0.625]))?
            .compose(rotation([-100., 0., 0.])?)?
            .compose(rotation([0., -20., 0.])?)?;
    } else if properties.full_3d {
        if properties.rotate {
            transform = transform
                .compose(rotation([0., 0., 180.])?)?
                .compose(Affine::translation([0., -0.0625, 0.]))?;
        }
        transform = transform
            .compose(Affine::translation([0., 0.1875, 0.]))?
            .compose(Affine::translation([0.0625, -0.125, 0.]))?
            .compose(scaling([0.625, -0.625, 0.625]))?;
    } else {
        transform = transform
            .compose(Affine::translation([0.1875, 0.1875, 0.]))?
            .compose(scaling([0.875; 3]))?
            .compose(rotation([0., 0., -20.])?)?
            .compose(rotation([-60., 0., 0.])?)?
            .compose(rotation([0., 0., -30.])?)?;
    }
    transform = transform
        .compose(rotation([-15., 0., 0.])?)?
        .compose(rotation([0., 0., 40.])?)?;
    Ok(("thirdperson_righthand", transform))
}
