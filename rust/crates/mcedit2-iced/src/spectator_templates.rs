//! 精确非流体模型的局部源面；坐标与邻面掩码在实例化时应用，不合并原始方块。
use super::*;
use mcedit2_world::sections::State;

#[derive(Default)]
pub(super) struct Variants<'a>(
    HashMap<ModelCacheKey<'a>, Option<crate::resource_models::VariantPlan>>,
);
impl<'a> Variants<'a> {
    pub fn at(
        &mut self,
        bank: Option<&TextureBank>,
        state: &'a State,
        pos: (i32, i32, i32),
    ) -> u64 {
        self.0
            .entry(model_cache_key(state, 0))
            .or_insert_with(|| bank.and_then(|b| b.model_variant_plan(state)))
            .as_ref()
            .map_or(0, |plan| plan.at(pos))
    }
}
pub(super) type Key<'a> = (ModelCacheKey<'a>, u8);
pub(crate) struct Template {
    budget: Option<crate::resource_budget::ResourceBudget>,
    faces: Vec<FaceTemplate>,
    /// 每 64 个原始面保留全部邻面掩码的位图；按源顺序直接枚举可见面。
    visible: Vec<[u64; 64]>,
}
/// 冻结热模板；不保留世界来源或历代缓存链。
pub(crate) type Cache = HashMap<Arc<State>, HashMap<(u64, u8), Arc<Template>>>;
struct FaceTemplate {
    quad: WorldQuad,
    tint: Option<u32>,
    reverse: bool,
    required: u8,
    repeating: RepeatingFace,
}
impl Template {
    pub fn bytes(&self) -> usize {
        self.faces.capacity() * std::mem::size_of::<FaceTemplate>()
            + self.visible.capacity() * std::mem::size_of::<[u64; 64]>()
    }
    pub fn biome_dependent(&self) -> bool {
        self.faces.iter().any(|face| face.tint.is_some())
    }
    pub fn compile(
        view: &SpectatorScene,
        voxel: &SurfaceVoxel,
        default_uv: [[f32; 2]; 4],
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        Self::compile_model(
            view.texture_bank.as_deref(),
            view.show_textures,
            true,
            voxel,
            default_uv,
            cancelled,
        )
    }
    pub(super) fn compile_model(
        bank: Option<&TextureBank>,
        textures: bool,
        dynamic_tints: bool,
        voxel: &SurfaceVoxel,
        default_uv: [[f32; 2]; 4],
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        Self::compile_model_inner(
            bank,
            textures,
            dynamic_tints,
            voxel,
            default_uv,
            cancelled,
            false,
        )
    }
    /// renderSingleBlock 没有世界颜色上下文；所有带 tintindex 的源面使用 BlockColors 的索引 0。
    pub(super) fn compile_single_block(
        bank: &TextureBank,
        voxel: &SurfaceVoxel,
        default_uv: [[f32; 2]; 4],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        let mut template =
            Self::compile_model_inner(Some(bank), true, false, voxel, default_uv, cancelled, true)
                .ok_or("独立方块模型准备已取消")?;
        if template.faces.iter().any(|face| face.tint.is_some()) {
            let tint = bank.single_block_tint(&voxel.state)?;
            for face in &mut template.faces {
                if face.tint.take().is_some() {
                    for color in &mut face.quad.rgba {
                        for c in 0..3 {
                            color[c] *= tint[c];
                        }
                    }
                }
            }
        }
        Ok(template)
    }
    #[inline(never)]
    fn compile_model_inner(
        bank: Option<&TextureBank>,
        textures: bool,
        dynamic_tints: bool,
        voxel: &SurfaceVoxel,
        default_uv: [[f32; 2]; 4],
        cancelled: &dyn Fn() -> bool,
        single_block: bool,
    ) -> Option<Self> {
        let atlas = bank.and_then(|b| b.atlas());
        let tint_pos = (voxel.x, voxel.y, voxel.z);
        let pos = if single_block {
            bank?.single_block_sample()
        } else {
            crate::resource_models::ModelSample::Position(tint_pos)
        };
        let mut faces = Vec::new();
        if let Some(triangles) = bank.and_then(|b| b.static_obj_at(&voxel.state, pos)) {
            for tri in triangles {
                if cancelled() {
                    return None;
                }
                let dynamic = if single_block {
                    tri.tint_index.map(|_| 0)
                } else {
                    tri.tint_index.filter(|&index| {
                        dynamic_tints && bank.is_some_and(|b| b.dynamic_tint(&voxel.state, index))
                    })
                };
                let fixed = tri
                    .tint_index
                    .filter(|_| dynamic.is_none())
                    .map_or([1.; 3], |index| {
                        bank.unwrap()
                            .static_obj_tint(&voxel.state, tint_pos, None, index)
                    });
                let plain = rgba(voxel, Face::Top);
                let colors = tri.rgba.map(|mut color| {
                    for c in 0..3 {
                        color[c] *= fixed[c];
                    }
                    if !textures {
                        color[0] *= plain.r;
                        color[1] *= plain.g;
                        color[2] *= plain.b;
                        color[3] *= plain.a;
                    }
                    color
                });
                let uv = if textures { tri.uv } else { [default_uv[0]; 3] };
                faces.push(FaceTemplate {
                    tint: dynamic,
                    quad: WorldQuad {
                        xyz: [tri.xyz[0], tri.xyz[1], tri.xyz[2], tri.xyz[2]],
                        uv: [uv[0], uv[1], uv[2], uv[2]],
                        rgba: [colors[0], colors[1], colors[2], colors[2]],
                        mode: tri.mode,
                        atlas_page: if textures {
                            tri.page
                        } else {
                            atlas.map_or(0, |a| a.white.page)
                        },
                        light: crate::lighting::FULLBRIGHT,
                    },
                    required: tri.cull.map_or(0, Face::flag),
                    reverse: false,
                    repeating: None,
                });
            }
        }
        let display_state = bank.map_or(voxel.state.as_ref(), |b| b.render_state(&voxel.state));
        let display_kind = bank.map_or(voxel.kind, |b| b.render_material(&voxel.state, voxel.kind));
        let elements = bank
            .and_then(|b| {
                if single_block {
                    b.single_block_geometry(&voxel.state)
                } else {
                    b.geometry_at(&voxel.state, pos)
                }
            })
            .unwrap_or_else(|| {
                if single_block {
                    return Vec::new();
                }
                models::boxes(display_state)
                    .into_iter()
                    .map(models::Part::cube)
                    .collect()
            });
        for (index, element) in elements.iter().enumerate() {
            for face in FACES {
                if cancelled() {
                    return None;
                }
                let details = (
                    bank.and_then(|b| b.face_defined_at(&voxel.state, face, index, pos)),
                    bank.and_then(|b| b.face_cull_direction_at(&voxel.state, face, index, pos)),
                );
                let Some(required) = required_face(element, face, details) else {
                    continue;
                };
                if !single_block && internally_occluded(&elements, index, face) {
                    continue;
                }
                let style = if textures {
                    bank.and_then(|b| b.atlas_face_style_at(&voxel.state, face, index, pos))
                } else {
                    None
                };
                let tint = style.and_then(|_| {
                    bank.and_then(|b| b.face_tint_index(&voxel.state, face, index, pos))
                });
                let dynamic = if single_block {
                    tint.map(|_| 0)
                } else {
                    tint.filter(|&index| {
                        dynamic_tints && bank.is_some_and(|b| b.dynamic_tint(&voxel.state, index))
                    })
                };
                let (uv, mut shade, mode, page) = match style {
                    Some((uv, mode, page)) => (uv, Color::WHITE, mode, page),
                    None => (
                        default_uv,
                        rgba(voxel, face),
                        AlphaMode::Opaque,
                        atlas.map_or(0, |a| a.white.page),
                    ),
                };
                if let Some(index) = tint.filter(|_| dynamic.is_none()) {
                    let fixed = bank
                        .unwrap()
                        .static_obj_tint(&voxel.state, tint_pos, None, index);
                    shade.r *= fixed[0];
                    shade.g *= fixed[1];
                    shade.b *= fixed[2];
                }
                let (mut mode, alpha) =
                    crate::textures::classify_block_alpha(display_state, display_kind, mode);
                shade.a *= alpha;
                if shade.a < 0.999 {
                    mode = AlphaMode::Blend;
                }
                let mut quad = WorldQuad {
                    xyz: element.face_vertices(face.corners()),
                    uv,
                    rgba: [[shade.r, shade.g, shade.b, shade.a]; 4],
                    mode: mode.gpu_bits() | crate::material_mode::SINGLE_SIDED,
                    atlas_page: page,
                    light: crate::lighting::FULLBRIGHT,
                };
                quad.classify_axis_facing();
                let mut oriented = quad;
                let reverse = oriented.align_winding(element.normal(face.normal()));
                let repeating = if textures {
                    bank.and_then(|b| b.atlas_repeating_face_at(&voxel.state, face, index, pos))
                } else {
                    None
                };
                faces.push(FaceTemplate {
                    quad,
                    tint: dynamic,
                    reverse,
                    required,
                    repeating,
                });
            }
        }
        let mut visible = vec![[0; 64]; faces.len().div_ceil(64)];
        for (index, face) in faces.iter().enumerate() {
            for (flags, bits) in visible[index / 64].iter_mut().enumerate() {
                if face.required == 0 || flags as u8 & face.required != 0 {
                    *bits |= 1u64 << (index % 64);
                }
            }
        }
        Some(Self {
            faces,
            visible,
            budget: bank.and_then(TextureBank::geometry_budget),
        })
    }
    pub fn append(
        &self,
        out: &mut Vec<WorldQuad>,
        flags: u8,
        delta: [f32; 3],
        voxel: &SurfaceVoxel,
        colors: &mut Option<crate::textures::TintContext<'_>>,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<()> {
        let mut tint_cache: Option<(u32, [f32; 3])> = None;
        for (word, visible) in self.visible.iter().enumerate() {
            if cancelled() {
                return None;
            }
            let mut bits = visible[usize::from(flags & 63)];
            while bits != 0 {
                let index = word * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let face = &self.faces[index];
                let mut quad = face.quad.translated(delta);
                if let (Some(index), Some(colors)) = (face.tint, colors.as_mut()) {
                    let tint =
                        if let Some((_, tint)) = tint_cache.filter(|(prior, _)| *prior == index) {
                            tint
                        } else {
                            let tint = colors.block(
                                &voxel.state,
                                index,
                                voxel.biome.as_deref(),
                                (voxel.x, voxel.y, voxel.z),
                            );
                            tint_cache = Some((index, tint));
                            tint
                        };
                    for color in &mut quad.rgba {
                        for c in 0..3 {
                            color[c] *= tint[c];
                        }
                    }
                }
                if let Some((uv, region, sampler)) = &face.repeating {
                    if crate::repeating_uv::address(
                        quad.xyz,
                        *uv,
                        region,
                        *sampler,
                        self.budget,
                        |xyz, uv| {
                            let mut tile = WorldQuad { xyz, uv, ..quad };
                            if face.reverse {
                                tile.reverse_winding();
                            }
                            out.push(tile)
                        },
                    ) {
                        continue;
                    }
                    log::error!("[models] {} 的源 UV 寻址超出安全范围", voxel.state.name);
                    return None;
                }
                let mut quad = quad;
                if face.reverse {
                    quad.reverse_winding();
                }
                out.push(quad);
            }
        }
        Some(())
    }
}

#[cfg(test)]
#[path = "spectator_template_tests.rs"]
mod tests;
