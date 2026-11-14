//! 原生眼睛层复用身体部件、姿态和资源定义；加色与光照合同按客户端版本选择。
use super::*;
pub(super) const TEXTURE: &str = "minecraft:entity/spider_eyes";
pub(super) const ENDERMAN_TEXTURE: &str = "minecraft:entity/enderman/enderman_eyes";
pub(super) const PHANTOM_TEXTURE: &str = "minecraft:entity/phantom_eyes";
pub(super) fn visible_when_invisible(id: &str) -> bool {
    monsters::is_spider(id) || matches!(id, "minecraft:enderman" | "minecraft:phantom")
}
pub(super) fn compile(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    layer: &str,
    texture: &str,
    poses: &BTreeMap<String, PartPose>,
    style: npc::RenderStyle,
) -> Result<Vec<WorldQuad>, String> {
    let quads = npc::render(
        catalog,
        bank,
        layer,
        texture,
        poses,
        npc::RenderStyle {
            tint: [1.; 4],
            ..style
        },
    )?;
    finish(catalog, visual, quads)
}
/// 独立全局矩阵的身体与眼睛使用同一变换，保留相同加色、光照和隐形合同。
pub(super) fn compile_transformed(
    catalog: &Catalog,
    bank: &TextureBank,
    visual: &EntityVisual,
    layer: &str,
    texture: &str,
    poses: &BTreeMap<String, PartPose>,
    transform: &serde_json::Value,
) -> Result<Vec<WorldQuad>, String> {
    let quads = npc::render_transformed(catalog, bank, layer, texture, poses, transform)?;
    finish(catalog, visual, quads)
}
fn finish(
    catalog: &Catalog,
    visual: &EntityVisual,
    mut quads: Vec<WorldQuad>,
) -> Result<Vec<WorldQuad>, String> {
    let legacy = catalog.version == "1.12.2";
    let invisible = byte(visual, "Invisible")?;
    for quad in &mut quads {
        quad.mode &= !(3
            | crate::material_mode::FORCE_OPAQUE_ALPHA
            | crate::material_mode::CUSTOM_ALPHA_CUTOFF
            | 32
            | (255 << 8));
        quad.mode |= 2
            | crate::material_mode::ADDITIVE
            | crate::material_mode::VISIBLE_WHEN_INVISIBLE
            | crate::material_mode::NO_MIPMAP;
        if legacy {
            quad.mode &= !crate::material_mode::SINGLE_SIDED;
            if !invisible {
                quad.mode |= crate::material_mode::BLEND_DEPTH_WRITE;
            }
            // 旧 cck 的 61680 坐标经 lightmap clamp 后为 block=15 / sky=0。
            quad.light = crate::lighting::fixed(crate::lighting::encode(
                mcedit2_world::saved_light::Levels {
                    block: Some(15),
                    sky: Some(0),
                },
            ));
        } else {
            quad.mode |= crate::material_mode::SINGLE_SIDED;
            quad.light = crate::lighting::UNLIT;
        }
    }
    Ok(quads)
}
