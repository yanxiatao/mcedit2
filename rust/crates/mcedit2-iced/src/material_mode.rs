//! Static source-material flags for the fullbright spectator GPU pipeline.
//! The low two bits retain the existing opaque/cutout/blend convention;
//! extensions do not affect atlas page selection or original world edits.
const KIND_MASK: u32 = 0b11;
/// 彩色发光告示牌描边在距源格心 16 格内显示，黑色正文由模型层保持常显。
pub(crate) const SIGN_NEAR_OUTLINE: u32 = 1 << 29;
/// 文字的已染色素材与源锚点；同一告示牌按源顺序绘制，不按字形中心拆散描边与正文。
pub(crate) const FONT_TEXT: u32 = 1 << 30;
/// 描边颜色已合成进纹理，此材质的顶点 RGB 因而存放真实源锚点，Alpha 仍是覆盖率。
/// 复用现有顶点通道，任意字体尺寸均不需要从文字边界猜测源位置或增加所有地形面的体积。
pub(crate) fn vertex_color(mode: u32, rgba: [f32; 4]) -> [f32; 4] {
    if rgba[3] == crate::guardian_eye::COLOR_TAG {
        [1.; 4]
    } else if mode & (SIGN_NEAR_OUTLINE | FONT_TEXT) != 0 {
        [1., 1., 1., rgba[3]]
    } else {
        rgba
    }
}
/// (-1,0) 的顶点 Alpha 保留“先按原图 .1 裁切、再乘正透明度”的原生片元语义。
/// 普通材质 Alpha 为 0..1；-1 由眼睛几何独占，不与此标识混用。
pub(crate) fn pre_tint_alpha(alpha: f32) -> bool {
    alpha > -1. && alpha < 0.
}
/// 顶点颜色已通过 vertex_color；返回最终覆盖 Alpha，加色仍保留原纹理 Alpha。
pub(crate) fn shade_texel(mode: u32, texel: [f32; 4], mut color: [f32; 4]) -> Option<[f32; 4]> {
    let pre_tint = pre_tint_alpha(color[3]);
    if pre_tint {
        if texel[3] < 0.1 {
            return None;
        }
        color[3] = -color[3];
    }
    let mut rgba = std::array::from_fn(|i| texel[i] * color[i]);
    if pre_tint {
        return Some(rgba);
    }
    if mode & ADDITIVE != 0 {
        if mode & (32 | CUSTOM_ALPHA_CUTOFF) != 0 {
            covered_alpha_f32(mode, rgba[3])?;
        }
    } else {
        rgba[3] = covered_alpha_f32(mode, rgba[3])?;
    }
    Some(rgba)
}
/// 自由视角拾取保持原有 double 重心插值，不先将顶点颜色量化为 float。
pub(crate) fn shade_texel_f64(mode: u32, texel: [f64; 4], mut color: [f64; 4]) -> Option<[f64; 4]> {
    let pre_tint = color[3] > -1. && color[3] < 0.;
    if pre_tint {
        if texel[3] < 0.1 {
            return None;
        }
        color[3] = -color[3];
    }
    let mut rgba = std::array::from_fn(|i| texel[i] * color[i]);
    if pre_tint {
        return Some(rgba);
    }
    if mode & ADDITIVE != 0 {
        if mode & (32 | CUSTOM_ALPHA_CUTOFF) != 0 {
            covered_alpha(mode, rgba[3])?;
        }
    } else {
        rgba[3] = covered_alpha(mode, rgba[3])?;
    }
    Some(rgba)
}
pub(crate) fn visible_from(mode: u32, anchor: [f64; 3], eye: [f64; 3]) -> bool {
    let distance = if mode & SIGN_NEAR_OUTLINE != 0 {
        256.
    } else if mode & NAME_LABEL != 0 {
        4096.
    } else {
        return true;
    };
    anchor
        .into_iter()
        .zip(eye)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        < distance
}
/// 朝向相机的矩形源面；xyz 第四槽保存真实旋转锚点，第四角由前三角无损恢复。
/// 复用现有面宽度，普通地形的布局和 GPU 缓存步幅不改变。
pub(crate) const BILLBOARD: u32 = 1 << 18;
/// 朝向相机但保持世界 Y 轴竖直；旋转面不使用轴向剔除，复用该字段未占用的值 7。
pub(crate) const YAW_BILLBOARD: u32 = BILLBOARD | AXIS_FACING_MASK;
pub(crate) fn is_yaw_billboard(mode: u32) -> bool {
    mode & YAW_BILLBOARD == YAW_BILLBOARD
}
/// 名称标签的可见距离按其面片锚点计算；不把字形颜色误作世界坐标。
pub(crate) const NAME_LABEL: u32 = 1 << 19;
/// 源 PNG 的整图采样；只在材质绑定时选择独立槽，不接受图集动画裁切或推进。
pub(crate) const SIMPLE_TEXTURE: u32 = 1 << 20;
/// 原生 decal 仅覆盖已有相等深度；不创造新的几何覆盖或拾取表面。
pub(crate) const DEPTH_EQUAL: u32 = 1 << 21;
/// 原生 ONE / ONE 加色；RGB 不预乘纹理 Alpha，也不按 Alpha 丢弃片段。
pub(crate) const ADDITIVE: u32 = 1 << 22;
/// 原生层明确在身体隐形时仍提交，例如蜘蛛眼睛；不使普通身体可见。
pub(crate) const VISIBLE_WHEN_INVISIBLE: u32 = 1 << 23;
/// 隐去材质时只保留几何语义；面片的锚点编码不能随纹理一同清除。
pub(crate) fn untextured(mode: u32) -> u32 {
    mode & (BILLBOARD
        | NAME_LABEL
        | VIEW_OFFSET_Z
        | SINGLE_SIDED
        | VISIBLE_WHEN_INVISIBLE
        | AXIS_FACING_MASK
        | ORDERED_OPAQUE)
}
pub(crate) const SINGLE_SIDED: u32 = 1 << 2;
pub(crate) const FORCE_OPAQUE_ALPHA: u32 = 1 << 3;
pub(crate) const CUSTOM_ALPHA_CUTOFF: u32 = 1 << 4;
/// 原版 VIEW_OFFSET_Z_LAYERING 在投影前缩放完整视图空间，不改变源模型尺寸。
pub(crate) const VIEW_OFFSET_Z: u32 = 1 << 6;
pub(crate) const VIEW_OFFSET_Z_SCALE: f32 = 0.999755859375;

