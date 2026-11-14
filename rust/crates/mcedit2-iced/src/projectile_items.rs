//! 投掷物复用完整物品资源与染色，冻结为保存朝向下的一份模型，不模拟飞行或粒子。
use super::*;
use crate::root_transform::Affine;
use mcedit2_world::{entity_projectiles::Projectile, inventory};

pub(super) fn prepare(
    visual: &EntityVisual,
    entity_id: &str,
    position: [f64; 3],
    bank: &TextureBank,
    online_skins: bool,
    cancelled: &dyn Fn() -> bool,
    context: Option<ItemContext<'_>>,
) -> Result<Prepared, String> {
    if cancelled() {
        return Err("投掷物外观准备已取消".into());
    }
    let projectile =
        Projectile::read(entity_id).ok_or_else(|| format!("没有绑定物品投掷物 {entity_id}"))?;
    let version = visual.source_version.ok_or("投掷物缺少源 DataVersion")?;
    projectile.check_version(entity_id, version)?;
    if bank.item_data_version() != Some(version) {
        return Err("投掷物资源与源版本不同".into());
    }
    let payload = projectile
        .payload_field(version)
        .map(|field| visual.source_field_optional(field))
        .transpose()?
        .flatten();
    let payload = match payload {
        Some(value) => {
            match inventory::read_stack(&value, Some(version)).map_err(|error| error.to_string())? {
                Some(entry) => {
                    let id = if entry.id.contains(':') {
                        entry.id
                    } else {
                        format!("minecraft:{}", entry.id)
                    };
                    Some((id, value))
                }
                None => None,
            }
        }
        None => None,
    };
    let mut prepared = Prepared {
        center: [position[0] - 0.5, position[1], position[2] - 0.5],
        faces: Vec::new(),
        textures: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    let id = payload
        .as_ref()
        .map(|(id, _)| id.as_str())
        .or(projectile.default_item(version));
    let Some(id) = id else {
        return Ok(prepared);
    };
    let value = payload.as_ref().map(|(_, value)| value);
    let key = Appearance::new(id, Some(version), value).with_online_skins(online_skins);
    let mut properties = key.properties()?;
    let [yaw, pitch] = visual.rotation.ok_or("投掷物缺少实际 Rotation")?;
    if let Some(value) = value {
        crate::item_world::apply_dropped(
            id,
            value,
            yaw,
            version,
            context.map(|c| c.environment),
            std::array::from_fn(|i| position[i] + context.map_or(0., |c| c.offset[i])),
            &mut properties,
        )?;
    }
    let mut scene = bank.item_scene_context(&key.model_id()?, &properties, "ground")?;
    if scene.builtin {
        for texture in crate::item_specials::prepare_materials(bank, &key, &mut scene, cancelled)? {
            prepared
                .textures
                .insert(texture.id.clone(), Arc::new(texture));
        }
    }
    crate::item_icons::generated_context(bank, &mut scene, true, cancelled)?;
    let scale = match projectile {
        Projectile::Fireball => 3.,
        Projectile::SmallFireball => 0.75,
        _ => 1.,
    };
    // 静态展示使用保存 Rotation，不再为朝向相机的逐帧转动重建物品场景。
    let transform = Affine::parse(&serde_json::json!({"origin":"corner", "rotation":{"y":-yaw}}))?
        .compose(Affine::parse(
            &serde_json::json!({"origin":"corner", "rotation":{"x":pitch}}),
        )?)?
        .compose(Affine::parse(
            &serde_json::json!({"origin":"corner", "scale":scale}),
        )?)?
        .compose(scene.transform)?;
    for mut face in scene.quads {
        if cancelled() {
            return Err("投掷物外观准备已取消".into());
        }
        face.xyz = face.xyz.map(|point| transform.point(point));
        let tint = key.tint(face.tint)?;
        for color in &mut face.colors {
            for i in 0..3 {
                color[i] *= tint[i];
            }
        }
        if face.winding * transform.determinant() < 0. {
            face.xyz.swap(1, 3);
            face.uv.swap(1, 3);
            face.colors.swap(1, 3);
        }
        prepared.faces.push(face);
    }
    Ok(prepared)
}
