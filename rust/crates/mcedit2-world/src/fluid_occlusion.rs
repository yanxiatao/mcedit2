//! 已按 1.21.1 客户端注册表核实的流体遮挡形状，不以模组 ID 后缀推断运行时类别。
use crate::{sections::State, surface_visibility::Direction, WorldError};

/// 完整边界证明或八分格形状；位序为 x + 2*z + 4*y。
/// 边界证明只表达整面覆盖，不能据此推断未覆盖的半格形状。
#[derive(Debug, Clone, Copy)]
pub enum Occlusion {
    Boundary(u8),
    Octants(u8),
}
impl Occlusion {
    /// 高度已由 FluidSurface 验证。水平液面从格底延伸到所给高度。
    pub fn covers(self, face: Direction, height: f32) -> bool {
        match self {
            Self::Boundary(mask) => mask & face.flag() != 0,
            Self::Octants(mask) => {
                let mut required = match face {
                    Direction::Up => 0xf0,
                    Direction::Down => 0x0f,
                    Direction::East => 0xaa,
                    Direction::West => 0x55,
                    Direction::South => 0xcc,
                    Direction::North => 0x33,
                };
                if !matches!(face, Direction::Up | Direction::Down) && height <= 0.5 {
                    required &= 0x0f;
                }
                mask & required == required
            }
        }
    }
}
fn property<'a>(state: &'a State, key: &str, default: &'a str) -> &'a str {
    // 省略属性采用该原版类的默认状态；显式非法值由对应分支拒绝。
    state
        .properties
        .iter()
        .find_map(|(k, v)| (k == key).then_some(v.as_str()))
        .unwrap_or(default)
}
fn invalid(state: &State, property: &str) -> WorldError {
    WorldError::Invalid(format!("原版流体遮挡形状属性 {property} 无效：{state:?}"))
}