pub(crate) fn view_scale(mode: u32) -> f64 {
    if mode & VIEW_OFFSET_Z != 0 {
        f64::from(VIEW_OFFSET_Z_SCALE)
    } else {
        1.0
    }
}

/// 相机 / 正交视图原点仍使用 f64 世界坐标，保留大坐标下源几何的精度。
pub(crate) fn view_point(point: [f64; 3], origin: [f64; 3], mode: u32) -> [f64; 3] {
    let scale = view_scale(mode);
    if scale == 1.0 {
        point
    } else {
        std::array::from_fn(|i| origin[i] + (point[i] - origin[i]) * scale)
    }
}
/// 原版 cutout 使用无 mip 的方块纹理状态；cutout_mipped 和 tripwire 仍使用 mip。
pub(crate) const NO_MIPMAP: u32 = 1 << 7;
/// 原版文字 / 地图与绊线共用 Alpha 混合、写深度的既有提交阶段。
pub(crate) const BLEND_DEPTH_WRITE: u32 = 1 << 28;
pub(crate) const TRIPWIRE: u32 = BLEND_DEPTH_WRITE;
/// 共面实体层在不透明深度通道中按源顺序绘制，不参与材质页或高度重排。
pub(crate) const ORDERED_OPAQUE: u32 = 1 << 24;
/// 构建时核实的轴向几何朝向：0=其他，1..6=±X/±Y/±Z；不参与材质采样。
pub(crate) const AXIS_FACING_SHIFT: u32 = 25;
pub(crate) const AXIS_FACING_MASK: u32 = 7 << AXIS_FACING_SHIFT;
const CUTOFF_SHIFT: u32 = 8;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Pass {
    #[default]
    Opaque,
    Tripwire,
    Translucent,
}
pub(crate) fn pass(mode: u32) -> Pass {
    if mode & KIND_MASK != 2 {
        Pass::Opaque
    } else if mode & TRIPWIRE != 0 {
        Pass::Tripwire
    } else {
        Pass::Translucent
    }
}

/// 将纹理与插值顶点 Alpha 的乘积按生产着色器的规则转换为覆盖率。
/// 拾取和 Canvas 同时调用此逻辑，保留自定义 MASK、tripwire 和 solid 语义。
pub(crate) fn covered_alpha(mode: u32, alpha: f64) -> Option<f64> {
    covered_alpha_precision(mode, alpha, false)
}

