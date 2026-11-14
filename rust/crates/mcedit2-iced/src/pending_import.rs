//! 世界放置的操作元数据，以及独立结构文档和工具光标共用的几何构建。
use crate::{
    picking::{Group, Index},
    spectator::{self, SpectatorScene, StreamMesh},
};
#[cfg(test)]
use mcedit2_world::editing::Session;
use mcedit2_world::{
    clipboard::BlockClipboard,
    surface::{self, SurfaceView},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
type PreviewGroups = BTreeMap<
    (i32, i32),
    (
        Vec<crate::world_mesh::WorldQuad>,
        Vec<Group>,
        Vec<Option<mcedit2_world::structure_document::Object>>,
    ),
>;
/// 追加同一区块的预览源面；保留逐源拾取组，避免每个 BE / 实体单独创建绘制批次。
fn append_preview_geometry(
    groups: &mut PreviewGroups,
    origin: [i32; 3],
    delta: [f32; 3],
    quads: Vec<crate::world_mesh::WorldQuad>,
    object: Option<mcedit2_world::structure_document::Object>,
    owner: Option<crate::picking::Owner>,
) -> Result<(), String> {
    if !delta.into_iter().all(f32::is_finite)
        || origin[1] != 0
        || origin[0].rem_euclid(16) != 0
        || origin[2].rem_euclid(16) != 0
    {
        return Err("预览源变换或区块原点无效".into());
    }
    if quads.is_empty() {
        return Ok(());
    }
    let (faces, owners, objects) = groups.entry((origin[0] / 16, origin[2] / 16)).or_default();
    let start = faces.len();
    faces.extend(quads.into_iter().map(|quad| quad.translated(delta)));
    owners.push(Group {
        quads: start..faces.len(),
        owner,
    });
    objects.push(object);
    Ok(())
}
#[derive(Default)]
pub(crate) struct State {
    pub running: bool,
    pub frame: Option<Arc<Placement>>,
    pub clipboard: Option<Arc<BlockClipboard>>,
    pub epoch: u64,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
}
/// 排队的预览重建属于发出请求时的世界和代次；取消或新参数会撤销它。
#[derive(Debug, Clone)]
pub(crate) struct Reload {
    key: Option<(usize, usize)>,
    epoch: u64,
}
impl Reload {
    pub(crate) fn matches(&self, app: &crate::App) -> bool {
        self.key == app.current_key() && self.epoch == app.pending_import.epoch
    }
}
pub(crate) fn reload(app: &crate::App) -> iced::Task<crate::Message> {
    iced::Task::done(crate::Message::PendingClipboardReload(Reload {
        key: app.current_key(),
        epoch: app.pending_import.epoch,
    }))
}
/// 世界放置的几何来自完整候选 Session；这里只保留操作框及其资源归属。
#[derive(Debug)]
pub(crate) struct Placement {
    pub anchor: [i32; 3],
    pub size: [u16; 3],
    pub diagnostics: Vec<String>,
    pub show_textures: bool,
    pub online_skins: bool,
    pub language: crate::language::Language,
    atlas: Option<Arc<crate::textures::TextureAtlas>>,
}
impl Placement {
    pub fn new(
        clip: &BlockClipboard,
        anchor: [i32; 3],
        show_textures: bool,
        online_skins: bool,
        language: crate::language::Language,
        atlas: Option<Arc<crate::textures::TextureAtlas>>,
        diagnostics: Vec<String>,
    ) -> Result<Self, String> {
        let anchor =
            std::array::from_fn::<_, 3, _>(|axis| anchor[axis].checked_add(clip.offset[axis]));
        let [Some(x), Some(y), Some(z)] = anchor else {
            return Err("放置外框坐标溢出".into());
        };
        Ok(Self {
            anchor: [x, y, z],
            size: clip.size,
            diagnostics,
            show_textures,
            online_skins,
            language,
            atlas,
        })
    }
    pub fn compatible_with(&self, atlas: Option<&Arc<crate::textures::TextureAtlas>>) -> bool {
        match (atlas, self.atlas.as_ref()) {
            (Some(current), Some(preview)) => {
                crate::textures::atlas_compatible(preview, current)
                    || crate::textures::atlas_compatible(current, preview)
            }
            (None, None) => true,
            _ => false,
        }
    }
}
impl State {
    pub fn invalidate(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.running = false;
    }
    pub fn begin(&mut self) {
        self.invalidate();
        self.running = true;
    }
    pub fn finish(&mut self, epoch: u64) -> bool {
        if self.epoch != epoch || !self.running {
            return false;
        }
        self.running = false;
        true
    }
    /// 只撤回真实在途或已显示的视觉结果；保留冻结来源供用户重新预览。
    pub fn resources_changed(&mut self) -> bool {
        if !self.running && self.frame.is_none() {
            return false;
        }
        self.invalidate();
        self.frame = None;
        true
    }
    pub fn cancel(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.running = false;
        self.frame = None;
        self.clipboard = None;
    }
    pub fn bounds(&self) -> Option<mcedit2_world::editing::Bounds> {
        let frame = self.frame.as_ref()?;
        let mut max = frame.anchor;
        for (axis, value) in max.iter_mut().enumerate() {
            *value = value.checked_add(i32::from(frame.size[axis]) - 1)?;
        }
        Some(mcedit2_world::editing::Bounds {
            minimum: frame.anchor,
            maximum: max,
        })
    }
}
#[derive(Debug)]
pub(crate) struct Frame {
    pub mesh: Arc<StreamMesh>,
    pub anchor: [i32; 3],
    pub size: [u16; 3],
    pub diagnostics: Vec<String>,
    pub show_textures: bool,
    pub online_skins: bool,
    pub language: crate::language::Language,
    pub objects: Vec<ObjectPart>,
}
#[derive(Debug)]
pub(crate) struct ObjectPart {
    pub picking: Arc<Index>,
    pub groups: Vec<Option<mcedit2_world::structure_document::Object>>,
}
/// 装备准备和身体提交使用同一来源解释，不能先用无上下文的 Rig 准备物品。
fn entity_context(
    id: Option<&str>,
    visual: &mut Result<mcedit2_world::entity_catalog::EntityVisual, String>,
    bank: &crate::textures::TextureBank,
    environment: Option<&crate::item_world::Environment>,
) {
    let Some(id) = id.filter(|id| {
        mcedit2_world::entity_zombification::supports(id) && !bank.has_declarative_entity(id)
    }) else {
        return;
    };
    let result = visual
        .as_mut()
        .map_err(|error| error.clone())
        .and_then(|visual| {
            let version = visual.source_version.ok_or("转化实体预览缺少实际源版本")?;
            if id == "minecraft:piglin" {
                let loved = match mcedit2_world::piglin_appearance::offhand(visual)? {
                    Some(item) => {
                        let tags = if let Some(environment) = environment {
                            environment.data.piglin_tags(version)?
                        } else {
                            bank.piglin_tags(version)?
                        };
                        tags.loves(&item)
                    }
                    None => false,
                };
                mcedit2_world::piglin_appearance::set(visual, loved);
            }
            Ok(())
        });
    if let Err(error) = result {
        *visual = Err(error);
    }
}
pub(crate) fn air(state: &mcedit2_world::sections::State) -> bool {
    matches!(
        state.name.as_str(),
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "numeric:0"
    )
}
#[cfg(test)]
pub(crate) fn build(
    session: &Session,
    clip: &BlockClipboard,
    anchor: [i32; 3],
    scene: &SpectatorScene,
    online_skins: bool,
    language: crate::language::Language,
    cancelled: &dyn Fn() -> bool,
) -> Result<Frame, String> {
    build_with_budget(
        session,
        clip,
        anchor,
        scene,
        online_skins,
        language,
        mcedit2_world::editing::EditWorkBudget::default(),
        cancelled,
    )
}
#[cfg(test)]
pub(crate) fn build_with_budget(
    session: &Session,
    clip: &BlockClipboard,
    anchor: [i32; 3],
    scene: &SpectatorScene,
    online_skins: bool,
    language: crate::language::Language,
    budget: mcedit2_world::editing::EditWorkBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<Frame, String> {
    clip.validate_with_budget(session.original_world(), session.dimension(), budget)
        .map_err(|error| error.to_string())?;
    build_geometry_with_budget(
        &Volume::from_clipboard(clip, session.original_world(), session.dimension()),
        anchor,
        scene,
        online_skins,
        language,
        budget,
        cancelled,
    )
}

/// 几何输入不声明世界所有权；结构和工具光标均明确提供实际材质与附加载荷。
pub(crate) struct Volume<'a> {
    pub size: [u16; 3],
    pub offset: [i32; 3],
    pub blocks: &'a [mcedit2_world::sections::State],
    pub mask: Option<&'a [bool]>,
    pub block_entities: &'a [mcedit2_world::clipboard::OpaqueBlockEntity],
    pub entities: &'a [mcedit2_world::clipboard::OpaqueEntity],
    pub biomes: Option<&'a mcedit2_world::clipboard::BiomeVolume>,
    pub numeric_biomes: Option<&'a mcedit2_world::numeric_biome::Volume>,
    pub data_version: Option<i32>,
    pub painting_variants: &'a BTreeMap<String, mcedit2_world::painting::Variant>,
    pub metadata: bool,
    pub map_world: Option<&'a mcedit2_world::WorldSummary>,
    pub item_environment: Option<crate::item_world::Environment>,
}
impl<'a> Volume<'a> {
    #[cfg(test)]
    fn from_clipboard(
        clip: &'a BlockClipboard,
        world: &'a mcedit2_world::WorldSummary,
        dimension: &str,
    ) -> Self {
        Self {
            size: clip.size,
            offset: clip.offset,
            blocks: &clip.blocks,
            mask: clip.mask.as_deref(),
            block_entities: &clip.block_entities,
            entities: &clip.entities,
            biomes: clip.biomes.as_ref(),
            numeric_biomes: clip.numeric_biomes.as_ref(),
            data_version: clip.data_version,
            painting_variants: &clip.painting_variants,
            metadata: clip.metadata.is_some(),
            map_world: Some(world),
            item_environment: Some(crate::item_world::Environment::new(world, dimension)),
        }
    }
}
pub(crate) fn build_geometry_with_budget(
    clip: &Volume<'_>,
    anchor: [i32; 3],
    scene: &SpectatorScene,
    online_skins: bool,
    language: crate::language::Language,
    budget: mcedit2_world::editing::EditWorkBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<Frame, String> {
    build_geometry(
        clip,
        anchor,
        scene,
        online_skins,
        language,
        budget,
        cancelled,
        false,
    )
}
/// 独立结构的归属只指向局部文档，不赋予世界 Session 或磁盘方块所有权。
pub(crate) fn build_document_geometry_with_budget(
    clip: &Volume<'_>,
    anchor: [i32; 3],
    scene: &SpectatorScene,
    online_skins: bool,
    language: crate::language::Language,
    budget: mcedit2_world::editing::EditWorkBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<Frame, String> {
    build_geometry(
        clip,
        anchor,
        scene,
        online_skins,
        language,
        budget,
        cancelled,
        true,
    )
}
fn build_geometry(
    clip: &Volume<'_>,
    anchor: [i32; 3],
    scene: &SpectatorScene,
    online_skins: bool,
    language: crate::language::Language,
    budget: mcedit2_world::editing::EditWorkBudget,
    cancelled: &dyn Fn() -> bool,
    document: bool,
) -> Result<Frame, String> {
    let mut diagnostics = Vec::new();
    // 待放置对象是独立预览，世界地形显隐不裁掉其候选方块或邻面。
    let mut preview_scene = scene.clone();
    preview_scene.block_layers = Default::default();
    preview_scene.texture_bank = scene.texture_bank.as_ref().map(|bank| {
        if bank.block_layers() == Default::default() {
            Arc::clone(bank)
        } else {
            Arc::new((**bank).clone().with_block_layers(Default::default()))
        }
    });
    let scene = &preview_scene;
    if clip.metadata {
        diagnostics.push("结构文档元数据不参与场景几何".into());
    }
    let origin: [Option<i32>; 3] =
        std::array::from_fn(|axis| anchor[axis].checked_add(clip.offset[axis]));
    let [Some(x0), Some(y0), Some(z0)] = origin else {
        return Err("结构放置原点溢出".into());
    };
    let [sx, sy, sz] = clip.size.map(usize::from);
    if sx == 0
        || sy == 0
        || sz == 0
        || sx.checked_mul(sy).and_then(|n| n.checked_mul(sz)) != Some(clip.blocks.len())
    {
        return Err("结构方块数量无效".into());
    }
    if clip
        .mask
        .is_some_and(|mask| mask.len() != clip.blocks.len())
    {
        return Err("几何选择掩码长度无效".into());
    }
    let x1 = x0.checked_add(sx as i32 - 1).ok_or("结构 X 越界")?;
    let y1 = y0.checked_add(sy as i32 - 1).ok_or("结构 Y 越界")?;
    let z1 = z0.checked_add(sz as i32 - 1).ok_or("结构 Z 越界")?;
    let _ = mcedit2_world::editing::Bounds::new((x0, y0, z0), (x0, y0, z0))
        .map_err(|e| e.to_string())?;
    let _ = mcedit2_world::editing::Bounds::new((x1, y1, z1), (x1, y1, z1))
        .map_err(|e| e.to_string())?;
    let biome = if let Some(b) = &clip.biomes {
        if b.resolution != 1 || b.size != clip.size || b.cells.len() != clip.blocks.len() {
            return Err("待放置 biome 布局不受支持".into());
        }
        Some(&b.cells)
    } else {
        None
    };
    let mut palette = surface::VoxelPalette::default();
    let mut voxels = Vec::new();
    use mcedit2_world::surface_visibility::{Cell, Direction, Visibility};
    let mut visibility =
        crate::source_visibility::Reader::new(scene.texture_bank.clone(), scene.show_textures);
    let block_entities = clip
        .block_entities
        .iter()
        .map(|entry| (entry.position, entry))
        .collect::<HashMap<_, _>>();
    let document_objects = if document {
        clip.block_entities
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                (
                    std::array::from_fn(|axis| [x0, y0, z0][axis] + entry.position[axis]),
                    mcedit2_world::structure_document::Object::BlockEntity(index),
                )
            })
            .collect::<HashMap<[i32; 3], _>>()
    } else {
        HashMap::new()
    };
    let mut block_visuals = Vec::new();
    let mut declarative_visuals = Vec::new();
    let index = |x: usize, y: usize, z: usize| (y * sz + z) * sx + x;
    let block_owner = |position: [i32; 3]| {
        document.then(|| crate::picking::Owner {
            position,
            state: Arc::new(
                clip.blocks[index(
                    (position[0] - x0) as usize,
                    (position[1] - y0) as usize,
                    (position[2] - z0) as usize,
                )]
                .clone(),
            ),
        })
    };
    for y in 0..sy {
        if cancelled() {
            return Err("待放置预览已取消".into());
        }
        for z in 0..sz {
            for x in 0..sx {
                let i = index(x, y, z);
                if clip.mask.as_ref().is_some_and(|m| !m[i]) || air(&clip.blocks[i]) {
                    continue;
                }
                let state = &clip.blocks[i];
                let mut replaces = false;
                if let Some(bank) = scene.texture_bank.as_ref() {
                    if let Some(policy) = bank.declarative_block_replaces(state) {
                        replaces = policy;
                        let source = block_entities
                            .get(&[x as i32, y as i32, z as i32])
                            .ok_or_else(|| "声明式方块外观缺少源 BE NBT".to_owned())
                            .and_then(|entry| {
                                mcedit2_world::entity_catalog::EntityVisual::parse_block_entity(
                                    &entry.compound,
                                    clip.data_version,
                                )
                            });
                        declarative_visuals.push((
                            [x0 + x as i32, y0 + y as i32, z0 + z as i32],
                            state.clone(),
                            source,
                        ));
                    }
                }
                if let Some(bank) = scene.texture_bank.as_ref().filter(|bank| {
                    !replaces
                        && (bank.vanilla_banner_supported(state)
                            || bank.vanilla_skull_supported(state)
                            || bank.vanilla_sign_supported(state)
                            || bank.vanilla_piston_supported(state))
                }) {
                    let relative = [x as i32, y as i32, z as i32];
                    let source = block_entities
                        .get(&relative)
                        .ok_or_else(|| "特殊方块缺少对应方块实体 NBT".to_owned())
                        .and_then(|entry| {
                            if bank.vanilla_banner_supported(state) {
                                mcedit2_world::banner::parse(
                                    &entry.compound,
                                    state,
                                    clip.data_version,
                                )
                                .map(mcedit2_world::scene_annotations::BlockVisual::Banner)
                            } else if bank.vanilla_skull_supported(state) {
                                mcedit2_world::skull::parse(
                                    &entry.compound,
                                    state,
                                    clip.data_version,
                                )
                                .map(mcedit2_world::scene_annotations::BlockVisual::Skull)
                            } else if bank.vanilla_piston_supported(state) {
                                mcedit2_world::piston::parse(
                                    &entry.compound,
                                    state,
                                    clip.data_version,
                                )
                                .map(mcedit2_world::scene_annotations::BlockVisual::Piston)
                            } else {
                                mcedit2_world::sign::parse(
                                    &entry.compound,
                                    state,
                                    clip.data_version,
                                )
                                .map(mcedit2_world::scene_annotations::BlockVisual::Sign)
                            }
                        });
                    block_visuals.push(([x0 + x as i32, y0 + y as i32, z0 + z as i32], source));
                }
                let kind = surface::material(state);
                let cell = Cell {
                    state,
                    kind,
                    position: (x0 + x as i32, y0 + y as i32, z0 + z as i32),
                };
                let mut faces = 0;
                for direction in Direction::ALL {
                    let [dx, dy, dz] = direction.offset();
                    let flag = direction.flag();
                    let (nx, ny, nz) = (
                        x as i64 + i64::from(dx),
                        y as i64 + i64::from(dy),
                        z as i64 + i64::from(dz),
                    );
                    let neighbor = if nx >= 0
                        && ny >= 0
                        && nz >= 0
                        && nx < sx as i64
                        && ny < sy as i64
                        && nz < sz as i64
                    {
                        let j = index(nx as usize, ny as usize, nz as usize);
                        if clip.mask.as_ref().is_some_and(|m| !m[j]) {
                            None
                        } else {
                            Some(Cell {
                                state: &clip.blocks[j],
                                kind: surface::material(&clip.blocks[j]),
                                position: (x0 + nx as i32, y0 + ny as i32, z0 + nz as i32),
                            })
                        }
                    } else {
                        None
                    };
                    if visibility.exposed(cell, direction, neighbor) {
                        faces |= flag;
                    }
                }
                let unconditional = visibility.unconditional(cell);
                if faces == 0
                    && !unconditional
                    && (kind == surface::VoxelMaterial::Liquid
                        || !matches!(
                            visibility
                                .fluid_sample(cell)
                                .map_err(|error| error.to_string())?,
                            mcedit2_world::fluid_surface::Sample::Fluid(..)
                        ))
                {
                    continue;
                }
                let fluid = mcedit2_world::surface_visibility::fluid_surface(
                    &mut visibility,
                    cell,
                    surface::ALL_FACES,
                    Some(mcedit2_world::editing::Bounds {
                        minimum: [x0, y0, z0],
                        maximum: [x1, y1, z1],
                    }),
                    &mut |(wx, wy, wz)| {
                        let (nx, ny, nz) = (
                            i64::from(wx) - i64::from(x0),
                            i64::from(wy) - i64::from(y0),
                            i64::from(wz) - i64::from(z0),
                        );
                        if nx < 0
                            || ny < 0
                            || nz < 0
                            || nx >= sx as i64
                            || ny >= sy as i64
                            || nz >= sz as i64
                        {
                            return None;
                        }
                        let j = index(nx as usize, ny as usize, nz as usize);
                        if clip.mask.as_ref().is_some_and(|mask| !mask[j]) {
                            return None;
                        }
                        Some(Cell {
                            state: &clip.blocks[j],
                            kind: surface::material(&clip.blocks[j]),
                            position: (wx, wy, wz),
                        })
                    },
                )
                .map_err(|error| error.to_string())?;
                if fluid.is_some_and(|fluid| fluid.faces() == 0)
                    && (kind == surface::VoxelMaterial::Liquid || (faces == 0 && !unconditional))
                {
                    continue;
                }
                let shared = palette.state(state);
                let biome = palette.biome(biome.map(|v| v[i].as_str()));
                voxels.push({
                    let data = mcedit2_world::surface::VoxelData {
                        state: shared,
                        biome,
                        original_block_entity: block_entities
                            .contains_key(&[x as i32, y as i32, z as i32]),
                        faces,
                        kind,
                        staged: true,
                        span: 1,
                        fluid: palette.fluid(fluid),
                        light: Default::default(),
                    };
                    palette.voxel([x0 + x as i32, y0 + y as i32, z0 + z as i32], data)
                });
            }
        }
    }
    // 按实际可见源体素及资源模型的面数估算新网格，在网格发射前拒绝超预算几何。
    let input_bytes = clip.blocks.iter().try_fold(0usize, |total, state| {
        let bytes = std::mem::size_of::<mcedit2_world::sections::State>()
            + state.name.len()
            + state
                .properties
                .iter()
                .map(|(a, b)| std::mem::size_of::<(String, String)>() + a.len() + b.len())
                .sum::<usize>();
        total.checked_add(bytes).ok_or("预览输入工作集溢出")
    })?;
    let mut work_bytes = input_bytes
        .checked_mul(2)
        .and_then(|n| {
            n.checked_add(voxels.capacity() * std::mem::size_of::<surface::SurfaceVoxel>())
        })
        .ok_or("预览工作集溢出")?;
    budget.check(work_bytes).map_err(|e| e.to_string())?;
    for voxel in &voxels {
        if cancelled() {
            return Err("预览几何已取消".into());
        }
        let pos = (voxel.x, voxel.y, voxel.z);
        let objects = scene
            .texture_bank
            .as_ref()
            .and_then(|bank| bank.static_obj_at(&voxel.state, pos))
            .map_or(0, |triangles| triangles.len());
        let parts = scene
            .texture_bank
            .as_ref()
            .and_then(|bank| bank.geometry_at(&voxel.state, pos))
            .map_or_else(
                || crate::models::boxes(&voxel.state).len(),
                |parts| parts.len(),
            );
        let faces = parts
            .checked_mul(6)
            .and_then(|n| n.checked_add(objects + 6))
            .ok_or("预览模型面数溢出")?;
        let bytes = faces
            .checked_mul(
                std::mem::size_of::<crate::world_mesh::WorldQuad>() * 3
                    + std::mem::size_of::<Group>()
                    + 96,
            )
            .ok_or("预览几何工作集溢出")?;
        work_bytes = work_bytes.checked_add(bytes).ok_or("预览几何工作集溢出")?;
        budget.check(work_bytes).map_err(|e| e.to_string())?;
    }
    let mut color_source = scene.snapshot.as_ref().map_or_else(
        || (*scene.surface.biomes).clone(),
        |snapshot| snapshot.biomes(),
    );
    if let Some(cells) = biome {
        color_source = color_source
            .with_block_biomes_with_budget([x0, y0, z0], clip.size, cells, budget)
            .map_err(|error| error.to_string())?;
    }
    if let Some(volume) = &clip.numeric_biomes {
        color_source = color_source
            .with_numeric_biomes([x0, y0, z0], volume)
            .map_err(|error| error.to_string())?;
    }
    let view = SurfaceView {
        biomes: Arc::new(color_source),
        x: x0,
        z: z0,
        y_min: y0,
        y_max: y1,
        width: clip.size[0],
        depth: clip.size[2],
        voxels,
        missing_columns: 0,
        source_chunks: 0,
        source_tiles: Vec::new(),
    };
    let mut synthetic = scene.clone();
    // 预览有独立的 biome 覆盖与来源，不得清空正在显示的大范围地形网格缓存。
    synthetic.chunk_cache = Default::default();
    synthetic.surface = Arc::new(view);
    synthetic.snapshot = None;
    let mut prepared_visuals = Vec::new();
    let mut prepared_frames = BTreeMap::new();
    let mut prepared_names = BTreeMap::new();
    let mut prepared_equipment = BTreeMap::new();
    let mut sign_materials = BTreeMap::new();
    let mut banner_materials = BTreeMap::new();
    let mut head_results = BTreeMap::new();
    if let Some(bank) = &scene.texture_bank {
        let mut fonts = crate::font_glyphs::Fonts::new(bank, language);
        let base = bank.atlas().ok_or("待放置预览缺少已完成图集")?;
        let atlas = scene
            .entity_frame
            .as_ref()
            .filter(|frame| {
                frame.show_textures == scene.show_textures
                    && frame.show_entities == scene.entity_enabled
                    && crate::textures::atlas_compatible(&frame.atlas, base)
            })
            .map_or_else(|| Arc::clone(base), |frame| Arc::clone(&frame.atlas));
        let mut textures = BTreeMap::new();
        for (position, source) in block_visuals {
            if cancelled() {
                return Err("待放置方块实体准备已取消".into());
            }
            let prepared = source.and_then(|source| match source {
                mcedit2_world::scene_annotations::BlockVisual::Piston(source) => Ok((
                    mcedit2_world::scene_annotations::BlockVisual::Piston(source),
                    None,
                )),
                mcedit2_world::scene_annotations::BlockVisual::Sign(source) => {
                    let prepared = if scene.show_textures {
                        crate::sign_geometry::prepare(
                            &source, bank, &mut fonts, language, cancelled,
                        )?
                    } else {
                        Arc::new(crate::sign_geometry::Prepared {
                            sides: [None, None],
                        })
                    };
                    for texture in prepared.textures() {
                        if !atlas.entries.contains_key(&texture.id) {
                            textures
                                .entry(texture.id.clone())
                                .or_insert_with(|| Arc::clone(texture));
                        }
                    }
                    sign_materials.insert(position, prepared);
                    Ok((
                        mcedit2_world::scene_annotations::BlockVisual::Sign(source),
                        None,
                    ))
                }
                mcedit2_world::scene_annotations::BlockVisual::Banner(source) => {
                    let material = crate::banner_geometry::prepare(&source, bank, cancelled)?;
                    for texture in material.textures() {
                        if !atlas.entries.contains_key(&texture.id) {
                            textures
                                .entry(texture.id.clone())
                                .or_insert_with(|| Arc::clone(texture));
                        }
                    }
                    banner_materials.insert(position, material);
                    Ok((
                        mcedit2_world::scene_annotations::BlockVisual::Banner(source),
                        None,
                    ))
                }
                mcedit2_world::scene_annotations::BlockVisual::Skull(mut source) => {
                    let mut id = None;
                    if let Some(profile) = source
                        .profile
                        .as_ref()
                        .filter(|_| source.kind == mcedit2_world::skull::Kind::Player)
                    {
                        let legacy = source.legacy_rotation.is_some();
                        let key = crate::skull_skin::key(profile, legacy);
                        let appearance = head_results.entry(key).or_insert_with(|| {
                            let sources = crate::skull_skin::Sources::local(online_skins)?;
                            crate::skull_skin::load(profile, legacy, &sources, cancelled)
                        });
                        let appearance = appearance.as_ref().map_err(Clone::clone)?;
                        diagnostics.push(format!(
                            "待放置头颅 {position:?}：{}",
                            appearance.diagnostic
                        ));
                        source.profile = Some(appearance.profile.clone());
                        if let Some(skin) = &appearance.skin {
                            id = Some(skin.texture.id.clone());
                            if !atlas.entries.contains_key(&skin.texture.id) {
                                textures
                                    .entry(skin.texture.id.clone())
                                    .or_insert_with(|| Arc::new(skin.texture.clone()));
                            }
                        }
                    }
                    Ok((
                        mcedit2_world::scene_annotations::BlockVisual::Skull(source),
                        id,
                    ))
                }
            });
            if cancelled() {
                return Err("待放置方块实体准备已取消".into());
            }
            prepared_visuals.push((position, prepared));
        }
        for (index, entry) in clip.entities.iter().enumerate() {
            let entities = mcedit2_world::entity_catalog::parse_preview(&entry.compound)?;
            let position = entities
                .first()
                .and_then(|e| e.pos)
                .ok_or("待放置实体缺少根 Pos")?;
            let offset = std::array::from_fn(|i| {
                f64::from([x0, y0, z0][i]) + entry.position[i] - position[i]
            });
            for mut entity in entities {
                if cancelled() {
                    return Err("待放置实体名称准备已取消".into());
                }
                if let Ok(visual) = &mut entity.visual {
                    visual.source_version = clip.data_version;
                }
                entity_context(
                    entity.id.as_deref(),
                    &mut entity.visual,
                    bank,
                    clip.item_environment.as_ref(),
                );
                if scene.entity_enabled {
                    if let Ok(visual) = &entity.visual {
                        if !entity
                            .id
                            .as_deref()
                            .is_some_and(|id| bank.has_declarative_entity(id))
                        {
                            let hands = entity
                                .pos
                                .ok_or_else(|| "待放置持有者缺少 Pos".to_string())
                                .and_then(|position| {
                                    crate::equipment_geometry::prepare(
                                        visual,
                                        mcedit2_world::scene_annotations::Kind::Entity,
                                        entity.id.as_deref(),
                                        position,
                                        None,
                                        bank,
                                        online_skins,
                                        cancelled,
                                        clip.item_environment.as_ref().map(|environment| {
                                            crate::item_geometry::ItemContext {
                                                environment,
                                                offset,
                                            }
                                        }),
                                    )
                                });
                            match hands {
                                Ok(hands) => {
                                    for (id, texture) in &hands.textures {
                                        textures
                                            .entry(id.clone())
                                            .or_insert_with(|| Arc::clone(texture));
                                    }
                                    diagnostics.extend(hands.diagnostics.iter().map(|error| {
                                        format!("待放置实体 {}：{error}", entity.path)
                                    }));
                                    if !hands.faces.is_empty() {
                                        prepared_equipment
                                            .insert((index, entity.path.clone()), hands);
                                    }
                                }
                                Err(error) => {
                                    if document {
                                        return Err(error.to_string());
                                    }
                                    if cancelled() {
                                        return Err("待放置手持物准备已取消".into());
                                    }
                                    let error = format!("待放置实体装备 {}：{error}", entity.path);
                                    log::error!("[equipment] {error}");
                                    diagnostics.push(error);
                                }
                            }
                        }
                        match crate::name_geometry::prepare(
                            visual, None, bank, &mut fonts, language, cancelled,
                        ) {
                            Ok(Some(name)) => {
                                for texture in name.textures() {
                                    if !atlas.entries.contains_key(&texture.id) {
                                        textures
                                            .entry(texture.id.clone())
                                            .or_insert_with(|| Arc::clone(texture));
                                    }
                                }
                                prepared_names.insert((index, entity.path.clone()), name);
                            }
                            Ok(None) => (),
                            Err(error) => {
                                if document {
                                    return Err(error.to_string());
                                }
                                if cancelled() {
                                    return Err("待放置实体名称准备已取消".into());
                                }
                                let error = format!("待放置实体名称 {}：{error}", entity.path);
                                log::error!("[entity-name] {error}");
                                diagnostics.push(error);
                            }
                        }
                    }
                }
                if !entity
                    .id
                    .as_deref()
                    .is_some_and(crate::item_geometry::supported)
                    || entity
                        .id
                        .as_deref()
                        .is_some_and(|id| bank.has_declarative_entity(id))
                {
                    continue;
                }
                if let Ok(visual) = &mut entity.visual {
                    visual.source_version = clip.data_version;
                }
                let prepared = entity
                    .visual
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|visual| {
                        crate::item_geometry::prepare(
                            visual,
                            entity.id.as_deref().ok_or("待放置物品实体缺少 ID")?,
                            entity.pos.ok_or("待放置物品实体缺少 Pos")?,
                            bank,
                            clip.map_world.map(|world| world.root.as_path()),
                            clip.map_world.and_then(|world| world.data_version),
                            online_skins,
                            cancelled,
                            clip.item_environment.as_ref().map(|environment| {
                                crate::frame_geometry::ItemContext {
                                    environment,
                                    offset,
                                }
                            }),
                            language,
                            None,
                        )
                    });
                if let Ok(prepared) = &prepared {
                    for (id, texture) in &prepared.textures {
                        textures
                            .entry(id.clone())
                            .or_insert_with(|| Arc::clone(texture));
                    }
                    diagnostics.extend(
                        prepared
                            .diagnostics
                            .iter()
                            .map(|d| format!("待放置物品实体 {}：{d}", entity.path)),
                    );
                }
                prepared_frames.insert((index, entity.path), prepared);
            }
        }
        let atlas = crate::entity_geometry::append_runtime_cached(&atlas, textures, cancelled)?;
        synthetic.texture_bank = Some(Arc::new(bank.with_runtime_atlas(atlas)?));
    }
    let mut mesh =
        spectator::build_scene_cancellable(&synthetic, cancelled).ok_or("待放置预览网格已取消")?;
    let mut chunks = mesh.chunks.to_vec();
    for chunk in &mut chunks {
        let groups = chunk
            .picking
            .source_groups()
            .iter()
            .map(|group| Group {
                quads: group.quads.clone(),
                owner: document.then(|| group.owner.clone()).flatten(),
            })
            .collect();
        chunk.picking = Arc::new(Index::build(
            chunk.origin,
            Arc::clone(&chunk.quads()),
            groups,
        )?);
        // 非实体、非源区块身份，不产生实体 Marker。
        chunk.key.2 = 254;
    }
    let mut visual_groups = PreviewGroups::new();
    if !prepared_visuals.is_empty() {
        let bank = synthetic
            .texture_bank
            .as_ref()
            .ok_or("待放置方块实体缺少材质源")?;
        let atlas = bank.atlas().ok_or("待放置方块实体缺少图集")?;
        for (position, prepared) in prepared_visuals {
            if cancelled() {
                return Err("待放置方块实体网格已取消".into());
            }
            let quads = match prepared.and_then(|(source, id)| match source {
                mcedit2_world::scene_annotations::BlockVisual::Piston(source) => {
                    crate::piston_geometry::quads(
                        &source,
                        bank,
                        position,
                        &synthetic.surface.biomes,
                        scene.show_textures,
                        cancelled,
                    )
                }
                mcedit2_world::scene_annotations::BlockVisual::Banner(source) => {
                    crate::banner_geometry::quads(
                        &source,
                        bank,
                        atlas,
                        banner_materials
                            .get(&position)
                            .ok_or("旗帜缺少已准备材质")?,
                        scene.show_textures,
                    )
                }
                mcedit2_world::scene_annotations::BlockVisual::Skull(source) => {
                    crate::skull_geometry::quads(
                        &source,
                        bank,
                        atlas,
                        id.as_deref(),
                        scene.show_textures,
                    )
                }
                mcedit2_world::scene_annotations::BlockVisual::Sign(source) => {
                    crate::sign_geometry::quads(
                        &source,
                        sign_materials
                            .get(&position)
                            .ok_or("待放置告示牌缺少已准备文字")?,
                        bank,
                        atlas,
                        scene.show_textures,
                    )
                }
            }) {
                Ok(quads) => quads,
                Err(error) => {
                    if document {
                        return Err(error.to_string());
                    }
                    diagnostics.push(format!("待放置方块实体 {position:?}：{error}"));
                    continue;
                }
            };
            let origin = [
                position[0].div_euclid(16) * 16,
                0,
                position[2].div_euclid(16) * 16,
            ];
            let delta = std::array::from_fn(|axis| (position[axis] - origin[axis]) as f32);
            append_preview_geometry(
                &mut visual_groups,
                origin,
                delta,
                quads,
                document_objects.get(&position).copied(),
                block_owner(position),
            )?;
        }
    }
    if !declarative_visuals.is_empty() {
        let bank = synthetic
            .texture_bank
            .as_ref()
            .ok_or("声明式预览缺少资源")?;
        for (position, state, source) in declarative_visuals {
            if cancelled() {
                return Err("声明式方块实体预览已取消".into());
            }
            let faces = source.and_then(|source| {
                bank.declarative_block_quads(&state, Some(&source), scene.show_textures)
                    .ok_or("待放置方块实体定义在准备期间消失")?
                    .1
            });
            let faces = match faces {
                Ok(faces) => faces,
                Err(error) => {
                    if document {
                        return Err(error.to_string());
                    }
                    diagnostics.push(format!("待放置方块实体 {position:?}：{error}"));
                    continue;
                }
            };
            let origin = [
                position[0].div_euclid(16) * 16,
                0,
                position[2].div_euclid(16) * 16,
            ];
            let delta = std::array::from_fn(|axis| (position[axis] - origin[axis]) as f32);
            append_preview_geometry(
                &mut visual_groups,
                origin,
                delta,
                faces,
                document_objects.get(&position).copied(),
                block_owner(position),
            )?;
        }
    }
    if !clip.entities.is_empty() {
        let bank = synthetic
            .texture_bank
            .as_ref()
            .ok_or("实体待放置预览需要已加载材质图集")?;
        let atlas = bank.atlas().ok_or("实体待放置预览缺少图集")?;
        for (entry_index, entry) in clip.entities.iter().enumerate() {
            if cancelled() {
                return Err("待放置实体预览已取消".into());
            }
            let entities = mcedit2_world::entity_catalog::parse_preview(&entry.compound)?;
            let root_pos = entities
                .first()
                .and_then(|v| v.pos)
                .ok_or("待放置实体缺少根 Pos")?;
            let translated_root = [
                f64::from(x0) + entry.position[0],
                f64::from(y0) + entry.position[1],
                f64::from(z0) + entry.position[2],
            ];
            for mut entity in entities {
                if let Ok(visual) = &mut entity.visual {
                    visual.source_version = clip.data_version;
                }
                let Some(position) = entity.pos else {
                    if document {
                        return Err(format!("实体 {} 缺少 Pos，无法显示", entity.path));
                    }
                    diagnostics.push(format!("实体 {} 缺少 Pos，无法显示", entity.path));
                    continue;
                };
                let target: [f64; 3] = std::array::from_fn(|axis| {
                    translated_root[axis] + position[axis] - root_pos[axis]
                });
                entity_context(
                    entity.id.as_deref(),
                    &mut entity.visual,
                    bank,
                    clip.item_environment.as_ref(),
                );
                if entity
                    .id
                    .as_deref()
                    .is_some_and(mcedit2_world::entity_water::needs_contact)
                    && !entity
                        .id
                        .as_deref()
                        .is_some_and(|id| bank.has_declarative_entity(id))
                {
                    let result = entity
                        .visual
                        .as_mut()
                        .map_err(|error| error.clone())
                        .and_then(|visual| {
                            let version = visual.source_version.ok_or("鱼类预览缺少实际源版本")?;
                            let tags = if let Some(environment) = &clip.item_environment {
                                environment.data.water_tags(version)?
                            } else {
                                // 独立文件预览显式使用该源版本的原生标签，不冒充某个世界的数据包。
                                Arc::new(mcedit2_world::entity_water::Tags::with_layers(
                                    version,
                                    &[],
                                )?)
                            };
                            let contact = mcedit2_world::entity_water::inspect(
                                entity.id.as_deref().expect("鱼类实际 ID 已读取"),
                                visual,
                                target,
                                &tags,
                                &mut |[x, y, z]| {
                                    let (nx, ny, nz) = (
                                        i64::from(x) - i64::from(x0),
                                        i64::from(y) - i64::from(y0),
                                        i64::from(z) - i64::from(z0),
                                    );
                                    if nx < 0
                                        || ny < 0
                                        || nz < 0
                                        || nx >= sx as i64
                                        || ny >= sy as i64
                                        || nz >= sz as i64
                                    {
                                        return Err(mcedit2_world::WorldError::Invalid(format!(
                                            "鱼类预览流体源格 {:?} 位于实际结构之外",
                                            [x, y, z]
                                        )));
                                    }
                                    let source_index = index(nx as usize, ny as usize, nz as usize);
                                    if clip.mask.as_ref().is_some_and(|mask| !mask[source_index]) {
                                        return Err(mcedit2_world::WorldError::Invalid(
                                            "鱼类预览所需流体源格未包含在结构选区中".into(),
                                        ));
                                    }
                                    Ok(clip.blocks[source_index].clone())
                                },
                            )
                            .map_err(|error| error.to_string())?;
                            mcedit2_world::entity_water::set(visual, contact);
                            Ok(())
                        });
                    if let Err(error) = result {
                        entity.visual = Err(error);
                    }
                }
                let mut center = [target[0] - 0.5, target[1], target[2] - 0.5];
                let axis = |v: f64| -> Result<i32, String> {
                    let origin = (v / 16.).floor() * 16.;
                    if !origin.is_finite() || origin < i32::MIN as f64 || origin > i32::MAX as f64 {
                        return Err("待放置实体原点越界".into());
                    }
                    Ok(origin as i32)
                };
                if entity.id.as_deref() == Some("minecraft:marker") {
                    continue;
                }
                let hands = prepared_equipment.get(&(entry_index, entity.path.clone()));
                let name = prepared_names
                    .get(&(entry_index, entity.path.clone()))
                    .filter(|name| name.always_visible);
                let is_frame = entity
                    .id
                    .as_deref()
                    .is_some_and(mcedit2_world::hanging::is_frame);
                let invisible=!is_frame && entity.visual.as_ref().is_ok_and(|visual|
                    matches!(visual.appearance.get("Invisible"),Some(mcedit2_world::nbt_document::Value::Byte(v)) if *v!=0));
                let has_fire = match entity
                    .visual
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(crate::entity_fire::present)
                {
                    Ok(value) => value,
                    Err(error) => {
                        if document {
                            return Err(error.to_string());
                        }
                        diagnostics.push(format!("待放置实体火焰 {}：{error}", entity.path));
                        false
                    }
                };
                if invisible
                    && name.is_none()
                    && hands.is_none()
                    && !has_fire
                    && !entity
                        .id
                        .as_deref()
                        .is_some_and(crate::entity_geometry::has_invisible_layer)
                {
                    continue;
                }
                let result = entity
                    .visual
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|visual| {
                        let declared = entity
                            .id
                            .as_deref()
                            .is_some_and(|id| bank.has_declarative_entity(id));
                        if entity
                            .id
                            .as_deref()
                            .is_some_and(crate::item_geometry::supported)
                            && !declared
                        {
                            prepared_frames
                                .get(&(entry_index, entity.path.clone()))
                                .ok_or("待放置物品实体未准备")?
                                .as_ref()
                                .map_err(Clone::clone)
                                .and_then(|prepared| {
                                    Ok((
                                        prepared.center,
                                        prepared.quads(atlas, scene.show_textures)?,
                                    ))
                                })
                        } else if entity
                            .id
                            .as_deref()
                            .is_some_and(mcedit2_world::painting::is_painting)
                            && !declared
                        {
                            crate::painting_geometry::quads_bound(
                                visual,
                                bank,
                                atlas,
                                scene.show_textures,
                                clip.painting_variants,
                            )
                        } else if entity
                            .id
                            .as_deref()
                            .is_some_and(mcedit2_world::entity_catalog::is_experience_orb)
                            && !declared
                        {
                            crate::experience_geometry::quads(
                                visual,
                                bank,
                                atlas,
                                scene.show_textures,
                            )
                            .map(|faces| {
                                ([position[0] - 0.5, position[1], position[2] - 0.5], faces)
                            })
                        } else if declared {
                            bank.declarative_entity_quads(
                                entity.id.as_deref().ok_or("声明式预览实体缺少 ID")?,
                                visual,
                            )
                            .ok_or("声明式预览实体定义消失")?
                            .map(|faces| {
                                ([position[0] - 0.5, position[1], position[2] - 0.5], faces)
                            })
                        } else {
                            mcedit2_world::entity_model::Catalog::for_data_version(
                                visual.source_version,
                            )
                            .and_then(|catalog| {
                                crate::entity_geometry::compile_visual(
                                    catalog,
                                    bank,
                                    mcedit2_world::scene_annotations::Kind::Entity,
                                    entity.id.as_deref(),
                                    visual,
                                    None,
                                    language,
                                )
                            })
                            .map(|faces| {
                                ([position[0] - 0.5, position[1], position[2] - 0.5], faces)
                            })
                        }
                    });
                let (mut quads, source_center) = match result {
                    Ok((source_center, faces)) => {
                        center =
                            std::array::from_fn(|i| target[i] + source_center[i] - position[i]);
                        (faces, source_center)
                    }
                    Err(error) => {
                        if document {
                            return Err(error.to_string());
                        }
                        diagnostics.push(format!(
                            "待放置实体 {}（目标 {target:?}）：{error}",
                            entity.path
                        ));
                        continue;
                    }
                };
                if !scene.show_textures {
                    quads.retain(|quad| quad.mode & crate::material_mode::ADDITIVE == 0);
                    for q in &mut quads {
                        for color in &mut q.rgba {
                            if crate::material_mode::pre_tint_alpha(color[3]) {
                                color[3] = 1.;
                            }
                        }
                        q.uv = [atlas.white.sample(0.5, 0.5); 4];
                        q.mode = crate::material_mode::untextured(q.mode);
                        q.atlas_page = atlas.white.page;
                    }
                }
                for quad in &mut quads {
                    if quad.mode & crate::material_mode::ADDITIVE != 0 {
                        // 独立结构预览没有世界光照贴图，旧眼睛层也明确使用不读光照的预览值。
                        quad.light = crate::lighting::UNLIT;
                    }
                }
                {
                    let labels = name
                        .map(|name| {
                            if is_frame {
                                let visual = entity.visual.as_ref().map_err(Clone::clone)?;
                                let frame = mcedit2_world::hanging::Frame::read(visual)?;
                                let item = visual.source_field_optional("Item")?;
                                let map = item.as_ref().is_some_and(|item| {
                                    mcedit2_world::hanging::is_map_item(
                                        item,
                                        visual.source_version.expect("展示框版本已核实"),
                                    )
                                });
                                let maximum = frame.render_bounds(map).1;
                                let frame_center = frame.center();
                                let anchor = std::array::from_fn(|i| {
                                    let source = if i == 1 {
                                        maximum[i] + 0.5
                                    } else {
                                        frame_center[i] - 0.5
                                    };
                                    (source + target[i] - position[i] - center[i]) as f32
                                });
                                return name.quads(atlas, anchor);
                            }
                            let top = quads
                                .iter()
                                .map(|q| q.bounds().1[1])
                                .reduce(f32::max)
                                .ok_or("实际模型未提供名称标签范围")?;
                            name.quads(atlas, [0., top + 0.5, 0.])
                        })
                        .transpose();
                    if invisible {
                        quads.retain(|quad| {
                            quad.mode & crate::material_mode::VISIBLE_WHEN_INVISIBLE != 0
                        });
                    }
                    match labels {
                        Ok(Some(labels)) => quads.extend(labels),
                        Ok(None) => (),
                        Err(error) => {
                            if document {
                                return Err(error.to_string());
                            }
                            let error = format!("待放置实体名称 {}：{error}", entity.path);
                            log::error!("[entity-name] {error}");
                            diagnostics.push(error);
                        }
                    }
                }
                {
                    if let Some(hands) = hands {
                        let hand_delta = std::array::from_fn(|i| {
                            (hands.center[i] + target[i] - position[i] - center[i]) as f32
                        });
                        quads.extend(
                            hands
                                .quads(atlas, scene.show_textures)?
                                .into_iter()
                                .map(|quad| quad.translated(hand_delta)),
                        );
                    }
                    let fire = (|| {
                        crate::entity_fire::quads(
                            entity.id.as_deref().ok_or("待放置燃烧实体缺少 ID")?,
                            entity.visual.as_ref().map_err(Clone::clone)?,
                            position,
                            source_center,
                            bank,
                            atlas,
                            scene.show_textures,
                            &mcedit2_world::scene_annotations::Lighting::Preview,
                            clip.painting_variants,
                        )
                    })();
                    match fire {
                        Ok(faces) => quads.extend(faces),
                        Err(error) => {
                            if document {
                                return Err(error.to_string());
                            }
                            let error = format!("待放置实体火焰 {}：{error}", entity.path);
                            log::error!("[entity-fire] {error}");
                            diagnostics.push(error);
                        }
                    }
                }
                let origin = [axis(center[0])?, 0, axis(center[2])?];
                let delta =
                    std::array::from_fn(|axis| (center[axis] - f64::from(origin[axis])) as f32);
                append_preview_geometry(
                    &mut visual_groups,
                    origin,
                    delta,
                    quads,
                    document.then_some(mcedit2_world::structure_document::Object::Entity(
                        entry_index,
                    )),
                    None,
                )?;
            }
        }
    }
    let mut objects = Vec::new();
    for ((cx, cz), (faces, owners, groups)) in visual_groups {
        if cancelled() {
            return Err("待放置外观区块整理已取消".into());
        }
        let origin = [cx * 16, 0, cz * 16];
        let quads = Arc::new(faces);
        let picking = Arc::new(Index::build(origin, Arc::clone(&quads), owners)?);
        if document {
            objects.push(ObjectPart {
                picking: Arc::clone(&picking),
                groups,
            });
        }
        chunks.push(crate::spectator::MeshChunk::new(
            (cx, cz, 254, 0, chunks.len() + 1),
            origin,
            quads,
            picking,
        ));
    }
    mesh.chunks = chunks.into();
    mesh.terrain = None;
    mesh.entities = None;
    mesh.complete = true;
    Ok(Frame {
        mesh: Arc::new(mesh),
        anchor: [x0, y0, z0],
        size: clip.size,
        diagnostics,
        show_textures: scene.show_textures,
        online_skins,
        language,
        objects,
    })
}
