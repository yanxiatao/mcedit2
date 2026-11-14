//! 世界物品的保存载荷、静态 ground 显示及共享四角材质；不推进客户端动画或物理。
use crate::{
    item_icons::{field, Appearance},
    resource_models::items::Quad,
    textures::{RuntimeTexture, TextureAtlas, TextureBank},
    world_mesh::WorldQuad,
};
use mcedit2_world::{entity_catalog::EntityVisual, nbt_document::Value};
use std::{collections::BTreeMap, sync::Arc};
#[path = "display_entities.rs"]
pub(crate) mod displays;
#[path = "projectile_items.rs"]
mod projectiles;
pub(crate) struct Prepared {
    pub center: [f64; 3],
    pub(crate) faces: Vec<Quad>,
    pub textures: BTreeMap<String, Arc<RuntimeTexture>>,
    pub diagnostics: Vec<String>,
}
#[derive(Clone, Copy)]
pub(crate) struct ItemContext<'a> {
    pub environment: &'a crate::item_world::Environment,
    pub offset: [f64; 3],
}
pub(crate) fn stack(visual: &EntityVisual) -> Result<Option<(String, i32, Value)>, String> {
    let Some(value) = visual.source_field_optional("Item")? else {
        return Ok(None);
    };
    let Value::Compound(fields) = &value else {
        return Err("实体 Item 必须为 Compound".into());
    };
    if fields.is_empty() {
        return Ok(None);
    }
    let id = match field(Some(&value), "id") {
        Some(Value::Short(358) | Value::Int(358)) if visual.source_version == Some(1343) => {
            "minecraft:filled_map".into()
        }
        Some(Value::String(id)) => {
            if id.contains(':') {
                id.clone()
            } else {
                format!("minecraft:{id}")
            }
        }
        _ => return Err("实体物品 id 缺失或不是已支持的注册字符串".into()),
    };
    let count = match field(
        Some(&value),
        if visual.source_version == Some(3955) {
            "count"
        } else {
            "Count"
        },
    ) {
        Some(Value::Int(n)) if visual.source_version == Some(3955) => *n,
        Some(Value::Byte(n)) if visual.source_version != Some(3955) => i32::from(*n),
        None if id == "minecraft:air" => 0,
        _ => return Err("实体物品数量编码与源 DataVersion 不符".into()),
    };
    Ok((id != "minecraft:air" && count > 0).then_some((id, count, value)))
}
impl Prepared {
    pub fn quads(&self, atlas: &TextureAtlas, textures: bool) -> Result<Vec<WorldQuad>, String> {
        let mut faces = Vec::new();
        for q in &self.faces {
            let material = if textures {
                atlas
                    .material(&q.texture, q.mode.unwrap_or(0))
                    .ok_or_else(|| format!("世界物品图集缺少 {}", q.texture))?
            } else {
                &atlas.white
            };
            let out = WorldQuad {
                xyz: q.xyz,
                uv: if textures { q.uv } else { [[0.5; 2]; 4] },
                rgba: if textures {
                    q.colors
                } else {
                    q.colors
                        .map(|rgba| crate::material_mode::vertex_color(q.mode.unwrap_or(0), rgba))
                },
                mode: if textures {
                    q.mode.unwrap_or(2 | 32)
                } else {
                    crate::material_mode::untextured(q.mode.unwrap_or(0))
                } | if q.cull {
                    crate::material_mode::SINGLE_SIDED
                } else {
                    0
                },
                atlas_page: material.page,
                light: crate::lighting::FULLBRIGHT,
            };
            let sampler = if textures {
                atlas.native_sampler(material, out.mode)?
            } else {
                None
            };
            crate::static_triangle_uv::bind_quad(
                out,
                material,
                sampler,
                Some(atlas.resource_budget()),
                |face| faces.push(face),
            )?;
        }
        Ok(faces)
    }
}

