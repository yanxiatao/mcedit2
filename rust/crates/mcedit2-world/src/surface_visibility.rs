//! 源读取的可见性接口；渲染资源的解释由调用方提供，不使世界格式层依赖 GPU 或 UI。
use crate::{
    sections::State,
    surface::{self, VoxelMaterial},
};

/// 精确源格的只读查询输入；位置用于资源模型的加权选择，不能用采样格冒充。
#[derive(Clone, Copy)]
pub struct Cell<'a> {
    pub state: &'a State,
    pub kind: VoxelMaterial,
    pub position: (i32, i32, i32),
}
/// 六个真实邻格方向，避免把方向标记与任意整数混用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    East,
    West,
    South,
    North,
}
impl Direction {
    pub const ALL: [Self; 6] = [
        Self::Up,
        Self::Down,
        Self::East,
        Self::West,
        Self::South,
        Self::North,
    ];
    pub fn offset(self) -> [i32; 3] {
        match self {
            Self::Up => [0, 1, 0],
            Self::Down => [0, -1, 0],
            Self::East => [1, 0, 0],
            Self::West => [-1, 0, 0],
            Self::South => [0, 0, 1],
            Self::North => [0, 0, -1],
        }
    }
    pub fn flag(self) -> u8 {
        match self {
            Self::Up => surface::FACE_TOP,
            Self::Down => surface::FACE_BOTTOM,
            Self::East => surface::FACE_POS_X,
            Self::West => surface::FACE_NEG_X,
            Self::South => surface::FACE_POS_Z,
            Self::North => surface::FACE_NEG_Z,
        }
    }
    pub fn opposite(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
            Self::East => Self::West,
            Self::West => Self::East,
            Self::South => Self::North,
            Self::North => Self::South,
        }
    }
}
/// 实际显示模型的单格判定；固定模型可按调色板共享，加权模型必须使用该格真实坐标。
#[derive(Clone, Copy)]
pub struct CellInfo {
    pub visible: bool,
    pub opaque: u8,
    pub ordinary_faces: bool,
    pub unconditional: bool,
}
/// 每个后台读取器独立持有的可见性查询器，可缓存模型分类而不锁住逐格循环。
pub trait Visibility {
    /// 返回能够脱离坐标复用的真实模型属性；None 明确表示必须走原逐格判定。
    fn fixed_cell(&mut self, state: &State, kind: VoxelMaterial) -> Option<CellInfo>;
    /// 当前读取器是否允许预先计算真实格判定；数据检查模式继续使用原逐面规则。
    fn prepare_cells(&self) -> bool;
    /// 在真实坐标计算一次，供同段六邻格共享；None 表示没有资源模型证明。
    fn evaluate_cell(&mut self, source: Cell<'_>) -> Option<CellInfo>;
    /// 调用方是否会消费 Section 连通数据；纯色与无资源读取不生成无用拓扑。
    fn track_sections(&self) -> bool;
    /// 六个显示边界均完整不透明时才封闭空域；不得用材质类别猜测模组形状。
    fn opaque_cell(&mut self, source: Cell<'_>) -> bool;
    /// 最高列预览在资源模式下保留作者模型的面，交由建模阶段证明遮挡。
    fn resource_aware(&self) -> bool;
    /// 原版 HalfTransparentBlock / LeavesBlock 邻格所需的侧面水贴图。
    fn water_overlay(&self, neighbor: Cell<'_>) -> bool;
    /// 当前方向是否仍需交给建模阶段；未知邻格的处理由调用者的实际窗口边界决定。
    fn exposed(
        &mut self,
        source: Cell<'_>,
        direction: Direction,
        neighbor: Option<Cell<'_>>,
    ) -> bool;
    /// 该源模型是否含不依赖邻面标记的几何；即使六个方向均被遮挡也必须保留源格。
    fn unconditional(&mut self, source: Cell<'_>) -> bool;
    /// 从真实邻格求流体显示高度；不得从曝光面标记猜测上方是否仍有流体。
    fn fluid_sample(
        &mut self,
        source: Cell<'_>,
    ) -> Result<crate::fluid_surface::Sample, crate::WorldError>;
    /// 状态本身能完整决定是否含流体时返回 Some，供调色板惰性缓存；首次实际使用才校验。
    /// 坐标相关的查询明确返回 None，不能把一个位置的结果套用到整段。
    fn fixed_fluid(
        &mut self,
        state: &State,
        kind: VoxelMaterial,
    ) -> Result<Option<bool>, crate::WorldError>;
    /// 流体遮挡使用已验证的运行时形状或完整边界证明，与本体邻面标记独立。
    fn fluid_occlusion(
        &mut self,
        source: Cell<'_>,
    ) -> Result<crate::fluid_occlusion::Occlusion, crate::WorldError>;
}
/// 无渲染资源的 CLI / 数据检查使用既有原版状态规则，不推断模组模型。
pub struct Native;
impl Visibility for Native {
    fn fixed_fluid(
        &mut self,
        _: &State,
        _: VoxelMaterial,
    ) -> Result<Option<bool>, crate::WorldError> {
        Ok(None)
    }
    fn prepare_cells(&self) -> bool {
        false
    }
    fn evaluate_cell(&mut self, _: Cell<'_>) -> Option<CellInfo> {
        None
    }
    fn fixed_cell(&mut self, _: &State, _: VoxelMaterial) -> Option<CellInfo> {
        None
    }
    fn track_sections(&self) -> bool {
        false
    }
    fn opaque_cell(&mut self, _: Cell<'_>) -> bool {
        // 没有资源的调用者不发布显示模型连通性。
        false
    }
    fn fluid_occlusion(
        &mut self,
        source: Cell<'_>,
    ) -> Result<crate::fluid_occlusion::Occlusion, crate::WorldError> {
        Ok(crate::fluid_occlusion::vanilla(source.state)?.unwrap_or(
            crate::fluid_occlusion::Occlusion::Boundary(
                if surface::known_full_occluder(source.state) {
                    surface::ALL_FACES
                } else {
                    0
                },
            ),
        ))
    }
    fn water_overlay(&self, neighbor: Cell<'_>) -> bool {
        crate::native_material::water_overlay(neighbor.state)
    }
    fn resource_aware(&self) -> bool {
        false
    }
    fn exposed(&mut self, source: Cell<'_>, _: Direction, neighbor: Option<Cell<'_>>) -> bool {
        surface::exact_face_exposed_against(
            source.state,
            source.kind,
            neighbor.map(|n| (n.state, n.kind)),
        )
    }
    fn unconditional(&mut self, _: Cell<'_>) -> bool {
        false
    }
    fn fluid_sample(
        &mut self,
        source: Cell<'_>,
    ) -> Result<crate::fluid_surface::Sample, crate::WorldError> {
        crate::fluid_surface::Sample::vanilla(source.state)
    }
}

/// 已读取区块缺少 section 表示空 section；错误索引和未知区块仍由调用者保留为未知。
/// 该共享空气仅属于邻域查询，不追加到源体素数组，也不改写区块或编辑数据。
pub(crate) fn fluid_state(chunk: &crate::sections::Chunk, x: u8, y: i32, z: u8) -> Option<&State> {
    if x >= 16 || z >= 16 {
        return None;
    }
    let section_y = i8::try_from(y.div_euclid(16)).ok()?;
    match chunk
        .sections
        .binary_search_by_key(&section_y, |section| section.y)
    {
        Ok(index) => chunk.sections[index].block(x, y.rem_euclid(16) as u8, z),
        Err(_) => {
            static AIR: std::sync::OnceLock<State> = std::sync::OnceLock::new();
            Some(AIR.get_or_init(|| State {
                name: "minecraft:air".into(),
                properties: Vec::new(),
            }))
        }
    }
}

/// 邻区块未知时不生成临时液体边界墙；显式体积外则保留剖切方向。
pub(crate) fn available_fluid_faces(
    position: (i32, i32, i32),
    clip: Option<crate::editing::Bounds>,
    known_chunk: impl Fn(i32, i32) -> bool,
) -> u8 {
    Direction::ALL.into_iter().fold(0, |mask, direction| {
        let [dx, dy, dz] = direction.offset();
        let available = match (
            position.0.checked_add(dx),
            position.1.checked_add(dy),
            position.2.checked_add(dz),
        ) {
            (Some(x), Some(y), Some(z)) => {
                clip.is_some_and(|clip| {
                    x < clip.minimum[0]
                        || x > clip.maximum[0]
                        || y < clip.minimum[1]
                        || y > clip.maximum[1]
                        || z < clip.minimum[2]
                        || z > clip.maximum[2]
                }) || known_chunk(x.div_euclid(16), z.div_euclid(16))
            }
            _ => true,
        };
        if available {
            mask | direction.flag()
        } else {
            mask
        }
    })
}

/// 查询未裁减的原始邻域；已经筛选的显示体素不能提供完整流体邻格。
pub fn fluid_surface<'a>(
    visibility: &mut dyn Visibility,
    source: Cell<'a>,
    available_faces: u8,
    clip: Option<crate::editing::Bounds>,
    query: &mut dyn FnMut((i32, i32, i32)) -> Option<Cell<'a>>,
) -> Result<Option<crate::fluid_surface::FluidSurface>, crate::WorldError> {
    use crate::fluid_surface::{inspect, Sample};
    let Some(surface) = inspect(
        visibility.fluid_sample(source)?,
        source.position,
        &mut |position| {
            query(position).map_or(Ok(Sample::Unknown), |cell| visibility.fluid_sample(cell))
        },
    )?
    else {
        return Ok(None);
    };
    // 这是可查询邻格 / 显式剖切方向，不是方块本体的曝光面。
    let mut visible = available_faces;
    let own_occlusion = visibility.fluid_occlusion(source)?;
    let mut overlays = 0;
    for direction in Direction::ALL {
        if visible & direction.flag() == 0 {
            continue;
        }
        if own_occlusion.covers(direction, 1.0) {
            visible &= !direction.flag();
            continue;
        }
        let [dx, dy, dz] = direction.offset();
        let (Some(x), Some(y), Some(z)) = (
            source.position.0.checked_add(dx),
            source.position.1.checked_add(dy),
            source.position.2.checked_add(dz),
        ) else {
            continue;
        };
        if clip.is_some_and(|clip| {
            x < clip.minimum[0]
                || x > clip.maximum[0]
                || y < clip.minimum[1]
                || y > clip.maximum[1]
                || z < clip.minimum[2]
                || z > clip.maximum[2]
        }) {
            continue;
        }
        let neighbor = query((x, y, z));
        if neighbor
            .map(|cell| visibility.fluid_sample(cell))
            .transpose()?
            .is_some_and(
                |sample| matches!(sample, Sample::Fluid(kind, _) if kind == surface.kind()),
            )
        {
            visible &= !direction.flag();
            continue;
        }
        let height = match direction {
            Direction::Up => surface
                .height(false, false)
                .min(surface.height(true, false))
                .min(surface.height(false, true))
                .min(surface.height(true, true)),
            Direction::Down => 1.0,
            Direction::East => surface.height(true, false).max(surface.height(true, true)),
            Direction::West => surface
                .height(false, false)
                .max(surface.height(false, true)),
            Direction::South => surface.height(false, true).max(surface.height(true, true)),
            Direction::North => surface
                .height(false, false)
                .max(surface.height(true, false)),
        };
        // 低于格顶的水面不与上方格底相接，不能用上方整块模型直接删除。
        if (direction != Direction::Up || height >= 1.0 - 1e-7)
            && neighbor
                .map(|cell| visibility.fluid_occlusion(cell))
                .transpose()?
                .is_some_and(|shape| shape.covers(direction.opposite(), height))
        {
            visible &= !direction.flag();
            continue;
        }
        if surface.kind() == crate::fluid_surface::Kind::Water
            && dy == 0
            && neighbor.is_some_and(|cell| visibility.water_overlay(cell))
        {
            overlays |= direction.flag();
        }
    }
    surface
        .with_faces(visible)?
        .with_overlays(overlays)
        .map(Some)
}
