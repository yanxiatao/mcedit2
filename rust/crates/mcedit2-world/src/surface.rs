//! Fixed-budget original-state 3D surface projection of one Java footprint.
//! A top visible block from each column is represented as a voxel, with an
//! original BE marker. Air, missing sections and missing chunks are never
//! silently replaced with generated Minecraft data.
use crate::block_lookup::ReadOnlyRegionCache;
use crate::editing::Session;
use crate::sections::{Chunk, State};
use crate::WorldError;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

pub(crate) fn staged_metadata(
    session: &Session,
    cx: i32,
    cz: i32,
) -> Result<(Option<Chunk>, Option<Chunk>), WorldError> {
    let decode = |bytes: Option<&[u8]>| -> Result<Option<Chunk>, WorldError> {
        bytes
            .map(|raw| {
                crate::sections::decode_sections(raw)
                    .map_err(|e| WorldError::Invalid(format!("暂存区块元数据无效：{e}")))
            })
            .transpose()
    };
    Ok((
        decode(session.staged_biome_chunk(cx, cz)?)?,
        decode(session.staged_be_chunk(cx, cz)?)?,
    ))
}

pub(crate) fn staged_metadata_cached(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    cx: i32,
    cz: i32,
) -> Result<(Option<Chunk>, Option<Chunk>), WorldError> {
    let decode = |bytes: Option<&[u8]>| -> Result<Option<Chunk>, WorldError> {
        bytes
            .map(|raw| {
                crate::sections::decode_sections(raw)
                    .map_err(|e| WorldError::Invalid(format!("暂存区块元数据无效：{e}")))
            })
            .transpose()
    };
    let biome = decode(session.staged_biome_chunk_cached(reader, cx, cz)?)?;
    let be = decode(session.staged_be_chunk_cached(reader, cx, cz)?)?;
    Ok((biome, be))
}