pub(crate) use mcedit2_world::entity_catalog::is_dropped;
/// 保存姿态下的物品堆；数量级、ground 变换及平面层距沿用准确客户端。
/// 随机相位没有保存到 NBT，编辑器冻结在源 Rotation；散布按物品载荷稳定取样。
pub(crate) fn dropped(
    visual: &EntityVisual,
    position: [f64; 3],
    bank: &TextureBank,
    online_skins: bool,
    cancelled: &dyn Fn() -> bool,
    context: Option<ItemContext<'_>>,
) -> Result<Prepared, String> {
    use sha2::{Digest, Sha256};
    let version = visual.source_version.ok_or("掉落物缺少源 DataVersion")?;
    if !matches!(version, 1343 | 3465 | 3955) {
        return Err(format!("未核实 DataVersion={version} 的掉落物格式"));
    }
    if bank.item_data_version() != Some(version) {
        return Err(format!(
            "掉落物版本 {version} 与资源 {:?} 不同",
            bank.item_data_version()
        ));
    }
    let mut prepared = Prepared {
        center: [position[0] - 0.5, position[1], position[2] - 0.5],
        faces: Vec::new(),
        textures: BTreeMap::new(),
        diagnostics: Vec::new(),
    };
    let Some((id, count, value)) = stack(visual)? else {
        return Ok(prepared);
    };
    let key = Appearance::new(&id, Some(version), Some(&value)).with_online_skins(online_skins);
    let mut properties = key.properties()?;
    let yaw = visual.rotation.ok_or("掉落物缺少实际 Rotation")?[0];
    let world_position = std::array::from_fn(|i| position[i] + context.map_or(0., |c| c.offset[i]));
    crate::item_world::apply_dropped(
        &id,
        &value,
        yaw,
        version,
        context.map(|c| c.environment),
        world_position,
        &mut properties,
    )?;
    let mut scene = bank.item_scene_context(&key.model_id()?, &properties, "ground")?;
    if scene.builtin {
        for texture in crate::item_specials::prepare_materials(bank, &key, &mut scene, cancelled)? {
            prepared
                .textures
                .insert(texture.id.clone(), Arc::new(texture));
        }
    }
    crate::item_icons::generated_context(bank, &mut scene, true, cancelled)?;
    let copies = match count {
        1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    };
    let (s, c) = yaw.to_radians().sin_cos();
    let mut hash = Sha256::new();
    hash.update(id.as_bytes());
    hash.update(key.damage.to_le_bytes());
    let hash: [u8; 32] = hash.finalize().into();
    let mut random = crate::java_random::LegacyRandom::new(i64::from_le_bytes(
        hash[..8].try_into().expect("SHA-256 宽度固定"),
    ));
    let scale = scene.display_scale;
    let height = 0.1 + 0.25 * scale[1];
    let z_start = if scene.gui_3d {
        0.
    } else {
        -0.09375 * (copies - 1) as f32 * 0.5 * scale[2]
    };
    for copy in 0..copies {
        if cancelled() {
            return Err("掉落物外观准备已取消".into());
        }
        let offset = if copy == 0 {
            [0.; 3]
        } else if scene.gui_3d {
            std::array::from_fn(|_| (random.float() * 2. - 1.) * 0.15)
        } else {
            [
                (random.float() * 2. - 1.) * 0.15 * 0.5,
                (random.float() * 2. - 1.) * 0.15 * 0.5,
                0.,
            ]
        };
        for face in &scene.quads {
            if cancelled() {
                return Err("掉落物模型编译已取消".into());
            }
            let mut face = face.clone();
            face.xyz = face.xyz.map(|p| {
                let mut p = scene.transform.point(p);
                for i in 0..3 {
                    p[i] += offset[i];
                }
                if !scene.gui_3d {
                    p[2] += z_start + copy as f32 * 0.09375 * scale[2];
                }
                [c * p[0] + s * p[2], p[1] + height, -s * p[0] + c * p[2]]
            });
            let tint = key.tint(face.tint)?;
            for color in &mut face.colors {
                for i in 0..3 {
                    color[i] *= tint[i];
                }
            }
            if face.winding * scene.transform.determinant() < 0. {
                face.xyz.swap(1, 3);
                face.uv.swap(1, 3);
                face.colors.swap(1, 3);
            }
            prepared.faces.push(face);
        }
    }
    Ok(prepared)
}

/// 世界与结构预览共用的物品实体准备入口；显式资源实体定义由调用方优先处理。
pub(crate) fn prepare(
    visual: &EntityVisual,
    entity_id: &str,
    position: [f64; 3],
    bank: &TextureBank,
    map_root: Option<&std::path::Path>,
    map_version: Option<i32>,
    online_skins: bool,
    cancelled: &dyn Fn() -> bool,
    context: Option<ItemContext<'_>>,
    language: crate::language::Language,
    maps: Option<&mcedit2_world::map_edit::View>,
) -> Result<Prepared, String> {
    if is_dropped(entity_id) {
        dropped(visual, position, bank, online_skins, cancelled, context)
    } else if displays::supported(entity_id) {
        displays::prepare(
            visual,
            entity_id,
            position,
            bank,
            online_skins,
            cancelled,
            context,
            language,
        )
    } else if mcedit2_world::entity_projectiles::Projectile::read(entity_id).is_some() {
        projectiles::prepare(
            visual,
            entity_id,
            position,
            bank,
            online_skins,
            cancelled,
            context,
        )
    } else if mcedit2_world::hanging::is_frame(entity_id) {
        crate::frame_geometry::prepare(
            visual,
            entity_id == "minecraft:glow_item_frame",
            bank,
            map_root,
            map_version,
            online_skins,
            cancelled,
            context,
            maps,
        )
    } else {
        Err(format!("没有物品实体渲染入口：{entity_id}"))
    }
}
pub(crate) fn supported(entity_id: &str) -> bool {
    is_dropped(entity_id)
        || mcedit2_world::hanging::is_frame(entity_id)
        || mcedit2_world::entity_projectiles::Projectile::read(entity_id).is_some()
        || displays::supported(entity_id)
}
