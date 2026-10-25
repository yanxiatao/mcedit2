//! One projected quad per original surface face; UVs point into the safe
//! prepacked resource atlas. Pixel sampling is performed on the GPU, not by
//! subdividing faces on the CPU. Original source/pick ordering is unchanged.
use crate::isometric::IsometricSurface;
use crate::textures::{AlphaMode, AtlasUv};
use iced::{Color, Point, Size};
use std::collections::HashMap;
const MAX_TRIANGLES: usize = 350_000;
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
    /// The two camera modes share one wgpu pipeline.
    pub world_mode: u32,
    /// Array layer in the multi-page resource-pack block atlas.
    pub atlas_page: u32,
}
impl Vertex {
    fn new(p: Point, depth: f32, c: Color, uv: [f32; 2], mode: AlphaMode) -> Self {
        Self {
            xy: [p.x, p.y],
            depth,
            rgba: [c.r, c.g, c.b, c.a],
            uv,
            mode: mode as u32,
            xyz: [0.0; 3],
            world_mode: 0,
            atlas_page: 0,
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
/// Conservative face and vertex limit; GPU path does not tessellate one
/// Minecraft block into dozens of CPU-subdivided tiles anymore.
pub(crate) fn build(view: &IsometricSurface, size: Size) -> Vec<Vertex> {
    let faces = view.faces_cached(size);
    let mut out = Vec::with_capacity(faces.len().min(MAX_TRIANGLES / 2) * 6);
    let atlas = view.texture_bank.as_ref().and_then(|bank| bank.atlas());
    let mut style_cache: HashMap<
        (String, Vec<(String, String)>, u8, usize, u64),
        Option<([[f32; 2]; 4], AlphaMode, u32)>,
    > = HashMap::new();
    for face in faces.iter().take(MAX_TRIANGLES / 2) {
        let voxel = &view.surface.voxels[face.voxel_index];
        let variant = if voxel.span > 1 {
            0
        } else {
            view.texture_bank
                .as_ref()
                .map_or(0, |bank| bank.model_variant_key(&voxel.state, face.pos))
        };
        let cache_key = (
            voxel.state.name.clone(),
            voxel.state.properties.clone(),
            face.face as u8,
            face.element_index,
            variant,
        );
        let style = style_cache.entry(cache_key).or_insert_with(|| {
            if view.show_textures {
                view.texture_bank.as_ref().and_then(|bank| {
                    bank.atlas_face_style_at(&voxel.state, face.face, face.element_index, face.pos)
                })
            } else {
                None
            }
        });
        let texture = style.as_ref().map(|(uv, _, _)| *uv);
        let color = if texture.is_some() {
            // Lighting remains a per-face multiplier. Transparency comes
            // from the PNG's original alpha, not a CPU alpha threshold.
            let shade = 1.0;
            let (_, alpha) = crate::textures::classify_block_alpha(
                &voxel.state,
                voxel.kind,
                style
                    .as_ref()
                    .map(|(_, a, _)| *a)
                    .unwrap_or(AlphaMode::Opaque),
            );
            let tint = view
                .texture_bank
                .as_ref()
                .map(|bank| {
                    bank.face_tint_with_biome_at(
                        &voxel.state,
                        face.face,
                        face.element_index,
                        face.pos,
                        voxel.biome.as_deref(),
                    )
                })
                .unwrap_or([1.0; 3]);
            Color::from_rgba(shade * tint[0], shade * tint[1], shade * tint[2], alpha)
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
        mode = crate::textures::classify_block_alpha(&voxel.state, voxel.kind, mode).0;
        if color.a < 0.999 {
            mode = AlphaMode::Blend;
        }
        quad(
            &mut out,
            face.corners,
            face.vertex_depths,
            color,
            coords,
            mode,
            page,
        );
    }
    out
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
        assert_eq!(vertices[0].mode, AlphaMode::Blend as u32);
        assert_eq!(std::mem::size_of::<Vertex>(), 60);
        assert_eq!(vertices[0].world_mode, 0);
    }
    #[test]
    fn gpu_style_cache_distinguishes_model_element_and_face() {
        let mut cache: HashMap<(String, Vec<(String, String)>, u8, usize), usize> = HashMap::new();
        cache.insert(("test:machine".to_string(), vec![], 0u8, 0usize), 1usize);
        cache.insert(("test:machine".to_string(), vec![], 0u8, 1usize), 2usize);
        assert_eq!(cache.len(), 2);
        assert_ne!(
            cache.get(&("test:machine".into(), vec![], 0, 0)),
            cache.get(&("test:machine".into(), vec![], 0, 1))
        );
    }
    #[test]
    fn gpu_quality_cycles_with_no_cpu_per_pixel_tessellation() {
        assert_eq!(Quality::Fast.next(), Quality::Balanced);
        assert_eq!(Quality::Balanced.next(), Quality::Detailed);
        assert_eq!(Quality::Detailed.next(), Quality::Fast);
        assert!(MAX_TRIANGLES * 3 < u32::MAX as usize);
    }
}
