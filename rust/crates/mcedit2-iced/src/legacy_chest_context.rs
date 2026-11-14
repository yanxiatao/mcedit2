//! 旧箱子的只读邻格配对；渲染状态与源状态、NBT 和拾取所有者分开。
use crate::textures::TextureBank;
use mcedit2_world::{sections::State, surface::SurfaceVoxel};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
type Position = (i32, i32, i32);
#[derive(Default)]
pub(crate) struct Context {
    states: HashMap<Position, Arc<State>>,
}
pub(crate) fn candidate(voxel: &SurfaceVoxel) -> bool {
    voxel.span == 1
        && matches!(
            voxel.state.name.as_str(),
            "minecraft:chest"
                | "minecraft:trapped_chest"
                | "minecraft:ender_chest"
                | "numeric:54"
                | "numeric:146"
                | "numeric:130"
        )
}
fn facing(state: &State) -> Option<&str> {
    state
        .properties
        .iter()
        .find(|(key, _)| key == "facing")
        .map(|(_, value)| value.as_str())
}
fn offset(p: Position, [x, z]: [i32; 2]) -> Option<Position> {
    Some((p.0.checked_add(x)?, p.1, p.2.checked_add(z)?))
}
impl Context {
    pub(crate) fn build<'a>(
        bank: Option<&'a TextureBank>,
        voxels: impl IntoIterator<Item = &'a SurfaceVoxel>,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        let Some(bank) = bank.filter(|b| b.has_legacy_chests()) else {
            return Some(Self::default());
        };
        let mut cells = HashMap::new();
        let mut warnings: BTreeMap<&str, usize> = BTreeMap::new();
        for voxel in voxels {
            if cancelled() {
                return None;
            }
            if !candidate(voxel) {
                continue;
            }
            if let Some(state) = bank.legacy_chest_state(&voxel.state) {
                let position = (voxel.x, voxel.y, voxel.z);
                if let Some((previous, old)) = cells.insert(position, (voxel, state)) {
                    if previous != voxel || old != state {
                        log::error!("[legacy-chest] {position:?} 存在不一致的重复源，拒绝构建预览");
                        return None;
                    }
                }
            }
        }
        let neighbors = |position: Position, state: &State| {
            [[1, 0], [-1, 0], [0, 1], [0, -1]]
                .into_iter()
                .filter_map(|step| {
                    let p = offset(position, step)?;
                    let (voxel, other) = cells.get(&p)?;
                    (voxel.original_block_entity && other.name == state.name).then_some((p, *other))
                })
                .collect::<Vec<_>>()
        };
        let mut result = Self::default();
        let mut variants = HashMap::new();
        for (&position, &(voxel, state)) in &cells {
            if cancelled() {
                return None;
            }
            let Some(dir) = facing(state) else {
                *warnings.entry("缺少源 facing，未解释邻箱关系").or_default() += 1;
                continue;
            };
            let axis = match dir {
                "south" => [1, 0],
                "west" => [0, 1],
                "north" => [-1, 0],
                "east" => [0, -1],
                _ => {
                    *warnings
                        .entry("源 facing 无效，未解释邻箱关系")
                        .or_default() += 1;
                    continue;
                }
            };
            let adjacent = neighbors(position, state);
            let mut kind = "single";
            if state.name != "minecraft:ender_chest"
                && voxel.original_block_entity
                && !adjacent.is_empty()
            {
                if adjacent.len() == 1
                    && facing(adjacent[0].1) == Some(dir)
                    && neighbors(adjacent[0].0, adjacent[0].1).len() == 1
                {
                    if offset(position, axis) == Some(adjacent[0].0) {
                        kind = "right";
                    } else if offset(position, axis.map(|n| -n)) == Some(adjacent[0].0) {
                        kind = "left";
                    } else {
                        kind = "invalid";
                        *warnings
                            .entry("邻箱不在同一横轴，显示缺失材质")
                            .or_default() += 1;
                    }
                } else {
                    kind = "invalid";
                    *warnings
                        .entry("邻箱方向或配对不唯一，显示缺失材质")
                        .or_default() += 1;
                }
            }
            let key = (state.name.as_str(), state.properties.as_slice(), kind);
            let rendered = variants.entry(key).or_insert_with(|| {
                let mut rendered = state.clone();
                rendered.properties.retain(|(key, _)| key != "type");
                rendered.properties.push(("type".into(), kind.into()));
                rendered.properties.sort();
                Arc::new(rendered)
            });
            result.states.insert(position, Arc::clone(rendered));
        }
        for (warning, count) in warnings {
            log::error!("[legacy-chest] {warning}（{count} 个源格）");
        }
        Some(result)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
    pub(crate) fn get(&self, voxel: &SurfaceVoxel) -> Option<&Arc<State>> {
        (voxel.span == 1)
            .then(|| self.states.get(&(voxel.x, voxel.y, voxel.z)))
            .flatten()
    }
    pub(crate) fn state<'a>(&'a self, voxel: &'a SurfaceVoxel) -> &'a Arc<State> {
        self.get(voxel).unwrap_or(&voxel.state)
    }
    pub(crate) fn voxel(
        &self,
        source: &SurfaceVoxel,
        palette: &mut mcedit2_world::surface::VoxelPalette,
    ) -> Option<SurfaceVoxel> {
        self.get(source)
            .map(|state| palette.updated(source, |data| data.state = Arc::clone(state)))
    }
}
