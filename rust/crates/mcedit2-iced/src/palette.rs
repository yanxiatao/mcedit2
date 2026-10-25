//! Non-destructive, approximate Java block color guide for the 3D inspector.
//! This is a visualization, not a resource-pack atlas or a registry conversion.
//! Unknown/modded states retain a stable synthetic color; no NBT is changed.
use iced::Color;
use mcedit2_world::sections::State;
use mcedit2_world::surface::VoxelMaterial;

fn rgba(rgb: (u8, u8, u8), alpha: f32) -> Color {
    Color::from_rgba(
        rgb.0 as f32 / 255.0,
        rgb.1 as f32 / 255.0,
        rgb.2 as f32 / 255.0,
        alpha,
    )
}
fn dye(name: &str) -> Option<(u8, u8, u8)> {
    let name = name.split('_').next()?;
    Some(match name {
        "white" => (231, 232, 230),
        "orange" => (232, 129, 45),
        "magenta" => (179, 71, 174),
        "light" => return None,
        "yellow" => (245, 198, 59),
        "lime" => (126, 183, 41),
        "pink" => (236, 143, 174),
        "gray" => (64, 69, 72),
        "cyan" => (40, 127, 140),
        "purple" => (126, 63, 173),
        "blue" => (54, 74, 154),
        "brown" => (123, 81, 52),
        "green" => (85, 110, 42),
        "red" => (165, 49, 49),
        "black" => (33, 36, 40),
        _ => return None,
    })
}
fn dyed(name: &str) -> Option<(u8, u8, u8)> {
    for suffix in [
        "_wool",
        "_concrete",
        "_terracotta",
        "_stained_glass",
        "_carpet",
    ] {
        if let Some(prefix) = name.strip_suffix(suffix) {
            return if prefix == "light_blue" {
                Some((112, 156, 215))
            } else if prefix == "light_gray" {
                Some((156, 161, 161))
            } else {
                dye(prefix)
            };
        }
    }
    None
}
fn stable_hash(name: &str) -> (u8, u8, u8) {
    let mut h = 0x811c9dc5u32;
    for b in name.bytes() {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    (
        75 + (h >> 16 & 127) as u8,
        75 + (h >> 8 & 127) as u8,
        75 + (h & 127) as u8,
    )
}
fn legacy(id: u16) -> Option<(u8, u8, u8)> {
    Some(match id {
        1 => (123, 125, 123),
        2 => (106, 152, 70),
        3 => (129, 97, 63),
        4 => (112, 112, 112),
        5 => (161, 125, 81),
        7 => (74, 73, 74),
        12 => (220, 207, 149),
        13 => (127, 123, 119),
        14 => (148, 123, 80),
        15 => (141, 116, 92),
        16 => (103, 102, 99),
        17 => (116, 89, 57),
        18 => (74, 126, 55),
        20 => (167, 200, 215),
        24 => (216, 204, 156),
        35 => (214, 215, 211),
        41 => (245, 207, 59),
        42 => (197, 199, 204),
        49 => (38, 31, 52),
        56 => (99, 157, 158),
        79 => (164, 202, 226),
        87 => (124, 56, 46),
        89 => (230, 178, 86),
        98 => (122, 123, 122),
        _ => return None,
    })
}
/// A canonical family tint, or None for unknown blocks.
pub(crate) fn tint(state: &State, kind: VoxelMaterial, top: bool) -> Option<Color> {
    let name = state
        .name
        .split_once(':')
        .map(|(_, n)| n)
        .unwrap_or(state.name.as_str());
    if let Some(raw) = state.name.strip_prefix("numeric:") {
        let id = raw.parse::<u16>().ok()?;
        return legacy(id).map(|c| rgba(c, 1.0));
    }
    let alpha = match kind {
        VoxelMaterial::Solid => 1.0,
        VoxelMaterial::Transparent => 0.62,
        VoxelMaterial::Liquid => 0.76,
        VoxelMaterial::Foliage => 0.88,
    };
    if let Some(dye_color) = dyed(name) {
        return Some(rgba(dye_color, alpha));
    }
    let color = if name == "water" || name == "bubble_column" {
        (46, 104, 205)
    } else if name == "lava" {
        (246, 106, 28)
    } else if name == "grass_block" {
        if top {
            (102, 159, 65)
        } else {
            (124, 106, 69)
        }
    } else if name == "podzol" {
        (102, 78, 53)
    } else if name == "mycelium" {
        (134, 109, 132)
    } else if name == "dirt" || name == "coarse_dirt" || name == "rooted_dirt" {
        (131, 95, 62)
    } else if name.contains("deepslate") {
        (72, 73, 76)
    } else if name.contains("tuff") {
        (105, 112, 100)
    } else if name.contains("stone") || name == "andesite" || name == "gravel" {
        (125, 126, 125)
    } else if name.contains("diorite") {
        (198, 199, 194)
    } else if name.contains("granite") {
        (154, 109, 91)
    } else if name.contains("sand") || name == "end_stone" {
        (219, 207, 154)
    } else if name.contains("snow") || name == "quartz_block" {
        (231, 232, 229)
    } else if name.contains("ice") {
        (161, 199, 233)
    } else if name.contains("leaves") || name.contains("moss") {
        (80, 130, 60)
    } else if name.contains("planks") || name.contains("log") || name.contains("wood") {
        if name.contains("spruce") {
            (107, 77, 44)
        } else if name.contains("birch") {
            (204, 189, 145)
        } else if name.contains("dark_oak") {
            (72, 55, 36)
        } else if name.contains("cherry") {
            (217, 164, 166)
        } else if name.contains("mangrove") {
            (119, 52, 46)
        } else {
            (155, 119, 72)
        }
    } else if name.contains("copper") {
        (165, 104, 65)
    } else if name.contains("gold") {
        (239, 198, 54)
    } else if name.contains("iron") {
        (188, 177, 164)
    } else if name.contains("diamond") {
        (74, 193, 184)
    } else if name.contains("emerald") {
        (44, 170, 91)
    } else if name.contains("redstone") {
        (164, 43, 47)
    } else if name.contains("glass") {
        (172, 203, 220)
    } else if name.contains("obsidian") {
        (40, 32, 55)
    } else if name.contains("netherrack") {
        (104, 50, 50)
    } else if name.contains("basalt") {
        (75, 72, 71)
    } else {
        return None;
    };
    Some(rgba(color, alpha))
}
pub(crate) fn unknown_tint(state: &State, kind: VoxelMaterial) -> Color {
    let alpha = match kind {
        VoxelMaterial::Solid => 1.0,
        VoxelMaterial::Liquid => 0.76,
        VoxelMaterial::Transparent => 0.62,
        VoxelMaterial::Foliage => 0.88,
    };
    rgba(stable_hash(&state.name), alpha)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state(s: &str) -> State {
        State {
            name: s.into(),
            properties: vec![],
        }
    }
    #[test]
    fn common_family_colors_keep_original_named_identity() {
        let water = tint(&state("minecraft:water"), VoxelMaterial::Liquid, true).unwrap();
        let lava = tint(&state("minecraft:lava"), VoxelMaterial::Liquid, true).unwrap();
        assert_ne!(water, lava);
        assert!(water.a < 1.0);
        let grass = tint(&state("minecraft:grass_block"), VoxelMaterial::Solid, true).unwrap();
        let side = tint(&state("minecraft:grass_block"), VoxelMaterial::Solid, false).unwrap();
        assert_ne!(grass, side);
    }
    #[test]
    fn numeric_and_unknown_states_are_stable_not_converted() {
        assert!(tint(&state("numeric:301"), VoxelMaterial::Solid, true).is_none());
        assert!(tint(&state("numeric:2"), VoxelMaterial::Solid, true).is_some());
        let custom = state("mod:alien_block");
        assert_eq!(
            unknown_tint(&custom, VoxelMaterial::Solid),
            unknown_tint(&custom, VoxelMaterial::Solid)
        );
    }
    #[test]
    fn dye_prefixes_and_alpha_are_respected() {
        assert_ne!(
            tint(
                &state("minecraft:light_blue_wool"),
                VoxelMaterial::Solid,
                true
            ),
            tint(&state("minecraft:blue_wool"), VoxelMaterial::Solid, true)
        );
        assert!(
            tint(
                &state("minecraft:blue_stained_glass"),
                VoxelMaterial::Transparent,
                true
            )
            .unwrap()
            .a < 1.0
        );
    }
}
