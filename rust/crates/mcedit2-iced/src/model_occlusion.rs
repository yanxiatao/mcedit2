//! 以实际不透明模型和已核实的原版同类邻面规则剔面，不修改源状态。
use crate::{
    gpu_mesh::{model_cache_key, ModelCacheKey},
    isometric::Face,
    resource_models::VariantPlan,
    spectator_stream::ExactNeighbors,
    textures::TextureBank,
};
use mcedit2_world::surface::SurfaceVoxel;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct Cache<'a> {
    variants: HashMap<ModelCacheKey<'a>, Option<VariantPlan>>,
    masks: HashMap<(ModelCacheKey<'a>, u8, bool), u8>,
    pub removed: usize,
}
impl<'a> Cache<'a> {
    pub(crate) fn faces(
        &mut self,
        bank: Option<&TextureBank>,
        index: &ExactNeighbors<'a>,
        voxel: &SurfaceVoxel,
        mut faces: u8,
        show_textures: bool,
    ) -> u8 {
        let Some(bank) = bank else {
            return faces;
        };
        if voxel.span != 1 {
            return faces;
        }
        let transparent = bank.render_material(&voxel.state, voxel.kind)
            == mcedit2_world::surface::VoxelMaterial::Transparent;
        let source = bank.render_state(&voxel.state);
        for face in [
            Face::Top,
            Face::Bottom,
            Face::PosX,
            Face::NegX,
            Face::PosZ,
            Face::NegZ,
        ] {
            if faces & face.flag() == 0 {
                continue;
            }
            let (dx, dy, dz) = face.normal();
            let (Some(x), Some(y), Some(z)) = (
                voxel.x.checked_add(dx),
                voxel.y.checked_add(dy),
                voxel.z.checked_add(dz),
            ) else {
                continue;
            };
            let Some(neighbor) = index.at((x, y, z)) else {
                continue;
            };
            if transparent
                && mcedit2_world::native_material::skip_same(
                    source,
                    bank.render_state(&neighbor.state),
                )
            {
                faces &= !face.flag();
                self.removed += 1;
                continue;
            }
            let variant = self
                .variants
                .entry(model_cache_key(&neighbor.state, 0))
                .or_insert_with(|| bank.model_variant_plan(&neighbor.state))
                .as_ref()
                .map_or(0, |plan| plan.at((x, y, z)));
            let mask = *self
                .masks
                .entry((
                    model_cache_key(&neighbor.state, variant),
                    neighbor.kind as u8,
                    show_textures,
                ))
                .or_insert_with(|| {
                    bank.opaque_boundary_mask(
                        &neighbor.state,
                        (x, y, z),
                        neighbor.kind,
                        show_textures,
                    )
                });
            let opposite = match face {
                Face::Top => Face::Bottom,
                Face::Bottom => Face::Top,
                Face::PosX => Face::NegX,
                Face::NegX => Face::PosX,
                Face::PosZ => Face::NegZ,
                Face::NegZ => Face::PosZ,
            };
            if mask & opposite.flag() != 0 {
                faces &= !face.flag();
                self.removed += 1;
            }
        }
        faces
    }
}