/// 仅处理静态注册表中已验证的 SlabBlock / StairBlock 及铜块子类。
/// 其他类别返回 None，由调用方提供自己的完整边界证明，不能猜测半格占用。
pub fn vanilla(state: &State) -> Result<Option<Occlusion>, WorldError> {
    let Some(name) = state.name.strip_prefix("minecraft:") else {
        return Ok(None);
    };
    let slab = matches!(
        name,
        "acacia_slab"
            | "andesite_slab"
            | "bamboo_mosaic_slab"
            | "bamboo_slab"
            | "birch_slab"
            | "blackstone_slab"
            | "brick_slab"
            | "cherry_slab"
            | "cobbled_deepslate_slab"
            | "cobblestone_slab"
            | "crimson_slab"
            | "cut_copper_slab"
            | "cut_red_sandstone_slab"
            | "cut_sandstone_slab"
            | "dark_oak_slab"
            | "dark_prismarine_slab"
            | "deepslate_brick_slab"
            | "deepslate_tile_slab"
            | "diorite_slab"
            | "end_stone_brick_slab"
            | "exposed_cut_copper_slab"
            | "granite_slab"
            | "jungle_slab"
            | "mangrove_slab"
            | "mossy_cobblestone_slab"
            | "mossy_stone_brick_slab"
            | "mud_brick_slab"
            | "nether_brick_slab"
            | "oak_slab"
            | "oxidized_cut_copper_slab"
            | "petrified_oak_slab"
            | "polished_andesite_slab"
            | "polished_blackstone_brick_slab"
            | "polished_blackstone_slab"
            | "polished_deepslate_slab"
            | "polished_diorite_slab"
            | "polished_granite_slab"
            | "polished_tuff_slab"
            | "prismarine_brick_slab"
            | "prismarine_slab"
            | "purpur_slab"
            | "quartz_slab"
            | "red_nether_brick_slab"
            | "red_sandstone_slab"
            | "sandstone_slab"
            | "smooth_quartz_slab"
            | "smooth_red_sandstone_slab"
            | "smooth_sandstone_slab"
            | "smooth_stone_slab"
            | "spruce_slab"
            | "stone_brick_slab"
            | "stone_slab"
            | "tuff_brick_slab"
            | "tuff_slab"
            | "warped_slab"
            | "waxed_cut_copper_slab"
            | "waxed_exposed_cut_copper_slab"
            | "waxed_oxidized_cut_copper_slab"
            | "waxed_weathered_cut_copper_slab"
            | "weathered_cut_copper_slab"
    );
    if slab {
        return match property(state, "type", "bottom") {
            "bottom" => Ok(Some(Occlusion::Octants(0x0f))),
            "top" => Ok(Some(Occlusion::Octants(0xf0))),
            "double" => Ok(Some(Occlusion::Octants(0xff))),
            _ => Err(invalid(state, "type")),
        };
    }
    let stairs = matches!(
        name,
        "acacia_stairs"
            | "andesite_stairs"
            | "bamboo_mosaic_stairs"
            | "bamboo_stairs"
            | "birch_stairs"
            | "blackstone_stairs"
            | "brick_stairs"
            | "cherry_stairs"
            | "cobbled_deepslate_stairs"
            | "cobblestone_stairs"
            | "crimson_stairs"
            | "cut_copper_stairs"
            | "dark_oak_stairs"
            | "dark_prismarine_stairs"
            | "deepslate_brick_stairs"
            | "deepslate_tile_stairs"
            | "diorite_stairs"
            | "end_stone_brick_stairs"
            | "exposed_cut_copper_stairs"
            | "granite_stairs"
            | "jungle_stairs"
            | "mangrove_stairs"
            | "mossy_cobblestone_stairs"
            | "mossy_stone_brick_stairs"
            | "mud_brick_stairs"
            | "nether_brick_stairs"
            | "oak_stairs"
            | "oxidized_cut_copper_stairs"
            | "polished_andesite_stairs"
            | "polished_blackstone_brick_stairs"
            | "polished_blackstone_stairs"
            | "polished_deepslate_stairs"
            | "polished_diorite_stairs"
            | "polished_granite_stairs"
            | "polished_tuff_stairs"
            | "prismarine_brick_stairs"
            | "prismarine_stairs"
            | "purpur_stairs"
            | "quartz_stairs"
            | "red_nether_brick_stairs"
            | "red_sandstone_stairs"
            | "sandstone_stairs"
            | "smooth_quartz_stairs"
            | "smooth_red_sandstone_stairs"
            | "smooth_sandstone_stairs"
            | "spruce_stairs"
            | "stone_brick_stairs"
            | "stone_stairs"
            | "tuff_brick_stairs"
            | "tuff_stairs"
            | "warped_stairs"
            | "waxed_cut_copper_stairs"
            | "waxed_exposed_cut_copper_stairs"
            | "waxed_oxidized_cut_copper_stairs"
            | "waxed_weathered_cut_copper_stairs"
            | "weathered_cut_copper_stairs"
    );
    if !stairs {
        return Ok(None);
    }
    // Direction.get2DDataValue 与 StairBlock.SHAPE_BY_STATE 的真实顺序。
    let facing = match property(state, "facing", "north") {
        "south" => 0,
        "west" => 1,
        "north" => 2,
        "east" => 3,
        _ => return Err(invalid(state, "facing")),
    };
    let shape = match property(state, "shape", "straight") {
        "straight" => 0,
        "inner_left" => 1,
        "inner_right" => 2,
        "outer_left" => 3,
        "outer_right" => 4,
        _ => return Err(invalid(state, "shape")),
    };
    const QUARTERS: [u8; 20] = [
        12, 5, 3, 10, 14, 13, 7, 11, 13, 7, 11, 14, 8, 4, 1, 2, 4, 1, 2, 8,
    ];
    let quarters = QUARTERS[shape * 4 + facing];
    let octants = match property(state, "half", "bottom") {
        "bottom" => 0x0f | quarters << 4,
        "top" => 0xf0 | quarters,
        _ => return Err(invalid(state, "half")),
    };
    Ok(Some(Occlusion::Octants(octants)))
}