/// 像素着色与 WGSL 同为 f32，保留 cutoff / 最小 Alpha 在该精度下的边界。
pub(crate) fn covered_alpha_f32(mode: u32, alpha: f32) -> Option<f32> {
    covered_alpha_precision(mode, alpha as f64, true).map(|v| v as f32)
}
/// 加色层以实际 RGB 贡献参与拾取，黑色无贡献区域不创造命中面。
pub(crate) fn covered_color(mode: u32, rgba: [f64; 4]) -> bool {
    if mode & ADDITIVE != 0 {
        rgba[..3].iter().any(|value| *value > 0.)
            && (mode & (32 | CUSTOM_ALPHA_CUTOFF) == 0 || covered_alpha(mode, rgba[3]).is_some())
    } else {
        covered_alpha(mode, rgba[3]).is_some()
    }
}
pub(crate) fn covered_color_f32(mode: u32, rgba: [f32; 4]) -> bool {
    if mode & ADDITIVE != 0 {
        rgba[..3].iter().any(|value| *value > 0.)
            && (mode & (32 | CUSTOM_ALPHA_CUTOFF) == 0
                || covered_alpha_f32(mode, rgba[3]).is_some())
    } else {
        covered_alpha_f32(mode, rgba[3]).is_some()
    }
}
fn covered_alpha_precision(mode: u32, alpha: f64, float32: bool) -> Option<f64> {
    let number = |value: f64| if float32 { value as f32 as f64 } else { value };
    if !alpha.is_finite() {
        return None;
    }
    let threshold = if mode & 32 != 0 {
        number(0.1)
    } else if mode & CUSTOM_ALPHA_CUTOFF != 0 {
        number(((mode >> CUTOFF_SHIFT) & 255) as f64 / 255.0)
    } else {
        0.5
    };
    if mode & KIND_MASK == 1 {
        return (alpha >= threshold).then_some(1.0);
    }
    if mode & FORCE_OPAQUE_ALPHA != 0 {
        return Some(1.0);
    }
    if mode & KIND_MASK == 2 && mode & (32 | CUSTOM_ALPHA_CUTOFF) != 0 && alpha < threshold {
        return None;
    }
    (alpha > number(0.0039)).then_some(alpha.clamp(0.0, 1.0))
}

#[cfg(test)]
pub(crate) fn kind(value: u32) -> u32 {
    value & KIND_MASK
}

/// An explicit glTF alpha mode takes priority over a PNG's inferred alpha
/// class. Vanilla and ordinary OBJ source materials remain unchanged.
pub(crate) fn encode(
    inferred: u32,
    gltf_mode: Option<&str>,
    gltf_cutoff: f32,
    double_sided: bool,
) -> u32 {
    let Some(mode) = gltf_mode else {
        return inferred;
    };
    let face = if double_sided { 0 } else { SINGLE_SIDED };
    match mode {
        "OPAQUE" => face | FORCE_OPAQUE_ALPHA,
        "MASK" => {
            let cutoff = if gltf_cutoff.is_finite() {
                gltf_cutoff.clamp(0.0, 1.0)
            } else {
                0.5
            };
            if cutoff == 0.1 {
                1 | face | 32
            } else {
                1 | face | CUSTOM_ALPHA_CUTOFF | (((cutoff * 255.0).round() as u32) << CUTOFF_SHIFT)
            }
        }
        "BLEND" => 2 | face,
        _ => inferred & KIND_MASK,
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared_coverage_matches_shader_thresholds_and_authored_alpha() {
        let solid = encode(2, Some("OPAQUE"), 0.5, false);
        assert_eq!(covered_alpha(solid, 0.0), Some(1.0));
        assert_eq!(covered_alpha(1, 0.49999), None);
        assert_eq!(covered_alpha(1, 0.5), Some(1.0));
        assert_eq!(covered_alpha(1 | 32, 0.09999), None);
        assert_eq!(covered_alpha(1 | 32, 0.1), Some(1.0));
        assert_eq!(covered_alpha(2 | 32, 0.1), Some(0.1));
        let mask = encode(2, Some("MASK"), 0.75, true);
        assert_eq!(covered_alpha(mask, 190.9 / 255.0), None);
        assert_eq!(covered_alpha(mask, 191.0 / 255.0), Some(1.0));
        assert_eq!(covered_alpha(2, 0.0039), None);
        assert_eq!(covered_alpha(2, 1.0 / 255.0), Some(1.0 / 255.0));
        assert_eq!(covered_alpha(2, f64::NAN), None);
    }

    fn explicit_modes_override_inferred_texture_transparency() {
        let solid = encode(2, Some("OPAQUE"), 0.5, false);
        assert_eq!(kind(solid), 0);
        assert_ne!(solid & FORCE_OPAQUE_ALPHA, 0);
        assert_ne!(solid & SINGLE_SIDED, 0);
        assert_eq!(kind(encode(0, Some("BLEND"), 0.5, true)), 2);
        assert_eq!(encode(2, None, 0.1, false), 2);
    }

    fn gltf_mask_cutoff_and_two_sided_flags_are_preserved() {
        let mask = encode(0, Some("MASK"), 0.75, false);
        assert_eq!(kind(mask), 1);
        assert_eq!(mask & SINGLE_SIDED, SINGLE_SIDED);
        assert_eq!(mask & CUSTOM_ALPHA_CUTOFF, CUSTOM_ALPHA_CUTOFF);
        assert_eq!((mask >> CUTOFF_SHIFT) & 255, 191);
        assert_eq!(
            (encode(0, Some("MASK"), 0.0, true) >> CUTOFF_SHIFT) & 255,
            0
        );
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Materials {
            shared_coverage_matches_shader_thresholds_and_authored_alpha();
            explicit_modes_override_inferred_texture_transparency();
            gltf_mask_cutoff_and_two_sided_flags_are_preserved();
        }
    }
}
