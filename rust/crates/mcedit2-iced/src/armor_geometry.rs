//! 原版护甲材料与实际纹饰；统一到身体模型、运行时图集和源装备载荷。
use crate::{
    entity_geometry::Rig,
    item_geometry::{ItemContext, Prepared},
    item_icons::Appearance,
    resource_models::items::Quad,
    textures::TextureBank,
};
use mcedit2_world::{
    armor_trim,
    entity_equipment::{Slot, Stack},
    entity_model::Catalog,
};
pub(crate) fn texture_dependency(id: &str) -> bool {
    id.split_once(':').is_some_and(|(_, path)| {
        path.starts_with("models/armor/") || path.starts_with("trims/models/armor/")
    })
}
struct Material {
    name: &'static str,
    dyed: bool,
}
fn material(stack: &Stack, slot: Slot, version: i32) -> Result<Option<Material>, String> {
    let registry = crate::item_catalog::get(Some(version)).ok_or("护甲没有准确物品目录")?;
    if !registry.items.contains_key(&stack.id) {
        return Err(format!("装备物品 {} 不在已核实的物品注册表中", stack.id));
    }
    let Some(name) = stack.id.strip_prefix("minecraft:") else {
        return Err(format!("装备 {} 的原生材料定义未知", stack.id));
    };
    let (prefix, part) = match name.rsplit_once('_') {
        Some(parts) => parts,
        None => return Ok(None),
    };
    let actual = match part {
        "helmet" => Slot::Head,
        "chestplate" => Slot::Chest,
        "leggings" => Slot::Legs,
        "boots" => Slot::Feet,
        _ => return Ok(None),
    };
    if actual != slot {
        return Ok(None);
    }
    let name = match prefix {
        "leather" => "leather",
        "chainmail" => "chainmail",
        "iron" => "iron",
        "golden" => "gold",
        "diamond" => "diamond",
        "netherite" if version != 1343 => "netherite",
        "turtle" if version != 1343 => "turtle",
        _ => return Ok(None),
    };
    Ok(Some(Material {
        name,
        dyed: name == "leather",
    }))
}
pub(crate) fn head_armor(stack: &Stack, version: i32) -> Result<bool, String> {
    Ok(material(stack, Slot::Head, version)?.is_some())
}
/// 各原生材料层共享同一套几何 / UV。cutout 按源顺序覆盖；decal 只作用于已写深度的护甲像素。
pub(crate) fn prepare(
    stack: &Stack,
    slot: Slot,
    version: i32,
    catalog: &Catalog,
    rig: &Rig,
    bank: &TextureBank,
    stop: &dyn Fn() -> bool,
    context: Option<ItemContext<'_>>,
    center: [f64; 3],
) -> Result<Option<Prepared>, String> {
    let Some(material) = material(stack, slot, version)? else {
        return Ok(None);
    };
    let leggings = slot == Slot::Legs;
    let mut trim_catalog = None;
    if version != 1343 {
        trim_catalog = Some(match context {
            Some(context) => context.environment.data.armor_trim_catalog(version)?,
            None => armor_trim::Catalog::builtin(version)?,
        });
    }
    let number = if leggings { 2 } else { 1 };
    let base = format!("minecraft:models/armor/{}_layer_{number}", material.name);
    let key = Appearance::new(&stack.id, Some(version), Some(&stack.value));
    let dyed = material.dyed
        && (version != 3955
            || trim_catalog
                .as_ref()
                .expect("现代纹饰目录已绑定")
                .dyeable(&stack.id));
    let tint = if dyed { key.tint(Some(0))? } else { [1.; 3] };
    let mut layers = vec![(base.clone(), tint, true, false)];
    if material.dyed {
        layers.push((format!("{base}_overlay"), [1.; 3], true, false));
    }
    if let Some(trim) = armor_trim::source(&stack.value, version)? {
        let style = trim_catalog.as_ref().ok_or("纹饰没有实际注册表")?.resolve(
            trim,
            &format!("minecraft:{}", material.name),
            leggings,
        )?;
        layers.push((style.sprite, [1.; 3], false, style.decal));
    }
    let parts: &[&str] = match slot {
        Slot::Head => &["/head", "/hat"],
        Slot::Chest => &["/body", "/left_arm", "/right_arm"],
        Slot::Legs => &["/body", "/left_leg", "/right_leg"],
        Slot::Feet => &["/left_leg", "/right_leg"],
        _ => return Err("非护甲槽不能建立护甲模型".into()),
    };
    let mut result = Prepared {
        center,
        faces: Vec::new(),
        textures: Default::default(),
        diagnostics: Vec::new(),
    };
    let faces = rig.armor_model(catalog, leggings, parts)?;
    let atlas = bank.atlas().ok_or("护甲准备缺少图集")?;
    // 原生按层提交整套模型；纹饰直接绑定自己的 sprite，保留独立动画和过滤。
    for (texture, tint, simple, decal) in layers {
        let mode = 1
            | 32
            | crate::material_mode::NO_MIPMAP
            | crate::material_mode::ORDERED_OPAQUE
            | if simple {
                crate::material_mode::SIMPLE_TEXTURE
            } else {
                0
            }
            | if decal {
                crate::material_mode::DEPTH_EQUAL
            } else {
                0
            }
            | if version != 1343 {
                crate::material_mode::VIEW_OFFSET_Z
            } else {
                0
            };
        if simple {
            bank.simple_sampler(&texture)?;
        }
        atlas
            .material(&texture, mode)
            .ok_or_else(|| format!("护甲实际材质缺失：{texture}"))?;
        for face in &faces {
            if stop() {
                return Err("护甲几何准备已取消".into());
            }
            result.faces.push(Quad {
                xyz: face.corners.map(|p| [p[0], p[1], p[2]]),
                uv: face.corners.map(|p| [p[3], p[4]]),
                texture: texture.clone(),
                tint: None,
                shade: true,
                colors: [[tint[0], tint[1], tint[2], 1.]; 4],
                winding: 1.,
                cull: false,
                mode: Some(mode),
            });
        }
    }
    Ok(Some(result))
}
