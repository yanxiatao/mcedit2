//! Static source-material flags for the fullbright spectator GPU pipeline.
//! The low two bits retain the existing opaque/cutout/blend convention;
//! extensions do not affect atlas page selection or original world edits.
const KIND_MASK: u32 = 0b11;
pub(crate) const SINGLE_SIDED: u32 = 1 << 2;
pub(crate) const FORCE_OPAQUE_ALPHA: u32 = 1 << 3;
pub(crate) const CUSTOM_ALPHA_CUTOFF: u32 = 1 << 4;
const CUTOFF_SHIFT: u32 = 8;

#[cfg(test)]
pub(crate) fn kind(value: u32) -> u32 {
    value & KIND_MASK
}

/// An explicit glTF alpha mode takes priority over a PNG's inferred alpha
/// class. Vanilla and ordinary OBJ source materials remain unchanged.
pub(crate) fn encode(
    inferred: u32, gltf_mode: Option<&str>, gltf_cutoff: f32, double_sided: bool,
) -> u32 {
    let Some(mode) = gltf_mode else { return inferred & KIND_MASK; };
    let face = if double_sided { 0 } else { SINGLE_SIDED };
    match mode {
        "OPAQUE" => face | FORCE_OPAQUE_ALPHA,
        "MASK" => {
            let cutoff = if gltf_cutoff.is_finite() {
                gltf_cutoff.clamp(0.0, 1.0)
            } else {
                0.5
            };
            1 | face | CUSTOM_ALPHA_CUTOFF
                | (((cutoff * 255.0).round() as u32) << CUTOFF_SHIFT)
        }
        "BLEND" => 2 | face,
        _ => inferred & KIND_MASK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_modes_override_inferred_texture_transparency() {
        let solid = encode(2, Some("OPAQUE"), 0.5, false);
        assert_eq!(kind(solid), 0);
        assert_ne!(solid & FORCE_OPAQUE_ALPHA, 0);
        assert_ne!(solid & SINGLE_SIDED, 0);
        assert_eq!(kind(encode(0, Some("BLEND"), 0.5, true)), 2);
        assert_eq!(encode(2, None, 0.1, false), 2);
    }
    #[test]
    fn gltf_mask_cutoff_and_two_sided_flags_are_preserved() {
        let mask = encode(0, Some("MASK"), 0.75, false);
        assert_eq!(kind(mask), 1);
        assert_eq!(mask & SINGLE_SIDED, SINGLE_SIDED);
        assert_eq!(mask & CUSTOM_ALPHA_CUTOFF, CUSTOM_ALPHA_CUTOFF);
        assert_eq!((mask >> CUTOFF_SHIFT) & 255, 191);
        assert_eq!((encode(0, Some("MASK"), 0.0, true) >> CUTOFF_SHIFT) & 255, 0);
    }
}
