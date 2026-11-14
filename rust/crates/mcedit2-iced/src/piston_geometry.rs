//! 保存进度的移动方块资源模型；几何、材质与普通地形共用同一后端。
use crate::{textures::TextureBank, world_mesh::WorldQuad};
use mcedit2_world::piston::Source;

pub(crate) fn quads(
    source: &Source,
    bank: &TextureBank,
    position: [i32; 3],
    biomes: &mcedit2_world::biome_source::Region,
    textures: bool,
    stop: &dyn Fn() -> bool,
) -> Result<Vec<WorldQuad>, String> {
    if !bank.vanilla_piston_supported(&source.state) {
        return Err("移动活塞没有原版源模型绑定".into());
    }
    if bank.item_data_version() != Some(source.version) {
        return Err(format!(
            "移动活塞源版本 {} 与资源 {:?} 不同",
            source.version,
            bank.item_data_version()
        ));
    }
    let moved = bank.render_state(&source.moved);
    let parts = source.render_parts(moved)?;
    let mut out = Vec::new();
    for part in parts {
        if stop() {
            return Err("移动活塞模型编译已取消".into());
        }
        let sample =
            std::array::from_fn::<_, 3, _>(|axis| position[axis] + part.sample_offset[axis]);
        for quad in crate::spectator::resource_block_quads(
            bank,
            &part.state,
            (sample[0], sample[1], sample[2]),
            biomes,
            textures,
            stop,
        )? {
            out.push(quad.translated(part.translation));
        }
    }
    Ok(out)
}
