//! 鞘翅使用原生模型部件、保存的运动状态和真实玩家装饰图片。
use crate::{
    entity_geometry::Rig, item_geometry::Prepared, player_skin::Appearance,
    resource_models::items::Quad, textures::TextureBank,
};
use mcedit2_world::{entity_catalog::EntityVisual, entity_model::Catalog};
use std::sync::Arc;

pub(crate) fn texture_dependency(id: &str) -> bool {
    id == "minecraft:entity/elytra"
}
pub(crate) fn prepare(
    visual: &EntityVisual,
    catalog: &Catalog,
    rig: &Rig,
    bank: &TextureBank,
    skin: Option<&Appearance>,
    online_skins: bool,
    stop: &dyn Fn() -> bool,
    center: [f64; 3],
) -> Result<Prepared, String> {
    let decoration = if online_skins {
        match skin
            .map(|skin| skin.decorations().as_ref().map_err(Clone::clone))
            .transpose()?
        {
            None => None,
            Some(value) => match value.elytra.as_ref().map_err(Clone::clone)? {
                Some(texture) => Some(texture),
                None => value.cape.as_ref().map_err(Clone::clone)?.as_ref(),
            },
        }
    } else {
        None
    };
    let texture_id = match decoration {
        Some(texture) => texture.id.clone(),
        None => {
            let id = "minecraft:entity/elytra";
            bank.simple_sampler(id)?;
            bank.atlas()
                .ok_or("鞘翅缺少图集")?
                .simple(id)
                .ok_or("鞘翅实际整图纹理缺失")?;
            id.to_owned()
        }
    };
    let mut result = Prepared {
        center,
        faces: Vec::new(),
        textures: Default::default(),
        diagnostics: Vec::new(),
    };
    for face in rig.elytra_model(catalog, visual)? {
        if stop() {
            return Err("鞘翅准备已取消".into());
        }
        result.faces.push(Quad {
            xyz: face.corners.map(|p| [p[0], p[1], p[2]]),
            uv: face.corners.map(|p| [p[3], p[4]]),
            texture: texture_id.clone(),
            tint: None,
            shade: true,
            colors: [[1.; 4]; 4],
            winding: 1.,
            cull: false,
            mode: Some(
                1 | 32
                    | crate::material_mode::NO_MIPMAP
                    | crate::material_mode::SIMPLE_TEXTURE
                    | if catalog.version != "1.12.2" {
                        crate::material_mode::VIEW_OFFSET_Z
                    } else {
                        0
                    },
            ),
        });
    }
    if let Some(texture) = decoration {
        result
            .textures
            .insert(texture.id.clone(), Arc::new(texture.clone()));
    }
    Ok(result)
}
