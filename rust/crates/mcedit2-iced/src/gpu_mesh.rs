//! One projected quad per original surface face; UVs point into the safe
//! prepacked resource atlas. Pixel sampling is performed on the GPU, not by
//! subdividing faces on the CPU. Original source/pick ordering is unchanged.
use crate::isometric::IsometricSurface;
use mcedit2_world::sections::State;
/// Read-only model identity borrowed directly from a loaded source voxel.
/// The weighted model variant belongs in the key: identical blockstate
/// properties can legitimately resolve to different original JSON geometry.
/// Entries never outlive the corresponding immutable surface snapshot.
pub(crate) type ModelCacheKey<'a> = (&'a str, &'a [(String, String)], u64);
/// 面样式按完整方块状态、面方向、模型元素与加权变体缓存。
type FaceStyleKey = (String, Vec<(String, String)>, u8, usize, u64);
pub(crate) type FaceStyle = ([[f32; 2]; 4], AlphaMode, u32);
pub(crate) fn model_cache_key(state: &State, variant: u64) -> ModelCacheKey<'_> {
    (state.name.as_str(), state.properties.as_slice(), variant)
}

use crate::textures::{AlphaMode, AtlasUv};
use iced::{Color, Point, Size};
use std::collections::HashMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Quality {
    Fast,
    #[default]
    Balanced,
    Detailed,
}
impl Quality {
    pub fn next(self) -> Self {
        match self {
            Self::Fast => Self::Balanced,
            Self::Balanced => Self::Detailed,
            Self::Detailed => Self::Fast,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Balanced => "Balanced",
            Self::Detailed => "Detailed",
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Vertex {
    pub xy: [f32; 2],
    /// Orthographic camera-space depth, normalized in the GPU primitive.
    pub depth: f32,
    pub rgba: [f32; 4],
    pub uv: [f32; 2],
    pub mode: u32,
    /// Relative original-world vertex position; spectator mode only.
    pub xyz: [f32; 3],
    /// 通用投影顶点的坐标通道；自由视角生产网格使用 WorldQuad 实例。
    pub world_mode: u32,
    /// Array layer in the multi-page resource-pack block atlas.
    pub atlas_page: u32,
    /// 显式满亮，或携带通道存在位的原始光照编码。
    pub light: u32,
}
impl Vertex {
    fn new(p: Point, depth: f32, c: Color, uv: [f32; 2], mode: AlphaMode) -> Self {
        Self {
            xy: [p.x, p.y],
            depth,
            rgba: [c.r, c.g, c.b, c.a],
            uv,
            mode: mode.gpu_bits(),
            xyz: [0.0; 3],
            world_mode: 0,
            atlas_page: 0,
            light: crate::lighting::FULLBRIGHT,
        }
    }
}
fn quad(
    out: &mut Vec<Vertex>,
    q: [Point; 4],
    depths: [f32; 4],
    c: Color,
    uv: [[f32; 2]; 4],
    mode: AlphaMode,
    page: u32,
) {
    for i in [0usize, 1, 2, 0, 2, 3] {
        let mut vertex = Vertex::new(q[i], depths[i], c, uv[i], mode);
        vertex.atlas_page = page;
        out.push(vertex);
    }
}
fn quad_uv(region: &AtlasUv) -> [[f32; 2]; 4] {
    [
        region.sample(0.0, 0.0),
        region.sample(1.0, 0.0),
        region.sample(1.0, 1.0),
        region.sample(0.0, 1.0),
    ]
}
/// One GPU quad per projected original face; no scene-wide polygon cap.
///
/// The mesh worker can abandon obsolete camera views between individual
/// source faces. Never return an incomplete mesh for caching or uploading.
pub(crate) fn build_cancellable(
    view: &IsometricSurface,
    size: Size,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<Vertex>> {
    if cancelled() {
        return None;
    }
    let faces = view.faces_cached_cancellable(size, cancelled)?;
    if cancelled() {
        return None;
    }
    let mut out = Vec::with_capacity(faces.len().saturating_mul(6));
    let atlas = view.texture_bank.as_ref().and_then(|bank| bank.atlas());
    let mut style_cache: HashMap<FaceStyleKey, Option<FaceStyle>> = HashMap::new();
    let mut fluid_materials = crate::fluid_material::Cache::new(view.texture_bank.as_deref());
    let mut colors = view
        .texture_bank
        .as_ref()
        .map(|bank| bank.tint_context(&view.surface.biomes));
    for face in faces.iter() {
        if cancelled() {
            return None;
        }
        let voxel = &view.surface.voxels[face.voxel_index];
        if let Some(style) = face.static_style_in(view, colors.as_mut()) {
            // 投影渲染器按六顶点源面划分材质页与上传段；后半退化，
            // 不得把两个不同页的 OBJ 三角形合成一个材质批次。
            for i in [0, 1, 2, 0, 2, 2] {
                out.push(Vertex {
                    xy: [face.corners[i].x, face.corners[i].y],
                    depth: face.vertex_depths[i],
                    rgba: style.rgba[i],
                    uv: style.uv[i],
                    mode: style.mode,
                    xyz: [0.0; 3],
                    world_mode: 0,
                    atlas_page: style.page,
                    light: crate::lighting::FULLBRIGHT,
                });
            }
            continue;
        }
        let light = crate::lighting::FULLBRIGHT;
        let state = face.state(voxel);
        let display_state = view
            .texture_bank
            .as_ref()
            .map_or(state, |b| b.render_state(state));
        let display_kind = if face.fluid_layer {
            mcedit2_world::surface::VoxelMaterial::Liquid
        } else {
            view.texture_bank
                .as_ref()
                .map_or(voxel.kind, |b| b.render_material(state, voxel.kind))
        };
        let variant = if voxel.span > 1 {
            0
        } else {
            view.texture_bank
                .as_ref()
                .map_or(0, |bank| bank.model_variant_key(state, face.pos))
        };
        let fluid_style = if view.show_textures && face.fluid_layer {
            fluid_materials.style_in(voxel, state, face.face, colors.as_mut())
        } else {
            None
        };
        let style = if view.show_textures {
            if voxel.span == 1 && display_kind == mcedit2_world::surface::VoxelMaterial::Liquid {
                fluid_style.map(|style| style.face)
            } else {
                let cache_key = (
                    state.name.clone(),
                    state.properties.clone(),
                    face.face as u8,
                    face.element_index,
                    variant,
                );
                *style_cache.entry(cache_key).or_insert_with(|| {
                    view.texture_bank.as_ref().and_then(|bank| {
                        bank.atlas_face_style_at(state, face.face, face.element_index, face.pos)
                    })
                })
            }
        } else {
            None
        };
        let texture = style.as_ref().map(|(uv, _, _)| *uv);
        let color = if texture.is_some() {
            // 固定全亮，只乘材质 / 生物群系着色；透明度来自原始 PNG。
            let (_, alpha) = crate::textures::classify_block_alpha(
                display_state,
                display_kind,
                style
                    .as_ref()
                    .map(|(_, a, _)| *a)
                    .unwrap_or(AlphaMode::Opaque),
            );
            let tint = if let Some(style) = fluid_style {
                style.tint
            } else {
                colors
                    .as_mut()
                    .map(|colors| {
                        colors.face(
                            state,
                            face.face,
                            face.element_index,
                            face.pos,
                            voxel.biome.as_deref(),
                        )
                    })
                    .unwrap_or([1.0; 3])
            };
            Color::from_rgba(tint[0], tint[1], tint[2], alpha)
        } else {
            face.color
        };
        let coords = texture
            .or_else(|| atlas.map(|a| quad_uv(&a.white)))
            .unwrap_or([[0.5, 0.5]; 4]);
        let mut mode = style
            .as_ref()
            .map(|(_, a, _)| *a)
            .unwrap_or(AlphaMode::Opaque);
        let page = style
            .as_ref()
            .map(|(_, _, page)| *page)
            .or_else(|| atlas.map(|a| a.white.page))
            .unwrap_or(0);
        mode = crate::textures::classify_block_alpha(display_state, display_kind, mode).0;
        if color.a < 0.999 {
            mode = AlphaMode::Blend;
        }
        if view.show_textures {
            if let Some((raw_uv, region, Some(sampler))) =
                view.texture_bank.as_ref().and_then(|bank| {
                    bank.atlas_repeating_face_at(state, face.face, face.element_index, face.pos)
                })
            {
                let source_mode = style.as_ref().map_or(0, |(_, alpha, _)| alpha.gpu_bits());
                let source = crate::world_mesh::WorldQuad {
                    xyz: std::array::from_fn(|i| {
                        [face.corners[i].x, face.corners[i].y, face.vertex_depths[i]]
                    }),
                    uv: raw_uv.map(|uv| uv.map(|value| value / 16.)),
                    rgba: [[color.r, color.g, color.b, color.a]; 4],
                    mode: mode.gpu_bits()
                        | (source_mode
                            & (crate::material_mode::SIMPLE_TEXTURE
                                | crate::material_mode::NO_MIPMAP)),
                    atlas_page: region.page,
                    light,
                };
                if let Err(error) = crate::static_triangle_uv::bind_quad(
                    source,
                    &region,
                    Some(sampler),
                    view.texture_bank
                        .as_deref()
                        .and_then(|bank| bank.geometry_budget()),
                    |piece| {
                        for i in [0, 1, 2, 0, 2, 3] {
                            out.push(Vertex {
                                xy: [piece.xyz[i][0], piece.xyz[i][1]],
                                depth: piece.xyz[i][2],
                                rgba: piece.rgba[i],
                                uv: piece.uv[i],
                                mode: piece.mode,
                                xyz: [0.; 3],
                                world_mode: 0,
                                atlas_page: piece.atlas_page,
                                light: piece.light,
                            });
                        }
                    },
                ) {
                    log::error!("[models] {} 的投影材质寻址失败：{error}", state.name);
                    return None;
                }
                continue;
            }
        }
        let start = out.len();
        quad(
            &mut out,
            face.corners,
            face.vertex_depths,
            color,
            coords,
            mode,
            page,
        );
        for vertex in &mut out[start..] {
            vertex.light = light;
        }
    }
    (!cancelled()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_face_quad_and_texture_uv_keep_alpha() {
        let mut vertices = Vec::new();
        let points = [
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 1.0),
            Point::new(0.0, 1.0),
        ];
        let rect = AtlasUv {
            start: [0.25, 0.5],
            end: [0.5, 0.75],
            page: 3,
        };
        quad(
            &mut vertices,
            points,
            [1.0, 2.0, 3.0, 4.0],
            Color::from_rgba(0.2, 0.3, 0.4, 0.5),
            quad_uv(&rect),
            AlphaMode::Blend,
            rect.page,
        );
        assert_eq!(vertices.len(), 6);
        assert_eq!(vertices[0].uv, [0.25, 0.5]);
        assert_eq!(vertices[2].uv, [0.5, 0.75]);
        assert_eq!(vertices[5].xy, [0.0, 1.0]);
        assert_eq!(vertices[5].rgba[3], 0.5);
        assert_eq!(vertices[0].depth, 1.0);
        assert_eq!(vertices[0].atlas_page, 3);
        assert_eq!(vertices[2].depth, 3.0);
        assert_eq!(vertices[0].mode, AlphaMode::Blend.gpu_bits());
        assert_eq!(std::mem::size_of::<Vertex>(), 64);
        assert_eq!(vertices[0].light, crate::lighting::FULLBRIGHT);
        assert_eq!(vertices[0].world_mode, 0);
    }

    #[test]
    fn borrowed_model_cache_keys_share_state_content_but_not_weighted_variants() {
        use std::collections::HashMap;
        let source = State {
            name: "minecraft:oak_stairs".into(),
            properties: vec![("facing".into(), "north".into())],
        };
        let equivalent = source.clone();
        let other_orientation = State {
            name: source.name.clone(),
            properties: vec![("facing".into(), "south".into())],
        };
        let key = model_cache_key(&source, 2);
        // A borrowed key must reference the source snapshot, not an allocated copy.
        assert!(std::ptr::eq(key.0.as_ptr(), source.name.as_ptr()));
        assert!(std::ptr::eq(key.1.as_ptr(), source.properties.as_ptr()));
        let mut cache: HashMap<ModelCacheKey<'_>, u8> = HashMap::new();
        cache.insert(key, 7);
        assert_eq!(cache.get(&model_cache_key(&equivalent, 2)), Some(&7));
        assert_eq!(cache.get(&model_cache_key(&source, 3)), None);
        assert_eq!(cache.get(&model_cache_key(&other_orientation, 2)), None);
        assert_eq!(cache.len(), 1);
    }
}
