//! 每个后台源读取器的资源可见性快照，按实际状态和加权模型缓存分类。
use crate::{resource_models::VariantPlan, textures::TextureBank};
use mcedit2_world::{
    sections::State,
    surface::VoxelMaterial,
    surface_visibility::{Cell, CellInfo, Direction, Native, Visibility},
};
use std::{collections::HashMap, sync::Arc};
#[derive(Clone, Copy)]
struct ModelInfo {
    opaque: u8,
    unconditional: bool,
}
struct Choices {
    state: Arc<State>,
    plan: Option<VariantPlan>,
    models: HashMap<(u64, u8), ModelInfo>,
    fluid_shape: Option<Option<mcedit2_world::fluid_occlusion::Occlusion>>,
}
pub(crate) struct Reader {
    bank: Option<Arc<TextureBank>>,
    show_textures: bool,
    block_layers: crate::block_visibility::Layers,
    states: HashMap<Arc<State>, Choices>,
    #[cfg(test)]
    fixed_queries: bool,
    /// 连续扫描同种石料时直接复用固定模型；加权变体仍按真实坐标求值。
    last_fixed: Option<(Arc<State>, VoxelMaterial, ModelInfo)>,
}
impl Reader {
    #[cfg(test)]
    pub(crate) fn disable_fixed_queries(&mut self) {
        self.fixed_queries = false;
    }
    pub(crate) fn new(bank: Option<Arc<TextureBank>>, show_textures: bool) -> Self {
        let block_layers = bank
            .as_ref()
            .map_or_else(Default::default, |bank| bank.block_layers());
        Self {
            bank,
            show_textures,
            block_layers,
            states: HashMap::new(),
            #[cfg(test)]
            fixed_queries: true,
            last_fixed: None,
        }
    }
    pub(crate) fn with_layers(mut self, layers: crate::block_visibility::Layers) -> Self {
        self.block_layers = layers;
        self
    }
    fn info(&mut self, cell: Cell<'_>) -> ModelInfo {
        if let Some((state, kind, info)) = &self.last_fixed {
            if *kind == cell.kind && state.as_ref() == cell.state {
                return *info;
            }
        }
        let bank = self.bank.as_ref().expect("资源读取器查询前已选择资源模式");
        let choices = match self.states.get_mut(cell.state) {
            Some(choices) => choices,
            None => {
                let state = Arc::new(cell.state.clone());
                self.states
                    .entry(Arc::clone(&state))
                    .or_insert_with(|| Choices {
                        state,
                        plan: bank.model_variant_plan(cell.state),
                        models: HashMap::new(),
                        fluid_shape: None,
                    })
            }
        };
        let variant = choices
            .plan
            .as_ref()
            .map_or(0, |plan| plan.at(cell.position));
        let info = *choices
            .models
            .entry((variant, cell.kind as u8))
            .or_insert_with(|| ModelInfo {
                opaque: bank.opaque_boundary_mask(
                    cell.state,
                    cell.position,
                    cell.kind,
                    self.show_textures,
                ),
                unconditional: bank.has_unconditional_geometry(cell.state, cell.position),
            });
        if !choices
            .plan
            .as_ref()
            .is_some_and(VariantPlan::position_dependent)
        {
            self.last_fixed = Some((Arc::clone(&choices.state), cell.kind, info));
        }
        info
    }
}
impl Visibility for Reader {
    fn fixed_fluid(
        &mut self,
        state: &State,
        kind: VoxelMaterial,
    ) -> Result<Option<bool>, mcedit2_world::WorldError> {
        if !self.prepare_cells() {
            return Ok(None);
        }
        self.fluid_sample(Cell {
            state,
            kind,
            position: (0, 0, 0),
        })
        .map(|sample| {
            Some(matches!(
                sample,
                mcedit2_world::fluid_surface::Sample::Fluid(..)
            ))
        })
    }
    fn prepare_cells(&self) -> bool {
        #[cfg(test)]
        if !self.fixed_queries {
            return false;
        }
        self.bank.is_some()
    }
    fn evaluate_cell(&mut self, cell: Cell<'_>) -> Option<CellInfo> {
        if !self.prepare_cells() {
            return None;
        }
        let bank = self.bank.as_ref()?;
        if !self.block_layers.visible(cell.state, Some(bank)) {
            return Some(CellInfo {
                visible: false,
                opaque: 0,
                ordinary_faces: true,
                unconditional: false,
            });
        }
        let rendered = bank.render_state(cell.state);
        let liquid = bank.render_material(cell.state, cell.kind) == VoxelMaterial::Liquid;
        let ordinary_faces = !(liquid && mcedit2_world::fluid_surface::kind(rendered).is_some());
        let unconditional_allowed =
            !(liquid && mcedit2_world::fluid_surface::is_named_fluid(rendered));
        let info = self.info(cell);
        Some(CellInfo {
            visible: true,
            opaque: info.opaque,
            ordinary_faces,
            unconditional: unconditional_allowed && info.unconditional,
        })
    }
    fn fixed_cell(&mut self, state: &State, kind: VoxelMaterial) -> Option<CellInfo> {
        let info = self.evaluate_cell(Cell {
            state,
            kind,
            position: (0, 0, 0),
        })?;
        if info.visible
            && self
                .states
                .get(state)
                .expect("实际状态模型已进入缓存")
                .plan
                .as_ref()
                .is_some_and(VariantPlan::position_dependent)
        {
            return None;
        }
        Some(info)
    }

