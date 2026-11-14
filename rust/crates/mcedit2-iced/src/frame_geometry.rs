//! 展示框本体与 fixed 物品模型；保留实际模型面、NBT、纹理和源实体归属。
use crate::{
    item_icons::Appearance,
    resource_models::items::{Quad, Scene},
    textures::TextureBank,
};
use mcedit2_world::{entity_catalog::EntityVisual, hanging};
use std::{collections::BTreeMap, path::Path, sync::Arc};

pub(crate) use crate::item_geometry::{ItemContext, Prepared};
fn rotate(facing: u8, p: [f32; 3]) -> [f32; 3] {
    let columns = [[1, 0, 0], [0, 1, 0], [0, 0, 1]].map(|p| hanging::frame_basis(facing, p));
    std::array::from_fn(|i| (0..3).map(|j| columns[j][i] as f32 * p[j]).sum())
}
fn transformed(
    scene: Scene,
    facing: u8,
    rotation: u8,
    scale: f32,
    depth: f32,
    key: Option<&Appearance>,
) -> Result<Vec<Quad>, String> {
    let (s, c) = (f32::from(rotation) * 45.).to_radians().sin_cos();
    scene
        .quads
        .into_iter()
        .map(|mut q| {
            q.xyz = q.xyz.map(|p| {
                let [x, y, z] = scene.transform.point(p).map(|v| v * scale);
                rotate(facing, [c * x - s * y, s * x + c * y, z + depth])
            });
            if let Some(key) = key {
                let tint = key.tint(q.tint)?;
                for color in &mut q.colors {
                    for i in 0..3 {
                        color[i] *= tint[i];
                    }
                }
            }
            // WorldQuad 使用朝外的叉积；元素面原有负 winding，镜像再反转一次。
            if q.winding * scene.transform.determinant() < 0. {
                q.xyz.swap(1, 3);
                q.uv.swap(1, 3);
                q.colors.swap(1, 3);
            }
            Ok(q)
        })
        .collect()
}
pub(crate) fn prepare(
    visual: &EntityVisual,
    glow: bool,
    bank: &TextureBank,
    map_root: Option<&Path>,
    map_version: Option<i32>,
    online_skins: bool,
    cancelled: &dyn Fn() -> bool,
    item_context: Option<ItemContext<'_>>,
    maps: Option<&mcedit2_world::map_edit::View>,
) -> Result<Prepared, String> {
    let source = hanging::Frame::read(visual)?;
    let payload = crate::item_geometry::stack(visual)?.map(|(id, _, value)| (id, value));
    let map = if payload.is_some() {
        crate::map_frame::parse(visual)?
    } else {
        None
    };
    let tile = map.as_ref().map_or(source.tile, |map| map.tile);
    let mut faces = if source.invisible {
        Vec::new()
    } else {
        let mut scene = bank.frame_scene(glow, map.is_some())?;
        // 元素已以方块中心为原点，对应原版 translate(-.5,-.5,-.5)。
        scene.transform = crate::root_transform::Affine::IDENTITY;
        // 现代 ItemFrameRenderer 显式使用 Sheets.solidBlockSheet；旧版保留 Alpha Test。
        // 显式扩展模型材质仍按其声明，原版元素不能误入普通物品透明通道。
        for face in &mut scene.quads {
            face.mode
                .get_or_insert(if visual.source_version == Some(1343) {
                    1 | 32
                } else {
                    crate::material_mode::FORCE_OPAQUE_ALPHA
                });
        }
        transformed(scene, source.facing, 0, 1., 0., None)?
    };
    let mut textures = BTreeMap::new();
    let mut diagnostics = Vec::new();
    if let Some(map) = map {
        let root = map_root.ok_or("地图展示框缺少来源世界目录")?;
        let image = match maps {
            Some(maps) => maps.read(root, map.id, map_version)?,
            None => mcedit2_world::map_data::read(root, map.id, map_version)?,
        };
        diagnostics.extend(image.warnings.iter().cloned());
        let mut markers = image.decorations.clone();
        let (_, value) = payload.as_ref().ok_or("地图展示框缺少实际物品")?;
        mcedit2_world::map_decorations::from_item(
            &mut markers,
            value,
            visual.source_version.ok_or("地图展示框缺少源版本")?,
            image.center,
            image.scale,
            image.unlimited_tracking,
        )?;
        let prepared = crate::map_geometry::prepare(&image, &markers, true, bank, cancelled)?;
        let depth = if map.invisible { 0.5 } else { 0.4375 };
        let angle = (map.rotation % 4) as f32 * 90. + 180.;
        let (s, c) = angle.to_radians().sin_cos();
        faces.extend(prepared.faces.into_iter().map(|mut face| {
            face.xyz = face.xyz.map(|[x, y, z]| {
                let (x, y) = (x / 128. - 0.5, y / 128. - 0.5);
                rotate(
                    map.facing,
                    [c * x - s * y, s * x + c * y, depth - (1. - z) / 128.],
                )
            });
            face
        }));
        textures.extend(prepared.textures);
    } else if let Some((id, value)) = payload {
        let version = visual.source_version;
        if bank.item_data_version() != version {
            return Err(format!(
                "资源客户端 {:?} 与展示框物品 {:?} 的版本不同",
                bank.item_data_version(),
                version
            ));
        }
        let key = Appearance::new(&id, version, Some(&value)).with_online_skins(online_skins);
        let mut properties = key.properties()?;
        let position = source.center();
        let position =
            std::array::from_fn(|i| position[i] + item_context.map_or(0., |c| c.offset[i]));
        crate::item_world::apply(
            &id,
            &value,
            &source,
            version.ok_or("展示框缺少源版本")?,
            item_context.map(|c| c.environment),
            position,
            &mut properties,
        )?;
        let mut scene = bank.item_scene_context(&key.model_id()?, &properties, "fixed")?;
        if scene.builtin {
            for texture in
                crate::item_specials::prepare_materials(bank, &key, &mut scene, cancelled)?
            {
                textures.insert(texture.id.clone(), Arc::new(texture));
            }
        }
        crate::item_icons::generated_context(bank, &mut scene, true, cancelled)?;
        faces.extend(transformed(
            scene,
            source.facing,
            source.rotation,
            0.5,
            if source.invisible { 0.5 } else { 0.4375 },
            Some(&key),
        )?);
    }
    if cancelled() {
        return Err("展示框外观准备已取消".into());
    }
    // Tile 的方块格心；渲染坐标 X/Z 以格心为整数，Y 保持原世界坐标。
    Ok(Prepared {
        center: [
            f64::from(tile[0]),
            f64::from(tile[1]) + 0.5,
            f64::from(tile[2]),
        ],
        faces,
        textures,
        diagnostics,
    })
}
