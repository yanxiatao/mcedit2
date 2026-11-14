//! 实际四类鱼模型、离水矩阵与热带鱼双层染色，复用世界 / 结构的同代上下文。
use super::*;
use mcedit2_world::fish_variant::{Shape, Tropical};
use serde_json::{json, Value as Json};

const SMALL_PATTERNS: [&str; 6] = [
    "minecraft:entity/fish/tropical_a_pattern_1",
    "minecraft:entity/fish/tropical_a_pattern_2",
    "minecraft:entity/fish/tropical_a_pattern_3",
    "minecraft:entity/fish/tropical_a_pattern_4",
    "minecraft:entity/fish/tropical_a_pattern_5",
    "minecraft:entity/fish/tropical_a_pattern_6",
];
const LARGE_PATTERNS: [&str; 6] = [
    "minecraft:entity/fish/tropical_b_pattern_1",
    "minecraft:entity/fish/tropical_b_pattern_2",
    "minecraft:entity/fish/tropical_b_pattern_3",
    "minecraft:entity/fish/tropical_b_pattern_4",
    "minecraft:entity/fish/tropical_b_pattern_5",
    "minecraft:entity/fish/tropical_b_pattern_6",
];

fn rotation(
    catalog: &Catalog,
    layer: &str,
    poses: &mut BTreeMap<String, PartPose>,
    path: &str,
    axis: usize,
    value: f32,
) -> Result<(), String> {
    let mut pose = catalog.resolved_pose(layer, path, poses)?;
    pose.rotation.as_mut().ok_or("鱼类实际部件缺少 rotation")?[axis] = value;
    poses.insert(path.into(), pose);
    Ok(())
}

fn transform(catalog: &Catalog, visual: &EntityVisual, id: &str) -> Result<Json, String> {
    let [yaw, _] = visual.rotation.ok_or("鱼类缺少实际 Rotation")?;
    let yaw = npc::frozen_yaw(catalog, visual, yaw)?;
    let scale = if catalog.version == "1.21.1" {
        npc::scale(visual)?
    } else {
        1.
    };
    let mut steps = vec![json!({"origin":"corner", "scale":scale})];
    if id == "minecraft:pufferfish" {
        // PufferfishRenderer 在父旋转前应用 cos(0) * .08，不使用离水翻转。
        steps.push(json!({"origin":"corner", "translation":[0.,0.08,0.]}));
    }
    steps.push(json!({"origin":"corner", "rotation":{"y":180.-yaw}}));
    // 三类鱼的附加 Y 摆动在零时刻为零；鲑鱼的 -0.4 平移在水中同样存在。
    if id == "minecraft:salmon" {
        steps.push(json!({"origin":"corner", "translation":[0.,0.,-0.4]}));
    }
    if mcedit2_world::entity_water::needs_contact(id) && !mcedit2_world::entity_water::read(visual)?
    {
        let translation = if id == "minecraft:cod" {
            [0.1, 0.1, -0.1]
        } else {
            [0.2, 0.1, 0.]
        };
        steps.push(json!({"origin":"corner", "translation":translation}));
        steps.push(json!({"origin":"corner", "rotation":{"z":90.}}));
    }
    steps.push(json!({"origin":"corner", "scale":[-1.,-1.,1.]}));
    steps.push(json!({"origin":"corner", "translation":[0.,-1.501,0.]}));
    Ok(Json::Array(steps))
}

fn tint(color: u8) -> Result<[f32; 4], String> {
    // 两版 DyeColor 的实际 texture diffuse 三通道与共享染料表完全相同。
    let [r, g, b] = crate::banner_geometry::color(color)?;
    Ok([r as f32 / 255., g as f32 / 255., b as f32 / 255., 1.])
}

fn render(
    catalog: &Catalog,
    bank: &TextureBank,
    layer: &str,
    texture: &str,
    poses: &BTreeMap<String, PartPose>,
    transform: &Json,
    tint: [f32; 4],
    ordered: bool,
) -> Result<Vec<WorldQuad>, String> {
    npc::render_transformed_material(
        catalog,
        bank,
        layer,
        texture,
        poses,
        transform,
        &json!({"alpha":"MASK", "alpha_test":true, "cutoff":0.1, "double_sided":true, "mipmap":false, "tint":tint, "ordered":ordered}),
    )
}

pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    id: &str,
    visual: &EntityVisual,
) -> Result<Vec<WorldQuad>, String> {
    if !matches!(catalog.version.as_str(), "1.20.1" | "1.21.1") {
        return Err(format!("{} 没有实际鱼类模型注册 {id}", catalog.version));
    }
    let transform = transform(catalog, visual, id)?;
    let mut poses = BTreeMap::new();
    let (layer, texture) = match id {
        "minecraft:cod" => {
            rotation(
                catalog,
                "minecraft:cod#main",
                &mut poses,
                "/tail_fin",
                1,
                0.,
            )?;
            ("minecraft:cod#main", "minecraft:entity/fish/cod")
        }
        "minecraft:salmon" => {
            rotation(
                catalog,
                "minecraft:salmon#main",
                &mut poses,
                "/body_back",
                1,
                0.,
            )?;
            ("minecraft:salmon#main", "minecraft:entity/fish/salmon")
        }
        "minecraft:pufferfish" => {
            let (layer, left, right) = match mcedit2_world::entity_physics::puff_state(visual)? {
                0 => ("minecraft:pufferfish_small#main", "/left_fin", "/right_fin"),
                1 => (
                    "minecraft:pufferfish_medium#main",
                    "/left_blue_fin",
                    "/right_blue_fin",
                ),
                2 => (
                    "minecraft:pufferfish_big#main",
                    "/left_blue_fin",
                    "/right_blue_fin",
                ),
                _ => unreachable!("实际膨胀状态已解析"),
            };
            rotation(catalog, layer, &mut poses, left, 2, 0.2)?;
            rotation(catalog, layer, &mut poses, right, 2, -0.2)?;
            (layer, "minecraft:entity/fish/pufferfish")
        }
        "minecraft:tropical_fish" => {
            let variant = Tropical::read(visual)?;
            let (layer, pattern_layer, base, patterns) = match variant.shape {
                Shape::Small => (
                    "minecraft:tropical_fish_small#main",
                    "minecraft:tropical_fish_small#pattern",
                    "minecraft:entity/fish/tropical_a",
                    &SMALL_PATTERNS,
                ),
                Shape::Large => (
                    "minecraft:tropical_fish_large#main",
                    "minecraft:tropical_fish_large#pattern",
                    "minecraft:entity/fish/tropical_b",
                    &LARGE_PATTERNS,
                ),
            };
            rotation(catalog, layer, &mut poses, "/tail", 1, 0.)?;
            let mut out = render(
                catalog,
                bank,
                layer,
                base,
                &poses,
                &transform,
                tint(variant.base_color)?,
                false,
            )?;
            if !byte(visual, "Invisible")? {
                let mut pattern_poses = BTreeMap::new();
                rotation(catalog, pattern_layer, &mut pattern_poses, "/tail", 1, 0.)?;
                out.extend(render(
                    catalog,
                    bank,
                    pattern_layer,
                    patterns[variant.pattern as usize],
                    &pattern_poses,
                    &transform,
                    tint(variant.pattern_color)?,
                    true,
                )?);
            }
            return Ok(out);
        }
        _ => return Err(format!("没有绑定鱼类模型 {id}")),
    };
    // 实际模型只响应鳍 / 尾的相位；不以 Rotation.pitch、Age 或 Motion 推算游动。
    render(
        catalog, bank, layer, texture, &poses, &transform, [1.; 4], false,
    )
}