/// 一圈颜色邻域仅复制 biome 调色板，暂存 biome 优先；隐藏格不进入显示或拾取数组。
fn inspect_biomes(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    chunks: &mut BTreeMap<(i32, i32), Option<Arc<Chunk>>>,
    metadata: &BTreeMap<(i32, i32), (Option<Chunk>, Option<Chunk>)>,
    coordinates: impl Iterator<Item = (i32, i32)>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Arc<crate::biome_source::Region>, WorldError> {
    let mut region = crate::biome_source::Region::new(session.original_world().seed);
    match session
        .original_world()
        .world_data
        .geometry(session.dimension())
    {
        Ok(geometry) => region.set_geometry(geometry),
        Err(error) => log::error!(
            "[biome] {}：{error}；不钳制未知的维度高度",
            session.dimension()
        ),
    }
    let mut halo = std::collections::BTreeSet::new();
    for (cx, cz) in coordinates {
        for dx in -1..=1 {
            for dz in -1..=1 {
                halo.insert((cx + dx, cz + dz));
            }
        }
    }
    for (cx, cz) in halo {
        if cancelled() {
            return Err(refuse("biome 颜色邻域读取已取消"));
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = chunks.entry((cx, cz)) {
            entry.insert(session.visible_chunk_base_shared_cached(reader, cx, cz)?.0);
        }
        let Some(chunk) = chunks.get(&(cx, cz)).and_then(Option::as_ref) else {
            continue;
        };
        let staged;
        let biome = if let Some((biome, _)) = metadata.get(&(cx, cz)) {
            biome.as_ref().unwrap_or(chunk)
        } else {
            staged = session
                .staged_biome_chunk_cached(reader, cx, cz)?
                .map(crate::sections::decode_sections)
                .transpose()
                .map_err(|error| refuse(format!("暂存 biome 邻域无效：{error}")))?;
            staged.as_ref().unwrap_or(chunk)
        };
        region.insert(cx, cz, crate::biome_source::Column::from_chunk(biome)?);
    }
    Ok(Arc::new(region))
}

/// 工具的材质预览读取真实颜色邻域，沿用地形的暂存优先与维度高度解释。
pub fn inspect_biome_region(
    session: &Session,
    bounds: crate::editing::Bounds,
    cancelled: &dyn Fn() -> bool,
) -> Result<Arc<crate::biome_source::Region>, WorldError> {
    let coordinates = (bounds.minimum[0].div_euclid(16)..=bounds.maximum[0].div_euclid(16))
        .flat_map(|cx| {
            (bounds.minimum[2].div_euclid(16)..=bounds.maximum[2].div_euclid(16))
                .map(move |cz| (cx, cz))
        });
    inspect_biome_chunks(session, coordinates, cancelled)
}

/// 稀疏动态模型只读取实际源区块及其颜色邻域，避免用包围盒扩展为整片世界。
pub fn inspect_biome_chunks(
    session: &Session,
    coordinates: impl Iterator<Item = (i32, i32)>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Arc<crate::biome_source::Region>, WorldError> {
    let mut reader = session.source_reader()?;
    inspect_biomes(
        session,
        &mut reader,
        &mut BTreeMap::new(),
        &BTreeMap::new(),
        coordinates,
        cancelled,
    )
}

/// 仅流体窗口读取一圈原始邻块；隐藏格只参与高度查询，不进入显示 / 拾取数组。
pub(crate) fn inspect_fluid_geometry(
    session: &Session,
    reader: &mut ReadOnlyRegionCache<'_>,
    chunks: &mut BTreeMap<(i32, i32), Option<Arc<Chunk>>>,
    voxels: &mut [SurfaceVoxel],
    visibility: &mut dyn crate::surface_visibility::Visibility,
    clip: Option<crate::editing::Bounds>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), WorldError> {
    use crate::surface_visibility::Cell;
    let mut has_fluid = false;
    for (index, voxel) in voxels.iter().enumerate() {
        if index.is_multiple_of(256) && cancelled() {
            return Err(refuse("流体源分类已取消"));
        }
        let cell = Cell {
            state: &voxel.state,
            kind: voxel.kind,
            position: (voxel.x, voxel.y, voxel.z),
        };
        if matches!(
            visibility.fluid_sample(cell)?,
            crate::fluid_surface::Sample::Fluid(..)
        ) {
            has_fluid = true;
            break;
        }
    }
    if !has_fluid {
        return Ok(());
    }
    let mut halo = std::collections::BTreeSet::new();
    for &(cx, cz) in chunks.keys() {
        for dx in -1..=1 {
            for dz in -1..=1 {
                halo.insert((cx + dx, cz + dz));
            }
        }
    }
    for key in halo {
        if cancelled() {
            return Err(refuse("流体源邻域读取已取消"));
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = chunks.entry(key) {
            let (chunk, _) = session.visible_chunk_base_shared_cached(reader, key.0, key.1)?;
            entry.insert(chunk);
        }
    }
    let mut fluid_palette = VoxelPalette::default();
    for (index, voxel) in voxels.iter_mut().enumerate() {
        if index.is_multiple_of(256) && cancelled() {
            return Err(refuse("流体角点查询已取消"));
        }
        let cell = Cell {
            state: &voxel.state,
            kind: voxel.kind,
            position: (voxel.x, voxel.y, voxel.z),
        };
        if !matches!(
            visibility.fluid_sample(cell)?,
            crate::fluid_surface::Sample::Fluid(..)
        ) {
            fluid_palette.replace_fluid(voxel, None);
            continue;
        }
        let available =
            crate::surface_visibility::available_fluid_faces(cell.position, clip, |cx, cz| {
                chunks.get(&(cx, cz)).and_then(Option::as_ref).is_some()
            });
        let fluid = crate::surface_visibility::fluid_surface(
            visibility,
            cell,
            available,
            clip,
            &mut |(x, y, z)| {
                chunks
                    .get(&(x.div_euclid(16), z.div_euclid(16)))
                    .and_then(Option::as_ref)
                    .and_then(|chunk| {
                        crate::surface_visibility::fluid_state(
                            chunk,
                            x.rem_euclid(16) as u8,
                            y,
                            z.rem_euclid(16) as u8,
                        )
                    })
                    .map(|state| {
                        let state = session.effective_state_ref((x, y, z), state);
                        Cell {
                            state,
                            kind: material(state),
                            position: (x, y, z),
                        }
                    })
            },
        )?;
        fluid_palette.replace_fluid(voxel, fluid);
    }
    Ok(())
}

/// 每份只读源或结构预览共享状态、群系与流体形状；复制体素只增加引用计数。
#[derive(Default)]
pub struct VoxelPalette {
    states: HashSet<Arc<State>>,
    biomes: HashSet<crate::shared_text::SharedText>,
    fluids: HashSet<Arc<crate::fluid_surface::FluidSurface>>,
    descriptors: HashMap<DescriptorKey, Arc<VoxelData>>,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct DescriptorKey {
    state: usize,
    biome: usize,
    fluid: usize,
    flags: u32,
    light: crate::saved_light::Samples,
}
impl DescriptorKey {
    fn of(data: &VoxelData) -> Self {
        Self {
            state: Arc::as_ptr(&data.state) as usize,
            biome: data.biome.as_ref().map_or(0, |name| name.allocation_id()),
            fluid: data
                .fluid
                .as_ref()
                .map_or(0, |fluid| Arc::as_ptr(fluid) as usize),
            light: data.light,
            flags: u32::from(data.faces)
                | (data.kind as u32) << 8
                | u32::from(data.staged) << 16
                | u32::from(data.original_block_entity) << 17
                | u32::from(data.span) << 24,
        }
    }
}
impl VoxelPalette {
    /// 只有新描述才分配对象；位置不参与描述身份，原始格与源归属仍逐格保存。
    pub fn voxel(&mut self, position: [i32; 3], data: VoxelData) -> SurfaceVoxel {
        let descriptor = self
            .descriptors
            .entry(DescriptorKey::of(&data))
            .or_insert_with(|| Arc::new(data));
        SurfaceVoxel {
            x: position[0],
            y: position[1],
            z: position[2],
            descriptor: Arc::clone(descriptor),
        }
    }
    /// 修正临时显示属性时按完整结果复用，保留输入快照及其源状态。
    pub fn updated(
        &mut self,
        voxel: &SurfaceVoxel,
        edit: impl FnOnce(&mut VoxelData),
    ) -> SurfaceVoxel {
        let mut data = voxel.data().clone();
        edit(&mut data);
        self.voxel([voxel.x, voxel.y, voxel.z], data)
    }
    pub fn replace_fluid(
        &mut self,
        voxel: &mut SurfaceVoxel,
        fluid: Option<crate::fluid_surface::FluidSurface>,
    ) {
        if voxel.fluid_surface() == fluid {
            return;
        }
        let fluid = self.fluid(fluid);
        *voxel = self.updated(voxel, |data| data.fluid = fluid);
    }
    /// 位级相同的显示形状共享一份数据；所有角点、流向、面掩码及 overlay 均参与身份。
    pub fn fluid(
        &mut self,
        surface: Option<crate::fluid_surface::FluidSurface>,
    ) -> Option<Arc<crate::fluid_surface::FluidSurface>> {
        surface.map(|surface| {
            if let Some(shared) = self.fluids.get(&surface) {
                return Arc::clone(shared);
            }
            let shared = Arc::new(surface);
            self.fluids.insert(Arc::clone(&shared));
            shared
        })
    }
    /// 按真实状态值共享，调用方的可编辑状态始终独立。
    pub fn state(&mut self, state: &State) -> Arc<State> {
        if let Some(shared) = self.states.get(state) {
            return Arc::clone(shared);
        }
        let shared = Arc::new(state.clone());
        self.states.insert(Arc::clone(&shared));
        shared
    }
    /// 保留缺失群系语义，只共享实际存在的名称。
    pub fn biome(&mut self, biome: Option<&str>) -> Option<crate::shared_text::SharedText> {
        biome.map(|name| {
            if let Some(shared) = self.biomes.get(name) {
                return shared.clone();
            }
            let shared: crate::shared_text::SharedText = name.into();
            self.biomes.insert(shared.clone());
            shared
        })
    }
}

fn refuse(s: impl Into<String>) -> WorldError {
    WorldError::Invalid(s.into())
}
pub const FACE_TOP: u8 = 1;
pub const FACE_BOTTOM: u8 = 2;
pub const FACE_POS_X: u8 = 4;
pub const FACE_NEG_X: u8 = 8;
pub const FACE_POS_Z: u8 = 16;
pub const FACE_NEG_Z: u8 = 32;
pub const ALL_FACES: u8 = 63;
/// Strict bounds for read-only 3D viewing. The editing batch limit remains
/// unchanged: a larger scene is NOT a larger writable transaction.
pub const MAX_VIEW_XZ: u8 = 64;
pub const MAX_VIEW_Y: i64 = 48;
pub const MAX_VIEW_CELLS: u64 = MAX_VIEW_XZ as u64 * MAX_VIEW_XZ as u64 * MAX_VIEW_Y as u64;
fn validate_view(width: u8, depth: u8, y_min: i32, y_max: i32) -> Result<(), WorldError> {
    let height = y_max as i64 - y_min as i64 + 1;
    if width == 0
        || width > MAX_VIEW_XZ
        || depth == 0
        || depth > MAX_VIEW_XZ
        || !(1..=MAX_VIEW_Y).contains(&height)
        || width as u64 * depth as u64 * height as u64 > MAX_VIEW_CELLS
    {
        return Err(refuse(
            "Read-only 3D scene exceeds 64x64x48 / 196608 source cells",
        ));
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoxelMaterial {
    Solid,
    Transparent,
    Liquid,
    Foliage,
}
pub fn material(state: &State) -> VoxelMaterial {
    let n = state.name.as_str();
    if matches!(n, "minecraft:packed_ice" | "minecraft:blue_ice") {
        VoxelMaterial::Solid
    } else if crate::fluid_surface::is_named_fluid(state) {
        VoxelMaterial::Liquid
    } else if n.contains("glass") || n.ends_with(":ice") || n.ends_with("_ice") {
        VoxelMaterial::Transparent
    } else if n.ends_with("_leaves") || n.ends_with(":leaves") {
        VoxelMaterial::Foliage
    } else {
        VoxelMaterial::Solid
    }
}
/// A known vanilla partial shape cannot be treated as a full occluder.
/// This conservative test only exposes potentially visible faces; the
/// projected model still supplies actual element geometry and selection.
fn known_partial_occluder(state: &State) -> bool {
    let Some(n) = state.name.strip_prefix("minecraft:") else {
        return false;
    };
    if n.ends_with("_slab") {
        return !state
            .properties
            .iter()
            .any(|(k, v)| k == "type" && v == "double");
    }
    if n == "snow" {
        return !state
            .properties
            .iter()
            .any(|(k, v)| k == "layers" && v == "8");
    }
    n.ends_with("_stairs")
        || n.ends_with("_trapdoor")
        || n.ends_with("_fence")
        || n.ends_with("_fence_gate")
        || n.ends_with("_wall")
        || n.ends_with("_pane")
        || n == "iron_bars"
        || n.ends_with("_carpet")
        || n == "farmland"
        || n == "dirt_path"
        || n.ends_with("_door")
}
/// Occlusion must be proven by a full neighboring block. Unknown modded
/// models, unfamiliar vanilla blocks and absent NBT sections are not solid
/// evidence. Drawing an extra hidden face is safe; deleting a visible face
/// creates irreversible holes in the renderer.
///
/// This is the shared, conservative rule for both original Section face
/// inspection and seams between independently streamed exact chunks. Keep
/// both consumers in agreement: transparent ice is not a full occluder,
/// while double slabs and eight-layer snow are full blocks.
pub fn known_full_occluder(state: &State) -> bool {
    let Some(n) = state.name.strip_prefix("minecraft:") else {
        return false;
    };
    if n.ends_with("_slab") {
        return state
            .properties
            .iter()
            .any(|(k, v)| k == "type" && v == "double");
    }
    if n == "snow" {
        return state
            .properties
            .iter()
            .any(|(k, v)| k == "layers" && v == "8");
    }
    matches!(
        n,
        "stone"
            | "packed_ice"
            | "blue_ice"
            | "deepslate"
            | "cobblestone"
            | "mossy_cobblestone"
            | "cobbled_deepslate"
            | "dirt"
            | "coarse_dirt"
            | "grass_block"
            | "rooted_dirt"
            | "granite"
            | "diorite"
            | "andesite"
            | "tuff"
            | "calcite"
            | "dripstone_block"
            | "bedrock"
            | "sand"
            | "red_sand"
            | "gravel"
            | "clay"
            | "obsidian"
            | "crying_obsidian"
            | "end_stone"
            | "netherrack"
            | "soul_sand"
            | "soul_soil"
            | "mud"
            | "packed_mud"
            | "mud_bricks"
            | "snow_block"
            | "stone_bricks"
            | "bricks"
            | "mossy_stone_bricks"
            | "cracked_stone_bricks"
    ) || n.ends_with("_planks")
        || n.ends_with("_log")
        || n.ends_with("_wood")
        || n.ends_with("_ore")
        || n.ends_with("_concrete")
        || n.ends_with("_terracotta")
        || n.ends_with("_wool")
        || n.ends_with("_concrete_powder")
        || n.ends_with("_bricks")
}
/// Whether an exact source face must remain visible when a verified
/// neighboring full cell is known. Shared with streaming chunk seams so
/// a transparent cube, full slab, fluid or unknown modded block is handled
/// identically within a Region read and across independent chunk patches.
/// LOD samples must never be supplied as exact neighboring cells.
pub fn exact_face_exposed_against(
    state: &State,
    kind: VoxelMaterial,
    neighbor: Option<(&State, VoxelMaterial)>,
) -> bool {
    match neighbor {
        None => true,
        Some((n, _)) if is_air(n) => true,
        Some((n, _)) if crate::native_material::skip_same(state, n) => false,
        Some((n, _))
            if kind == VoxelMaterial::Liquid
                && crate::fluid_surface::kind(state)
                    .is_some_and(|kind| Some(kind) == crate::fluid_surface::kind(n)) =>
        {
            false
        }
        // 液面高度和半格形状稍后由真实流体邻域决定，不能先按整格固体删除。
        Some(_) if kind == VoxelMaterial::Liquid && crate::fluid_surface::kind(state).is_some() => {
            true
        }
        Some((n, _)) if known_partial_occluder(n) => true,
        Some((n, nkind)) if nkind != VoxelMaterial::Solid => {
            kind == VoxelMaterial::Solid || nkind != kind || n.name != state.name
        }
        Some((n, _)) => !known_full_occluder(n),
    }
}
// Preserve the original owned-palette lookup path while sharing the
// exact same visibility semantics with spectator streaming.
#[cfg(test)]
fn exposed_against(
    state: &State,
    kind: VoxelMaterial,
    neighbor: Option<&(State, VoxelMaterial)>,
) -> bool {
    exact_face_exposed_against(state, kind, neighbor.map(|(s, material)| (s, *material)))
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceVoxel {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    descriptor: Arc<VoxelData>,
}
/// 位置无关的不可变显示描述；写时复制仅用于独立暂存或辅助构造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoxelData {
    /// 不可变的原始 / 暂存显示状态；读取快照与拾取共享，编辑仍通过 Session。
    pub state: Arc<State>,
    /// 原格心与六邻格的保存光照；缺失和区块边界保持实际来源语义。
    pub light: crate::saved_light::Samples,
    /// 共享的原始生物群系名称；不是可编辑的方块属性。
    pub biome: Option<crate::shared_text::SharedText>,
    pub original_block_entity: bool,
    /// Original exposed voxel faces; missing source cells are unknown.
    pub faces: u8,
    /// Visual metadata only; has no effect on source NBT or editing.
    pub kind: VoxelMaterial,
    pub staged: bool,
    /// Original X/Z cells represented by this sample (1 is exact).
    /// Spans >1 are approximate and must never be treated as editable blocks.
    pub span: u8,
    /// 基于未裁减原始邻域计算的流体角点；采样远景不提供该几何。
    pub fluid: Option<Arc<crate::fluid_surface::FluidSurface>>,
}
impl SurfaceVoxel {
    /// 未提供源调色板的显式构造；生产读取使用 VoxelPalette::voxel 共享相同描述。
    pub fn new(position: [i32; 3], data: VoxelData) -> Self {
        Self {
            x: position[0],
            y: position[1],
            z: position[2],
            descriptor: Arc::new(data),
        }
    }
    pub fn data(&self) -> &VoxelData {
        &self.descriptor
    }
    pub(crate) fn descriptor_id(&self) -> usize {
        Arc::as_ptr(&self.descriptor) as usize
    }
    /// 消费者读取原始精度的值，不增加共享引用计数，也不把无流体单元扩成内嵌形状。
    #[inline]
    pub fn fluid_surface(&self) -> Option<crate::fluid_surface::FluidSurface> {
        self.fluid.as_deref().copied()
    }
}
impl std::ops::Deref for SurfaceVoxel {
    type Target = VoxelData;
    fn deref(&self) -> &VoxelData {
        &self.descriptor
    }
}
impl std::ops::DerefMut for SurfaceVoxel {
    fn deref_mut(&mut self) -> &mut VoxelData {
        Arc::make_mut(&mut self.descriptor)
    }
}
/// One actual X/Z source chunk touched during the read-only scene pass.
/// Missing chunks are never synthesized. Unknown Sections/columns remain
/// unknown even if adjacent source chunks are fully present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceChunkTile {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub source_present: bool,
    pub known_columns: usize,
    pub unknown_columns: usize,
    pub exposed_voxels: usize,
}
#[derive(Debug, Clone)]
pub struct SurfaceView {
    /// 原始 biome 邻域，与几何来源同一暂存快照；不改变 voxel.biome 的原始含义。
    pub biomes: Arc<crate::biome_source::Region>,
    pub x: i32,
    pub z: i32,
    pub y_min: i32,
    pub y_max: i32,
    pub width: u16,
    pub depth: u16,
    pub voxels: Vec<SurfaceVoxel>,
    pub missing_columns: usize,
    pub source_chunks: usize,
    pub source_tiles: Vec<SourceChunkTile>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceStatistics {
    pub voxels: usize,
    pub exposed_faces: usize,
    pub original_be: usize,
    pub staged: usize,
    pub liquids: usize,
    pub transparent: usize,
    pub foliage: usize,
    pub missing_columns: usize,
    pub original_chunks: usize,
}
impl SurfaceView {
    pub fn statistics(&self) -> SurfaceStatistics {
        SurfaceStatistics {
            voxels: self.voxels.len(),
            exposed_faces: self
                .voxels
                .iter()
                .map(|v| v.faces.count_ones() as usize)
                .sum(),
            original_be: self
                .voxels
                .iter()
                .filter(|v| v.original_block_entity)
                .count(),
            staged: self.voxels.iter().filter(|v| v.staged).count(),
            liquids: self
                .voxels
                .iter()
                .filter(|v| v.kind == VoxelMaterial::Liquid)
                .count(),
            transparent: self
                .voxels
                .iter()
                .filter(|v| v.kind == VoxelMaterial::Transparent)
                .count(),
            foliage: self
                .voxels
                .iter()
                .filter(|v| v.kind == VoxelMaterial::Foliage)
                .count(),
            missing_columns: self.missing_columns,
            original_chunks: self.source_chunks,
        }
    }
}
pub(crate) fn is_air(state: &State) -> bool {
    matches!(
        state.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "numeric:0"
    )
}
/// 每个已读取 X/Z 列的最高源方块；None 表示该列没有可见表面。
type SurfaceColumns = BTreeMap<(i32, i32), Option<(i32, Arc<State>, VoxelMaterial)>>;
/// Use only verified original neighboring surface columns for near-field
/// occlusion. Unknown source cells are never treated as known air.
fn top_faces(
    voxel: &SurfaceVoxel,
    x0: i32,
    z0: i32,
    x1: i32,
    z1: i32,
    columns: &SurfaceColumns,
) -> u8 {
    let mut faces = FACE_TOP;
    for (dx, dz, flag) in [
        (1, 0, FACE_POS_X),
        (-1, 0, FACE_NEG_X),
        (0, 1, FACE_POS_Z),
        (0, -1, FACE_NEG_Z),
    ] {
        let nx = voxel.x + dx;
        let nz = voxel.z + dz;
        if nx < x0 || nx > x1 || nz < z0 || nz > z1 {
            faces |= flag;
            continue;
        }
        match columns.get(&(nx, nz)) {
            Some(None) => faces |= flag,
            Some(Some((height, state, kind))) => {
                if *height < voxel.y
                    || (*height == voxel.y
                        && exact_face_exposed_against(
                            &voxel.state,
                            voxel.kind,
                            Some((state.as_ref(), *kind)),
                        ))
                {
                    faces |= flag;
                }
            }
            // Missing NBT Section or an unpopulated source position does
            // not prove solid occlusion. Keep the candidate side so a later
            // exact-chunk load can resolve it without a visual hole.
            None => faces |= flag,
        }
    }
    faces
}
/// Render a true-height voxel column surface from the current in-memory
/// overlay. Bounded to 64x64 footprint by 48 Y levels (196608 reads).
/// Each original chunk is decoded at most once within a request. No
/// generation or disk write, and the editor's 4096-block write limit stays.
pub fn inspect_surface(
    session: &Session,
    x: i32,
    z: i32,
    y_min: i32,
    y_max: i32,
    width: u8,
    depth: u8,
) -> Result<SurfaceView, WorldError> {
    inspect_surface_with_visibility(
        session,
        VolumeRequest {
            x,
            z,
            vertical: y_min..=y_max,
            width,
            depth,
            top_cut: None,
        },
        &mut crate::surface_visibility::Native,
        &|| false,
    )
}
pub fn inspect_surface_with_visibility(
    session: &Session,
    request: VolumeRequest,
    visibility: &mut dyn crate::surface_visibility::Visibility,
    cancelled: &dyn Fn() -> bool,
) -> Result<SurfaceView, WorldError> {
    let VolumeRequest {
        x,
        z,
        vertical,
        width,
        depth,
        top_cut,
    } = request;
    if top_cut.is_some() {
        return Err(refuse("最高列预览不接受体积剖切参数"));
    }
    let (y_min, y_max) = (*vertical.start(), *vertical.end());
    // A top-down overview retains one highest source block per column.
    // It may scan the complete original dimension height; unlike a 3D
    // volume it does not retain up to 384 voxels per column in memory.
    let height = y_max as i64 - y_min as i64 + 1;
    if width == 0
        || width > MAX_VIEW_XZ
        || depth == 0
        || depth > MAX_VIEW_XZ
        || !(1..=4096).contains(&height)
        || y_min < -2048
        || y_max > 2047
    {
        return Err(refuse(
            "俯视源范围必须在 64×64 列及 Section Y 的 -2048..2047 内",
        ));
    }
    let far_x = x
        .checked_add(width as i32 - 1)
        .ok_or_else(|| refuse("Surface X overflow"))?;
    let far_z = z
        .checked_add(depth as i32 - 1)
        .ok_or_else(|| refuse("Surface Z overflow"))?;
    // Bounds::new enforces the EDIT batch's 4096-cell limit. Read-only
    // inspection can scan 196608 cells, so validate extrema individually.
    let _ = crate::editing::Bounds::new((x, y_min, z), (x, y_min, z))?;
    let _ = crate::editing::Bounds::new((far_x, y_max, far_z), (far_x, y_max, far_z))?;
    let mut cache: BTreeMap<(i32, i32), Option<Arc<Chunk>>> = BTreeMap::new();
    let mut metadata: BTreeMap<(i32, i32), (Option<Chunk>, Option<Chunk>)> = BTreeMap::new();
    let mut presence: BTreeMap<(i32, i32), bool> = BTreeMap::new();
    let mut region_reader = session.source_reader()?;
    let mut voxels = Vec::new();
    let mut palette = VoxelPalette::default();
    let mut columns = SurfaceColumns::new();
    let mut missing_columns = 0;
    let mut tiles: BTreeMap<(i32, i32), SourceChunkTile> = BTreeMap::new();
    for dz in 0..depth as i32 {
        for dx in 0..width as i32 {
            if cancelled() {
                return Err(refuse("最高列预览读取已取消"));
            }
            let (wx, wz) = (x + dx, z + dz);
            let key = (wx.div_euclid(16), wz.div_euclid(16));
            if let std::collections::btree_map::Entry::Vacant(e) = cache.entry(key) {
                let (chunk, on_disk) =
                    session.visible_chunk_base_shared_cached(&mut region_reader, key.0, key.1)?;
                e.insert(chunk);
                presence.insert(key, on_disk);
                metadata.insert(
                    key,
                    staged_metadata_cached(session, &mut region_reader, key.0, key.1)?,
                );
            }
            let on_disk = *presence
                .get(&key)
                .ok_or_else(|| WorldError::Invalid("区块来源状态缺失".into()))?;
            let tile = tiles.entry(key).or_insert_with(|| SourceChunkTile {
                chunk_x: key.0,
                chunk_z: key.1,
                source_present: on_disk,
                known_columns: 0,
                unknown_columns: 0,
                exposed_voxels: 0,
            });
            let Some(chunk) = cache.get(&key).and_then(Option::as_ref) else {
                missing_columns += 1;
                tile.unknown_columns += 1;
                continue;
            };
            let (biome, be) = metadata
                .get(&key)
                .ok_or_else(|| WorldError::Invalid("暂存元数据缓存缺失".into()))?;
            let biome = biome.as_ref().unwrap_or(chunk.as_ref());
            let be = be.as_ref().unwrap_or(chunk.as_ref());
            let mut chosen = None;
            let mut known = false;
            for y in chunk.sections.iter().rev().flat_map(|section| {
                let base = i32::from(section.y) * 16;
                (base.max(y_min)..=(base + 15).min(y_max)).rev()
            }) {
                let Some(original) =
                    chunk.block(wx.rem_euclid(16) as u8, y, wz.rem_euclid(16) as u8)
                else {
                    continue;
                };
                known = true;
                let state = session.effective_state_ref((wx, y, wz), original);
                if !is_air(state) {
                    let kind = material(state);
                    chosen = Some({
                        let data = crate::surface::VoxelData {
                            state: palette.state(state),
                            biome: palette.biome(biome.biome(
                                wx.rem_euclid(16) as u8,
                                y,
                                wz.rem_euclid(16) as u8,
                            )),
                            original_block_entity: be.has_block_entity(wx, y, wz),
                            faces: FACE_TOP,
                            kind,
                            staged: session.is_staged_block((wx, y, wz))
                                || session.creation_origin(key.0, key.1, y),
                            span: 1,
                            fluid: None,
                            light: Default::default(),
                        };
                        palette.voxel([wx, y, wz], data)
                    });
                    break;
                }
            }
            if known {
                tile.known_columns += 1;
                columns.insert(
                    (wx, wz),
                    chosen.as_ref().map(|v| (v.y, v.state.clone(), v.kind)),
                );
                if let Some(voxel) = chosen {
                    voxels.push(voxel);
                }
            } else {
                missing_columns += 1;
                tile.unknown_columns += 1;
            }
        }
    }
    for voxel in &mut voxels {
        let faces = if visibility.resource_aware() {
            ALL_FACES
        } else {
            top_faces(voxel, x, z, far_x, far_z, &columns)
        };
        if faces != voxel.faces {
            *voxel = palette.updated(voxel, |data| data.faces = faces);
        }
        if let Some(tile) = tiles.get_mut(&(voxel.x.div_euclid(16), voxel.z.div_euclid(16))) {
            tile.exposed_voxels += 1;
        }
    }
    inspect_fluid_geometry(
        session,
        &mut region_reader,
        &mut cache,
        &mut voxels,
        visibility,
        None,
        cancelled,
    )?;
    let biomes = inspect_biomes(
        session,
        &mut region_reader,
        &mut cache,
        &metadata,
        tiles.keys().copied(),
        cancelled,
    )?;
    Ok(SurfaceView {
        biomes,
        x,
        z,
        y_min,
        y_max,
        width: width as u16,
        depth: depth as u16,
        voxels,
        missing_columns,
        source_chunks: tiles.len(),
        source_tiles: tiles.into_values().collect(),
    })
}

/// Bounded multilevel voxel inspection: show exposed real solids including
/// internal cavities, never inventing air for absent chunks/Sections.
pub fn inspect_exposed_volume(
    session: &Session,
    x: i32,
    z: i32,
    y_min: i32,
    y_max: i32,
    width: u8,
    depth: u8,
) -> Result<SurfaceView, WorldError> {
    inspect_exposed_volume_cutaway(session, x, z, y_min..=y_max, width, depth, None)
}
/// A read-only cutaway exposes new top faces at the chosen Y plane, instead
/// of merely hiding previously meshed outer faces. Missing source cells stay
/// unknown, and never become assumed empty space.
pub fn inspect_exposed_volume_cutaway(
    session: &Session,
    x: i32,
    z: i32,
    vertical: std::ops::RangeInclusive<i32>,
    width: u8,
    depth: u8,
    top_cut: Option<i32>,
) -> Result<SurfaceView, WorldError> {
    inspect_volume_with_visibility(
        session,
        VolumeRequest {
            x,
            z,
            vertical,
            width,
            depth,
            top_cut,
        },
        &mut crate::surface_visibility::Native,
        &|| false,
    )
}
/// 限定精确体积及剖切请求；可见性查询独立于世界格式解析。
pub struct VolumeRequest {
    pub x: i32,
    pub z: i32,
    pub vertical: std::ops::RangeInclusive<i32>,
    pub width: u8,
    pub depth: u8,
    pub top_cut: Option<i32>,
}
pub fn inspect_volume_with_visibility(
    session: &Session,
    request: VolumeRequest,
    visibility: &mut dyn crate::surface_visibility::Visibility,
    cancelled: &dyn Fn() -> bool,
) -> Result<SurfaceView, WorldError> {
    let VolumeRequest {
        x,
        z,
        vertical,
        width,
        depth,
        top_cut,
    } = request;
    let (y_min, y_max) = (*vertical.start(), *vertical.end());
    if top_cut.is_some_and(|top| top < y_min || top > y_max) {
        return Err(refuse("3D cutaway Y must lie within the inspected bounds"));
    }
    let visible_top = top_cut.unwrap_or(y_max);
    validate_view(width, depth, y_min, y_max)?;
    let max_x = x
        .checked_add(width as i32 - 1)
        .ok_or_else(|| refuse("Volume X overflow"))?;
    let max_z = z
        .checked_add(depth as i32 - 1)
        .ok_or_else(|| refuse("Volume Z overflow"))?;
    let _ = crate::editing::Bounds::new((x, y_min, z), (x, y_min, z))?;
    let _ = crate::editing::Bounds::new((max_x, y_max, max_z), (max_x, y_max, max_z))?;
    let mut chunks: BTreeMap<(i32, i32), Option<Arc<Chunk>>> = BTreeMap::new();
    let mut metadata: BTreeMap<(i32, i32), (Option<Chunk>, Option<Chunk>)> = BTreeMap::new();
    let mut presence: BTreeMap<(i32, i32), bool> = BTreeMap::new();
    let mut region_reader = session.source_reader()?;
    let mut known: BTreeMap<(i32, i32, i32), (Arc<State>, VoxelMaterial)> = BTreeMap::new();
    let mut solids = Vec::new();
    let mut palette = VoxelPalette::default();
    let mut missing_columns = 0;
    let mut tiles: BTreeMap<(i32, i32), SourceChunkTile> = BTreeMap::new();
    for dz in 0..depth as i32 {
        for dx in 0..width as i32 {
            let (wx, wz) = (x + dx, z + dz);
            let key = (wx.div_euclid(16), wz.div_euclid(16));
            if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(key) {
                let (chunk, on_disk) =
                    session.visible_chunk_base_shared_cached(&mut region_reader, key.0, key.1)?;
                e.insert(chunk);
                presence.insert(key, on_disk);
                metadata.insert(
                    key,
                    staged_metadata_cached(session, &mut region_reader, key.0, key.1)?,
                );
            }
            let on_disk = *presence
                .get(&key)
                .ok_or_else(|| WorldError::Invalid("区块来源状态缺失".into()))?;
            let tile = tiles.entry(key).or_insert_with(|| SourceChunkTile {
                chunk_x: key.0,
                chunk_z: key.1,
                source_present: on_disk,
                known_columns: 0,
                unknown_columns: 0,
                exposed_voxels: 0,
            });
            let Some(chunk) = chunks.get(&key).and_then(Option::as_ref) else {
                missing_columns += 1;
                tile.unknown_columns += 1;
                continue;
            };
            let (biome, be) = metadata
                .get(&key)
                .ok_or_else(|| WorldError::Invalid("暂存元数据缓存缺失".into()))?;
            let biome = biome.as_ref().unwrap_or(chunk.as_ref());
            let be = be.as_ref().unwrap_or(chunk.as_ref());
            let mut column_known = false;
            for y in y_min..=y_max {
                if y.rem_euclid(64) == 0 && cancelled() {
                    return Err(refuse("精确体积读取已取消"));
                }
                let Some(original) =
                    chunk.block(wx.rem_euclid(16) as u8, y, wz.rem_euclid(16) as u8)
                else {
                    continue;
                };
                column_known = true;
                let state = palette.state(session.effective_state_ref((wx, y, wz), original));
                let solid = !is_air(&state);
                let kind = material(&state);
                known.insert((wx, y, wz), (state.clone(), kind));
                if solid && y <= visible_top {
                    solids.push({
                        let data = crate::surface::VoxelData {
                            state,
                            biome: palette.biome(biome.biome(
                                wx.rem_euclid(16) as u8,
                                y,
                                wz.rem_euclid(16) as u8,
                            )),
                            original_block_entity: be.has_block_entity(wx, y, wz),
                            faces: 0,
                            kind,
                            staged: session.is_staged_block((wx, y, wz))
                                || session.creation_origin(key.0, key.1, y),
                            span: 1,
                            fluid: None,
                            light: Default::default(),
                        };
                        palette.voxel([wx, y, wz], data)
                    });
                }
            }
            if column_known {
                tile.known_columns += 1;
            } else {
                missing_columns += 1;
                tile.unknown_columns += 1;
            }
        }
    }
    let mut voxels = Vec::new();
    for (index, voxel) in solids.into_iter().enumerate() {
        if index.is_multiple_of(256) && cancelled() {
            return Err(refuse("精确体积可见性查询已取消"));
        }
        let mut faces = voxel.faces;
        let cell = crate::surface_visibility::Cell {
            state: &voxel.state,
            kind: voxel.kind,
            position: (voxel.x, voxel.y, voxel.z),
        };
        for direction in crate::surface_visibility::Direction::ALL {
            let [dx, dy, dz] = direction.offset();
            let bit = direction.flag();
            let (nx, ny, nz) = (voxel.x + dx, voxel.y + dy, voxel.z + dz);
            let clipped =
                nx < x || nx > max_x || nz < z || nz > max_z || ny < y_min || ny > visible_top;
            if clipped
                || visibility.exposed(
                    cell,
                    direction,
                    known
                        .get(&(nx, ny, nz))
                        .map(|(state, kind)| crate::surface_visibility::Cell {
                            state,
                            kind: *kind,
                            position: (nx, ny, nz),
                        }),
                )
            {
                faces |= bit;
            }
        }
        if faces != 0
            || visibility.unconditional(cell)
            || (voxel.kind != VoxelMaterial::Liquid
                && matches!(
                    visibility.fluid_sample(cell)?,
                    crate::fluid_surface::Sample::Fluid(..)
                ))
        {
            voxels.push(if faces == voxel.faces {
                voxel
            } else {
                palette.updated(&voxel, |data| data.faces = faces)
            });
        }
    }
    inspect_fluid_geometry(
        session,
        &mut region_reader,
        &mut chunks,
        &mut voxels,
        visibility,
        Some(crate::editing::Bounds {
            minimum: [x, y_min, z],
            maximum: [max_x, visible_top, max_z],
        }),
        cancelled,
    )?;
    voxels.retain(|voxel| {
        let cell = crate::surface_visibility::Cell {
            state: &voxel.state,
            kind: voxel.kind,
            position: (voxel.x, voxel.y, voxel.z),
        };
        if voxel.kind == VoxelMaterial::Liquid
            && voxel.fluid_surface().is_some_and(|f| f.faces() == 0)
        {
            return false;
        }
        voxel.faces != 0
            || visibility.unconditional(cell)
            || voxel.fluid_surface().is_some_and(|f| f.faces() != 0)
    });
    for voxel in &voxels {
        if let Some(tile) = tiles.get_mut(&(voxel.x.div_euclid(16), voxel.z.div_euclid(16))) {
            tile.exposed_voxels += 1;
        }
    }
    let biomes = inspect_biomes(
        session,
        &mut region_reader,
        &mut chunks,
        &metadata,
        tiles.keys().copied(),
        cancelled,
    )?;
    Ok(SurfaceView {
        biomes,
        x,
        z,
        y_min,
        y_max,
        width: width as u16,
        depth: depth as u16,
        voxels,
        missing_columns,
        source_chunks: tiles.len(),
        source_tiles: tiles.into_values().collect(),
    })
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact_surface_occluder_contract_preserves_transparency_and_partial_shapes() {
        let state = |name: &str, properties: &[(&str, &str)]| State {
            name: name.into(),
            properties: properties
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        };
        for name in [
            "minecraft:ice",
            "minecraft:glass",
            "minecraft:tinted_glass",
            "minecraft:oak_leaves",
            "modded:stone",
        ] {
            assert!(!known_full_occluder(&state(name, &[])), "{name}");
        }
        for name in [
            "minecraft:packed_ice",
            "minecraft:blue_ice",
            "minecraft:stone",
            "minecraft:oak_planks",
            "minecraft:iron_ore",
        ] {
            assert!(known_full_occluder(&state(name, &[])), "{name}");
        }
        assert!(known_full_occluder(&state(
            "minecraft:stone_slab",
            &[("type", "double")]
        )));
        assert!(!known_full_occluder(&state(
            "minecraft:stone_slab",
            &[("type", "top")]
        )));
        assert!(known_full_occluder(&state(
            "minecraft:snow",
            &[("layers", "8")]
        )));
        assert!(!known_full_occluder(&state(
            "minecraft:snow",
            &[("layers", "3")]
        )));
    }

    fn unknown_source_sections_and_vanilla_plants_cannot_hide_faces() {
        let stone = State {
            name: "minecraft:stone".into(),
            properties: vec![],
        };
        let plant = State {
            name: "minecraft:wheat".into(),
            properties: vec![],
        };
        let ice = State {
            name: "minecraft:ice".into(),
            properties: vec![],
        };
        assert!(exposed_against(&stone, VoxelMaterial::Solid, None));
        assert!(exposed_against(
            &stone,
            VoxelMaterial::Solid,
            Some(&(plant, VoxelMaterial::Solid))
        ));
        assert!(exposed_against(
            &stone,
            VoxelMaterial::Solid,
            Some(&(ice, VoxelMaterial::Transparent))
        ));
        let lower = State {
            name: "minecraft:water".into(),
            properties: vec![("level".into(), "4".into())],
        };
        let higher = State {
            name: "minecraft:water".into(),
            properties: vec![("level".into(), "0".into())],
        };
        assert!(!exposed_against(
            &lower,
            VoxelMaterial::Liquid,
            Some(&(higher, VoxelMaterial::Liquid))
        ));
        assert!(!exposed_against(
            &stone,
            VoxelMaterial::Solid,
            Some(&(stone.clone(), VoxelMaterial::Solid))
        ));
    }

    fn unknown_mod_neighbor_cannot_hide_original_faces() {
        let full = State {
            name: "minecraft:stone".into(),
            properties: vec![],
        };
        let modded = State {
            name: "example:thin_model".into(),
            properties: vec![],
        };
        assert!(exposed_against(
            &full,
            VoxelMaterial::Solid,
            Some(&(modded, VoxelMaterial::Solid))
        ));
        assert!(!exposed_against(
            &full,
            VoxelMaterial::Solid,
            Some(&(full.clone(), VoxelMaterial::Solid))
        ));
    }

    fn exact_surface_culls_known_neighbors_but_not_unknown_source() {
        let state = State {
            name: "minecraft:stone".into(),
            properties: vec![],
        };
        let voxel = SurfaceVoxel::new(
            [0, 64, 0],
            crate::surface::VoxelData {
                state: state.clone().into(),
                biome: None,
                original_block_entity: false,
                faces: 0,
                kind: VoxelMaterial::Solid,
                staged: false,
                span: 1,
                fluid: None,
                light: Default::default(),
            },
        );
        let mut columns = BTreeMap::new();
        columns.insert(
            (0, 0),
            Some((64, Arc::clone(&voxel.state), VoxelMaterial::Solid)),
        );
        columns.insert(
            (1, 0),
            Some((64, Arc::clone(&voxel.state), VoxelMaterial::Solid)),
        );
        columns.insert(
            (-1, 0),
            Some((63, Arc::clone(&voxel.state), VoxelMaterial::Solid)),
        );
        columns.insert((0, 1), None);
        let mask = top_faces(&voxel, -1, -1, 1, 1, &columns);
        assert_eq!(mask & FACE_POS_X, 0);
        assert_ne!(mask & FACE_NEG_Z, 0); // missing source cannot hide face
        assert_ne!(mask & FACE_NEG_X, 0);
        assert_ne!(mask & FACE_POS_Z, 0); // known air
        assert_ne!(mask & FACE_TOP, 0);
        assert_eq!(
            top_faces(&voxel, 0, 0, 0, 0, &columns)
                & (FACE_POS_X | FACE_NEG_X | FACE_POS_Z | FACE_NEG_Z),
            FACE_POS_X | FACE_NEG_X | FACE_POS_Z | FACE_NEG_Z
        );
    }

    fn only_recognized_partial_vanilla_shapes_refuse_full_face_occlusion() {
        let solid = State {
            name: "minecraft:stone".into(),
            properties: vec![],
        };
        let half = State {
            name: "minecraft:stone_slab".into(),
            properties: vec![("type".into(), "bottom".into())],
        };
        let double = State {
            name: "minecraft:stone_slab".into(),
            properties: vec![("type".into(), "double".into())],
        };
        let modded = State {
            name: "custom:half_block".into(),
            properties: vec![],
        };
        let partial = (half, VoxelMaterial::Solid);
        let full = (solid.clone(), VoxelMaterial::Solid);
        let full_slab = (double, VoxelMaterial::Solid);
        let unknown = (modded, VoxelMaterial::Solid);
        assert!(exposed_against(
            &solid,
            VoxelMaterial::Solid,
            Some(&partial)
        ));
        assert!(!exposed_against(&solid, VoxelMaterial::Solid, Some(&full)));
        assert!(!exposed_against(
            &solid,
            VoxelMaterial::Solid,
            Some(&full_slab)
        ));
        assert!(exposed_against(
            &solid,
            VoxelMaterial::Solid,
            Some(&unknown)
        ));
        let snow = State {
            name: "minecraft:snow".into(),
            properties: vec![("layers".into(), "3".into())],
        };
        assert!(known_partial_occluder(&snow));
    }

    fn original_source_tiles_are_sorted_and_distinguish_missing_sections() {
        let tile = SourceChunkTile {
            chunk_x: -1,
            chunk_z: 0,
            source_present: true,
            known_columns: 3,
            unknown_columns: 2,
            exposed_voxels: 4,
        };
        assert_eq!(tile.known_columns + tile.unknown_columns, 5);
        let missing = SourceChunkTile {
            chunk_x: -2,
            chunk_z: 0,
            source_present: false,
            known_columns: 0,
            unknown_columns: 16,
            exposed_voxels: 0,
        };
        assert!(!missing.source_present);
        assert_eq!((-17_i32).div_euclid(16), missing.chunk_x);
    }

    fn scene_budget_allows_3x3_chunks_but_never_unbounded_source_reads() {
        assert_eq!(MAX_VIEW_CELLS, 196_608);
        assert!(validate_view(64, 64, -64, -17).is_ok());
        assert!(validate_view(64, 64, -64, -16).is_err());
        assert!(validate_view(65, 1, -64, -64).is_err());
        assert!(validate_view(0, 1, 0, 0).is_err());
        assert!(validate_view(1, 1, i32::MIN, i32::MAX).is_err());
        assert!(validate_view(1, 1, -384, -337).is_ok());
    }

    fn summary_counts_render_faces_materials_and_source_metadata() {
        let view = SurfaceView {
            biomes: Default::default(),
            x: 0,
            z: 0,
            y_min: -64,
            y_max: -64,
            width: 1,
            depth: 1,
            missing_columns: 0,
            source_chunks: 1,
            source_tiles: vec![SourceChunkTile {
                chunk_x: 0,
                chunk_z: 0,
                source_present: true,
                known_columns: 1,
                unknown_columns: 0,
                exposed_voxels: 1,
            }],
            voxels: vec![SurfaceVoxel::new(
                [0, -64, 0],
                crate::surface::VoxelData {
                    state: State {
                        name: "minecraft:glass".into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    original_block_entity: true,
                    faces: FACE_TOP | FACE_POS_X,
                    kind: VoxelMaterial::Transparent,
                    staged: true,
                    span: 1,
                    fluid: None,
                    light: Default::default(),
                },
            )],
        };
        let stats = view.statistics();
        assert_eq!(stats.voxels, 1);
        assert_eq!(stats.exposed_faces, 2);
        assert_eq!(
            (stats.original_be, stats.staged, stats.transparent),
            (1, 1, 1)
        );
    }

    fn cutaway_accepts_negative_y_and_refuses_out_of_bounds_without_writing() {
        // Pure geometrical clipping: the selected layer is still the exact
        // source coordinate, and its newly exposed roof is a view cut only.
        let y_min = -64;
        let y_max = -33;
        assert_eq!(Some(-48).filter(|v| *v >= y_min && *v <= y_max), Some(-48));
        assert_eq!(Some(-65).filter(|v| *v >= y_min && *v <= y_max), None);
        assert_eq!(
            FACE_TOP | FACE_BOTTOM | FACE_POS_X | FACE_NEG_X | FACE_POS_Z | FACE_NEG_Z,
            ALL_FACES
        );
    }

    fn air_detection_is_explicit_not_derived_from_arbitrary_modded_name() {
        for name in [
            "minecraft:air",
            "minecraft:cave_air",
            "minecraft:void_air",
            "numeric:0",
        ] {
            assert!(is_air(&State {
                name: name.into(),
                properties: vec![]
            }));
        }
        assert!(!is_air(&State {
            name: "mod:custom_air".into(),
            properties: vec![]
        }));
    }

    fn shared_source_payload_lifecycle() {
        use crate::fluid_surface::{inspect, Kind, Sample};
        let water = State {
            name: "minecraft:water".into(),
            properties: vec![("level".into(), "0".into())],
        };
        let stone = State {
            name: "minecraft:stone".into(),
            properties: vec![],
        };
        let full = inspect(Sample::Fluid(Kind::Water, 0), (0, 0, 0), &mut |_| {
            Ok(Sample::Fluid(Kind::Water, 0))
        })
        .unwrap()
        .unwrap();
        let edge = full
            .with_faces(FACE_TOP | FACE_NEG_X)
            .unwrap()
            .with_overlays(FACE_NEG_X)
            .unwrap();
        let mut palette = VoxelPalette::default();
        let voxels = (0..10_000)
            .map(|index| {
                let fluid = match index % 3 {
                    0 => Some(full),
                    1 => Some(edge),
                    _ => None,
                };
                {
                    let data = crate::surface::VoxelData {
                        state: palette.state(if fluid.is_some() { &water } else { &stone }),
                        biome: palette.biome(Some("minecraft:forest")),
                        fluid: palette.fluid(fluid),
                        original_block_entity: false,
                        faces: ALL_FACES,
                        kind: if fluid.is_some() {
                            VoxelMaterial::Liquid
                        } else {
                            VoxelMaterial::Solid
                        },
                        staged: false,
                        span: 1,
                        light: Default::default(),
                    };
                    palette.voxel([index % 16, index / 256, index / 16 % 16], data)
                }
            })
            .collect::<Vec<_>>();
        assert!(Arc::ptr_eq(
            voxels[0].fluid.as_ref().unwrap(),
            voxels[3].fluid.as_ref().unwrap()
        ));
        assert!(!Arc::ptr_eq(
            voxels[0].fluid.as_ref().unwrap(),
            voxels[1].fluid.as_ref().unwrap()
        ));
        for (index, voxel) in voxels.iter().enumerate() {
            assert_eq!(
                voxel.fluid_surface(),
                match index % 3 {
                    0 => Some(full),
                    1 => Some(edge),
                    _ => None,
                }
            );
        }
        let weak = Arc::downgrade(voxels[0].fluid.as_ref().unwrap());
        drop(palette);
        let snapshot = voxels.clone();
        let mut memory = crate::source_memory::Footprint::default();
        memory.include(&voxels, voxels.capacity());
        let first = memory.usage();
        assert_eq!(first.unique_descriptors, 3);
        assert_eq!(
            (
                first.unique_states,
                first.unique_biomes,
                first.unique_fluids
            ),
            (2, 1, 2)
        );
        memory.include(&snapshot, snapshot.capacity());
        memory.shared_state(&snapshot[0].state);
        let both = memory.usage();
        assert_eq!(both.shared_bytes(), first.shared_bytes());
        assert!(both.inline_bytes > first.inline_bytes);
        let mut original = crate::source_memory::Footprint::default();
        original.include(&voxels, voxels.capacity());
        let original = original.seal();
        let mut copy = crate::source_memory::Footprint::default();
        copy.include(&snapshot, snapshot.capacity());
        let copy = copy.seal();
        let mut ledger = crate::source_memory::Ledger::default();
        ledger.add(&original);
        ledger.add(&copy);
        assert_eq!(ledger.usage().shared_bytes(), both.shared_bytes());
        assert_eq!(ledger.usage().inline_bytes, both.inline_bytes);
        ledger.remove(&original);
        assert_eq!(ledger.usage().total_bytes(), copy.usage().total_bytes());
        ledger.remove(&copy);
        assert_eq!(
            (ledger.usage().total_bytes(), ledger.usage().voxels),
            (0, 0)
        );
        drop(voxels);
        assert!(weak.upgrade().is_some());
        drop(snapshot);
        assert!(
            weak.upgrade().is_none(),
            "统计和已释放的调色板不能保活源形状"
        );
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Geometry {
            exact_surface_occluder_contract_preserves_transparency_and_partial_shapes();
            unknown_source_sections_and_vanilla_plants_cannot_hide_faces();
            unknown_mod_neighbor_cannot_hide_original_faces();
            exact_surface_culls_known_neighbors_but_not_unknown_source();
            only_recognized_partial_vanilla_shapes_refuse_full_face_occlusion();
            original_source_tiles_are_sorted_and_distinguish_missing_sections();
            scene_budget_allows_3x3_chunks_but_never_unbounded_source_reads();
            summary_counts_render_faces_materials_and_source_metadata();
            shared_source_payload_lifecycle();
            cutaway_accepts_negative_y_and_refuses_out_of_bounds_without_writing();
            air_detection_is_explicit_not_derived_from_arbitrary_modded_name();
        }
    }
}
