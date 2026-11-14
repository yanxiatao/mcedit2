//! 已从 1.21.1 客户端核对的原版材质 / 邻面规则；不推断模组的同名方块。
use crate::sections::State;
#[path = "native_layers.rs"]
mod registry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Solid,
    Cutout,
    CutoutMipped,
    Translucent,
    Tripwire,
}

fn stained(name: &str, pane: bool) -> bool {
    if name
        == if pane {
            "stained_glass_pane"
        } else {
            "stained_glass"
        }
    {
        return true;
    }
    let suffix = if pane {
        "_stained_glass_pane"
    } else {
        "_stained_glass"
    };
    name.strip_suffix(suffix).is_some_and(|color| {
        matches!(
            color,
            "white"
                | "orange"
                | "magenta"
                | "light_blue"
                | "yellow"
                | "lime"
                | "pink"
                | "gray"
                | "light_gray"
                | "cyan"
                | "purple"
                | "blue"
                | "brown"
                | "green"
                | "red"
                | "black"
        )
    })
}
fn leaves(name: &str) -> bool {
    matches!(
        name,
        "oak_leaves"
            | "spruce_leaves"
            | "birch_leaves"
            | "jungle_leaves"
            | "acacia_leaves"
            | "dark_oak_leaves"
            | "mangrove_leaves"
            | "cherry_leaves"
            | "azalea_leaves"
            | "flowering_azalea_leaves"
            | "leaves"
            | "leaves2"
    )
}
fn half_transparent(name: &str) -> bool {
    matches!(name, "glass" | "tinted_glass" | "ice" | "frosted_ice") || stained(name, false)
}
/// 完整原版注册表采用客户端通道；叶子按精细显示，既有旧式别名保留明确规则。
pub fn layer(state: &State) -> Option<Layer> {
    let name = state.name.strip_prefix("minecraft:")?;
    match name {
        "packed_ice" | "blue_ice" => Some(Layer::Solid),
        "glass" => Some(Layer::Cutout),
        "glass_pane" => Some(Layer::CutoutMipped),
        "ice" | "frosted_ice" | "tinted_glass" => Some(Layer::Translucent),
        name if stained(name, false) || stained(name, true) => Some(Layer::Translucent),
        name if leaves(name) => Some(Layer::CutoutMipped),
        _ => registry::layer(name),
    }
}
/// 流体有独立通道；其无方块几何的 Block 默认 solid 不能套到水面上。
pub fn fluid_layer(state: &State) -> Option<Layer> {
    match state.name.as_str() {
        "minecraft:water"
        | "minecraft:flowing_water"
        | "minecraft:bubble_column"
        | "numeric:8"
        | "numeric:9" => Some(Layer::Translucent),
        "minecraft:lava" | "minecraft:flowing_lava" | "numeric:10" | "numeric:11" => {
            Some(Layer::Solid)
        }
        _ => None,
    }
}
/// HalfTransparentBlock 同一实际方块不绘制共面边界；不同染色方块不合并。
pub fn skip_same(source: &State, neighbor: &State) -> bool {
    if source.name != neighbor.name {
        return false;
    }
    let Some(name) = source.name.strip_prefix("minecraft:") else {
        return false;
    };
    if !half_transparent(name) {
        return false;
    }
    if name == "stained_glass" {
        return source
            .properties
            .iter()
            .find(|(key, _)| key == "color")
            .map(|(_, value)| value.as_str())
            == neighbor
                .properties
                .iter()
                .find(|(key, _)| key == "color")
                .map(|(_, value)| value.as_str());
    }
    true
}
/// 原版侧面水 overlay 依据 HalfTransparentBlock / LeavesBlock 类别。
pub fn water_overlay(neighbor: &State) -> bool {
    neighbor
        .name
        .strip_prefix("minecraft:")
        .is_some_and(|name| half_transparent(name) || leaves(name))
}