    fn track_sections(&self) -> bool {
        self.bank.is_some() && (self.show_textures || !self.block_layers.blocks)
    }
    fn opaque_cell(&mut self, source: Cell<'_>) -> bool {
        self.block_layers
            .visible(source.state, self.bank.as_deref())
            && self.bank.is_some()
            && self.info(source).opaque == mcedit2_world::surface::ALL_FACES
    }
    fn fluid_occlusion(
        &mut self,
        source: Cell<'_>,
    ) -> Result<mcedit2_world::fluid_occlusion::Occlusion, mcedit2_world::WorldError> {
        if !self
            .block_layers
            .visible(source.state, self.bank.as_deref())
        {
            return Ok(mcedit2_world::fluid_occlusion::Occlusion::Boundary(0));
        }
        let Some(bank) = self.bank.as_ref() else {
            return Native.fluid_occlusion(source);
        };
        let shape = if let Some(cached) = self.states.get(source.state).and_then(|c| c.fluid_shape)
        {
            cached
        } else {
            let shape = mcedit2_world::fluid_occlusion::vanilla(bank.render_state(source.state))?;
            self.info(source);
            self.states
                .get_mut(source.state)
                .expect("已缓存源状态")
                .fluid_shape = Some(shape);
            shape
        };
        Ok(shape.unwrap_or_else(|| {
            mcedit2_world::fluid_occlusion::Occlusion::Boundary(self.info(source).opaque)
        }))
    }
    fn water_overlay(&self, neighbor: Cell<'_>) -> bool {
        self.bank.as_ref().map_or_else(
            || Native.water_overlay(neighbor),
            |bank| mcedit2_world::native_material::water_overlay(bank.render_state(neighbor.state)),
        )
    }
    fn resource_aware(&self) -> bool {
        self.bank.is_some()
    }
    fn exposed(
        &mut self,
        source: Cell<'_>,
        direction: Direction,
        neighbor: Option<Cell<'_>>,
    ) -> bool {
        if !self
            .block_layers
            .visible(source.state, self.bank.as_deref())
        {
            return false;
        }
        let Some(bank) = &self.bank else {
            return Native.exposed(source, direction, neighbor);
        };
        let Some(neighbor) = neighbor else {
            return true;
        };
        let current = bank.render_state(source.state);
        let other = bank.render_state(neighbor.state);
        if matches!(
            other.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "numeric:0"
        ) {
            return true;
        }
        // 原生流体由既有专用几何生成；只保留其同种流体内部面的既有语义。
        let current_kind = bank.render_material(source.state, source.kind);
        if current_kind == VoxelMaterial::Liquid
            && mcedit2_world::fluid_surface::kind(current).is_some()
        {
            return mcedit2_world::fluid_surface::kind(current)
                != mcedit2_world::fluid_surface::kind(other);
        }
        if current_kind == VoxelMaterial::Transparent
            && mcedit2_world::native_material::skip_same(current, other)
        {
            return false;
        }
        self.info(neighbor).opaque & direction.opposite().flag() == 0
    }
    fn unconditional(&mut self, source: Cell<'_>) -> bool {
        if !self
            .block_layers
            .visible(source.state, self.bank.as_deref())
        {
            return false;
        }
        if self.bank.as_ref().is_some_and(|bank| {
            bank.render_material(source.state, source.kind) == VoxelMaterial::Liquid
                && mcedit2_world::fluid_surface::is_named_fluid(bank.render_state(source.state))
        }) {
            // 原生流体的每个面都服从真实邻面标记，8/9 高度不是无条件内部模型。
            return false;
        }
        if self.bank.is_none() {
            Native.unconditional(source)
        } else {
            self.info(source).unconditional
        }
    }
    fn fluid_sample(
        &mut self,
        source: Cell<'_>,
    ) -> Result<mcedit2_world::fluid_surface::Sample, mcedit2_world::WorldError> {
        if !self.block_layers.blocks {
            return Ok(mcedit2_world::fluid_surface::Sample::Other);
        }
        let Some(bank) = self.bank.as_ref() else {
            return Native.fluid_sample(source);
        };
        mcedit2_world::fluid_surface::Sample::vanilla(bank.render_state(source.state))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    assets: Option<crate::textures::VisibilityIdentity>,
    show_textures: bool,
    block_layers: crate::block_visibility::Layers,
}
pub(crate) fn identity(
    bank: Option<&Arc<TextureBank>>,
    show_textures: bool,
    block_layers: crate::block_visibility::Layers,
) -> Identity {
    Identity {
        assets: bank.map(|bank| bank.visibility_id()),
        show_textures,
        block_layers,
    }
}
