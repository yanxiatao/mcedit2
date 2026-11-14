//! 原始实体、方块实体及计划更新的位置快照；只读，不把标记变成方块或碰撞箱。
use crate::{
    block_lookup::ReadOnlyRegionCache,
    entity_catalog,
    sections::{parse_root, Tag},
    WorldError, WorldSummary,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[path = "scene_annotation_commands.rs"]
mod commands;
#[path = "scene_annotation_light.rs"]
mod light;
#[path = "scene_annotation_water.rs"]
mod water;
#[path = "scene_annotation_zombification.rs"]
mod zombification;
pub use commands::CommandLayer;
pub use light::Lighting;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Entity,
    Item,
    BlockEntity,
    BlockTick,
    FluidTick,
    Player,
    ChunkSection,
    Heightmap,
    SpawnDark,
    SpawnNight,
    TerrainIncomplete,
}
pub type LayerMask = u16;
/// 区块段和高度面具有真实范围，不复用实体的一格定位框。
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    Box {
        minimum: [f64; 3],
        maximum: [f64; 3],
    },
    Horizontal {
        minimum: [f64; 2],
        maximum: [f64; 2],
        y: f64,
    },
}
impl Geometry {
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        match *self {
            Self::Box { minimum, maximum } => (minimum, maximum),
            Self::Horizontal {
                minimum,
                maximum,
                y,
            } => (
                [minimum[0], y - 0.001, minimum[1]],
                [maximum[0], y + 0.001, maximum[1]],
            ),
        }
    }
}
impl Kind {
    /// 原生位置及区块图层的完整目录，显示控件与配置标志核对共用。
    pub const ALL: [Self; 11] = [
        Self::Entity,
        Self::Item,
        Self::BlockEntity,
        Self::BlockTick,
        Self::FluidTick,
        Self::Player,
        Self::ChunkSection,
        Self::Heightmap,
        Self::SpawnDark,
        Self::SpawnNight,
        Self::TerrainIncomplete,
    ];
    pub const fn bit(self) -> LayerMask {
        1 << self as u8
    }
}
/// 需要真实方块实体载荷才能绘制的原版外观；失败仍由同一源标记提供定位和诊断。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockVisual {
    Banner(crate::banner::Source),
    Skull(crate::skull::Source),
    Sign(crate::sign::Source),
    Piston(crate::piston::Source),
}

fn is_block_visual_name(name: &str) -> bool {
    crate::banner::is_banner_state_name(name)
        || crate::skull::is_skull_state_name(name)
        || crate::sign::is_sign_state_name(name)
        || crate::piston::is_piston_state_name(name)
}

fn is_block_visual_id(id: &str) -> bool {
    crate::banner::is_banner_id(id)
        || crate::skull::is_skull_id(id)
        || crate::sign::is_sign_id(id)
        || crate::piston::is_piston_id(id)
}

fn read_block_visual(
    entry: &Tag<'_>,
    state: &crate::sections::State,
    version: Option<i32>,
) -> Result<BlockVisual, String> {
    let id = entry
        .require("id", 8)
        .and_then(Tag::string)
        .map_err(|e| e.to_string())?;
    // 命名状态先确定模型类型，避免错误的 BE 类型覆盖真实方块；数字状态由资源绑定后再次核实。
    if crate::banner::is_banner_state_name(&state.name) {
        return crate::banner::read(entry, state, version).map(BlockVisual::Banner);
    }
    if crate::skull::is_skull_state_name(&state.name) {
        return crate::skull::read(entry, state, version).map(BlockVisual::Skull);
    }
    if crate::sign::is_sign_state_name(&state.name) {
        return crate::sign::read(entry, state, version).map(BlockVisual::Sign);
    }
    if crate::piston::is_piston_state_name(&state.name) {
        return crate::piston::read(entry, state, version).map(BlockVisual::Piston);
    }
    if state.name.starts_with("numeric:") {
        if crate::piston::is_piston_id(&id) {
            return crate::piston::read(entry, state, version).map(BlockVisual::Piston);
        }
        if crate::banner::is_banner_id(&id) {
            return crate::banner::read(entry, state, version).map(BlockVisual::Banner);
        }
        if crate::skull::is_skull_id(&id) {
            return crate::skull::read(entry, state, version).map(BlockVisual::Skull);
        }
        if crate::sign::is_sign_id(&id) {
            return crate::sign::read(entry, state, version).map(BlockVisual::Sign);
        }
    }
    Err(format!("方块 {} 与外观载荷 {id} 不匹配", state.name))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub kind: Kind,
    pub id: Option<String>,
    /// Minecraft 连续实体位置或方块坐标；悬挂实体使用原版中心，原始 Pos 保留于源 NBT。
    pub position: [f64; 3],
    pub path: String,
    pub source: String,
    pub chunk: (i32, i32),
    pub sha256: String,
    pub fields: usize,
    pub bytes: usize,
    /// 保留源字段的数值，不推测旧版与现代版的时间基准。
    pub tick: Option<i64>,
    pub priority: Option<i32>,
    pub visual: Option<std::sync::Arc<crate::entity_catalog::EntityVisual>>,
    /// 调色板及同代 Session 覆盖提供的显示状态，不从方块实体 ID 猜方向或底色。
    pub block_state: Option<std::sync::Arc<crate::sections::State>>,
    /// 源外观无效或所需 BE 缺失时保持显式失败表示，显示端使用缺失材质。
    pub visual_error: Option<String>,
    /// 与该 BE 原始载荷同代的动态方块外观；损坏时保留定位标记及诊断。
    pub block_visual: Option<std::sync::Arc<BlockVisual>>,
    pub geometry: Option<Geometry>,
}
impl Marker {
    /// 地形模型以整数方块中心为基底，Minecraft 连续实体位置需要移去半格。
    pub fn center(&self) -> [f64; 3] {
        match self.kind {
            Kind::Entity | Kind::Item | Kind::Player => [
                self.position[0] - 0.5,
                self.position[1],
                self.position[2] - 0.5,
            ],
            _ => self.position,
        }
    }
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        if let Some(geometry) = &self.geometry {
            return geometry.bounds();
        }
        let center = self.center();
        (center.map(|v| v - 0.5), center.map(|v| v + 0.5))
    }
}
#[derive(Clone, Debug, Default)]
pub struct ChunkAnnotations {
    pub markers: Vec<Marker>,
    pub diagnostics: Vec<String>,
    /// 显示固定使用 Preview 全亮度；不采集世界光照探针。
    pub lighting: Lighting,
}
fn invalid(error: impl ToString) -> WorldError {
    WorldError::Invalid(error.to_string())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn list<'a>(tag: &'a Tag<'a>, name: &str) -> Result<Option<&'a Tag<'a>>, WorldError> {
    let Some(list) = tag.field(name) else {
        return Ok(None);
    };
    if list.kind != 9 || list.items.iter().any(|item| item.kind != 10) {
        return Err(invalid(format!("位置图层的 {name} 必须为 Compound 列表")));
    }
    Ok(Some(list))
}
fn optional_number(tag: &Tag<'_>, name: &str) -> Result<Option<i64>, WorldError> {
    tag.field(name)
        .map(|v| match v.kind {
            3 => v.i32().map(i64::from).map_err(invalid),
            4 if v.raw.len() == 8 => Ok(i64::from_be_bytes(v.raw.try_into().unwrap())),
            _ => Err(invalid(format!("位置图层字段 {name} 必须为 Int 或 Long"))),
        })
        .transpose()
}
fn positioned(
    container: &Tag<'_>,
    name: &str,
    kind: Kind,
    chunk: (i32, i32),
    source: &str,
    out: &mut ChunkAnnotations,
    visual_chunk: Option<&Result<crate::sections::Chunk, String>>,
    numeric_visuals: &BTreeSet<(u16, u8)>,
    declarative: &BTreeSet<String>,
    display: Option<&crate::editing::Session>,
) -> Result<(), WorldError> {
    let Some(list) = list(container, name)? else {
        return Ok(());
    };
    let mut positions = BTreeSet::new();
    for (index, entry) in list.items.iter().enumerate() {
        let xyz = ["x", "y", "z"].map(|key| entry.require(key, 3).and_then(Tag::i32));
        let [x, y, z] = xyz;
        let position = [
            x.map_err(invalid)?,
            y.map_err(invalid)?,
            z.map_err(invalid)?,
        ];
        if position[0].div_euclid(16) != chunk.0 || position[2].div_euclid(16) != chunk.1 {
            return Err(invalid(format!(
                "{name}/{index} 的坐标不属于源区块 {chunk:?}"
            )));
        }
        if kind == Kind::BlockEntity && !positions.insert(position) {
            return Err(invalid(format!(
                "{name} 存在重复的方块实体坐标 {position:?}"
            )));
        }
        let id = entry
            .field(if kind == Kind::BlockEntity { "id" } else { "i" })
            .map(|tag| {
                if tag.kind == 8 {
                    tag.string().map_err(invalid)
                } else if tag.kind == 3 && kind != Kind::BlockEntity {
                    tag.i32().map(|v| v.to_string()).map_err(invalid)
                } else {
                    Err(invalid(format!("{name}/{index} 的类型标识格式无效")))
                }
            })
            .transpose()?;
        let tick = if kind == Kind::BlockEntity {
            None
        } else {
            optional_number(entry, "t")?
        };
        let priority = if kind == Kind::BlockEntity {
            None
        } else {
            entry
                .field("p")
                .map(|v| v.i32().map_err(invalid))
                .transpose()?
        };
        let mut visual_error = None;
        let command_visual =
            if kind == Kind::BlockEntity && id.as_deref().is_some_and(commands::is_command_id) {
                match entity_catalog::EntityVisual::read(entry).and_then(|visual| {
                    commands::name(&visual)?;
                    Ok(visual)
                }) {
                    Ok(v) => Some(std::sync::Arc::new(v)),
                    Err(error) => {
                        out.diagnostics
                            .push(format!("命令方块外观 {name}/{index}：{error}"));
                        visual_error = Some(error);
                        None
                    }
                }
            } else {
                None
            };
        let block_state = (kind == Kind::BlockEntity)
            .then(|| {
                display
                    .and_then(|session| {
                        session.display_change((position[0], position[1], position[2]))
                    })
                    .or_else(|| {
                        visual_chunk?.as_ref().ok()?.block(
                            position[0].rem_euclid(16) as u8,
                            position[1],
                            position[2].rem_euclid(16) as u8,
                        )
                    })
                    .cloned()
                    .map(std::sync::Arc::new)
            })
            .flatten();
        let visual = if kind == Kind::BlockEntity
            && block_state
                .as_deref()
                .is_some_and(|state| declarative.contains(&state.name))
        {
            match entity_catalog::EntityVisual::read_block_entity(
                entry,
                visual_chunk
                    .and_then(|chunk| chunk.as_ref().ok())
                    .and_then(|chunk| chunk.data_version),
            ) {
                Ok(visual) => Some(std::sync::Arc::new(visual)),
                Err(error) => {
                    out.diagnostics
                        .push(format!("声明式方块实体 {name}/{index} 源外观无效：{error}"));
                    None
                }
            }
        } else {
            command_visual
        };
        let block_visual = if kind == Kind::BlockEntity
            && (id.as_deref().is_some_and(is_block_visual_id)
                || block_state
                    .as_deref()
                    .is_some_and(|state| block_visual_state(state, numeric_visuals)))
        {
            let read = || -> Result<BlockVisual, String> {
                let decoded = visual_chunk
                    .ok_or("动态方块缺少同代源区块")?
                    .as_ref()
                    .map_err(String::clone)?;
                let state = block_state
                    .as_deref()
                    .ok_or("动态方块位置缺少真实源方块状态")?;
                read_block_visual(entry, state, decoded.data_version)
            };
            match read() {
                Ok(source) => Some(std::sync::Arc::new(source)),
                Err(error) => {
                    out.diagnostics
                        .push(format!("动态方块外观 {name}/{index}：{error}"));
                    visual_error = Some(error);
                    None
                }
            }
        } else {
            None
        };
        out.markers.push(Marker {
            kind,
            id,
            position: position.map(f64::from),
            path: format!("{name}/{index}"),
            source: source.into(),
            chunk,
            sha256: digest(entry.raw),
            fields: entry.fields.len(),
            bytes: entry.raw.len(),
            tick,
            priority,
            visual,
            block_state,
            visual_error,
            block_visual,
            geometry: None,
        });
    }
    Ok(())
}
fn block_visual_state(state: &crate::sections::State, numeric: &BTreeSet<(u16, u8)>) -> bool {
    is_block_visual_name(&state.name)
        || state
            .name
            .strip_prefix("numeric:")
            .and_then(|id| id.parse::<u16>().ok())
            .is_some_and(|id| {
                state
                    .properties
                    .iter()
                    .find(|(name, _)| name == "data")
                    .and_then(|(_, data)| data.parse::<u8>().ok())
                    .is_some_and(|data| numeric.contains(&(id, data)))
            })
}
fn terrain(
    raw: &[u8],
    chunk: (i32, i32),
    out: &mut ChunkAnnotations,
    numeric: &BTreeSet<(u16, u8)>,
    declarative: &BTreeSet<String>,
    display: Option<&crate::editing::Session>,
) -> Result<(), WorldError> {
    let root = parse_root(raw).map_err(invalid)?;
    let container = root.field("Level").unwrap_or(&root);
    if container.kind != 10 {
        return Err(invalid("位置图层的地形容器不是 Compound"));
    }
    for (key, expected) in [("xPos", chunk.0), ("zPos", chunk.1)] {
        if container
            .field(key)
            .map(Tag::i32)
            .transpose()
            .map_err(invalid)?
            .is_some_and(|v| v != expected)
        {
            return Err(invalid("位置图层的地形坐标与 Region 槽位不符"));
        }
    }
    let sections = container
        .field("sections")
        .or_else(|| container.field("Sections"));
    let mut numeric_low_bytes = [false; 256];
    for &(id, _) in numeric {
        numeric_low_bytes[(id & 255) as usize] = true;
    }
    let possible = display.is_some_and(|session| {
        session
            .display_changes_in_chunk(chunk.0, chunk.1)
            .any(|(_, state)| {
                block_visual_state(state, numeric) || declarative.contains(&state.name)
            })
    }) || sections.is_some_and(|sections| {
        sections.items.iter().any(|section| {
            section
                .field("block_states")
                .and_then(|states| states.field("palette"))
                .or_else(|| section.field("Palette"))
                .is_some_and(|palette| {
                    palette.items.iter().any(|entry| {
                        entry
                            .field("Name")
                            .and_then(|name| name.string().ok())
                            .is_some_and(|name| {
                                is_block_visual_name(&name) || declarative.contains(&name)
                            })
                    })
                })
                || (!numeric.is_empty()
                    && section.field("Blocks").is_some_and(|blocks| {
                        blocks.raw.get(4..).is_some_and(|bytes| {
                            bytes.iter().any(|byte| numeric_low_bytes[*byte as usize])
                        })
                    }))
        })
    }) || ["block_entities", "TileEntities"].iter().any(|name| {
        container.field(name).is_some_and(|list| {
            list.items.iter().any(|entry| {
                entry
                    .field("id")
                    .and_then(|id| id.string().ok())
                    .is_some_and(|id| is_block_visual_id(&id) || !declarative.is_empty())
            })
        })
    });
    let decoded =
        possible.then(|| crate::sections::decode_sections(raw).map_err(|e| e.to_string()));
    for (modern, legacy, kind) in [
        ("block_entities", "TileEntities", Kind::BlockEntity),
        ("block_ticks", "TileTicks", Kind::BlockTick),
        ("fluid_ticks", "LiquidTicks", Kind::FluidTick),
    ] {
        if container.field(modern).is_some() && container.field(legacy).is_some() {
            return Err(invalid(format!(
                "位置图层同时存在 {modern} 和 {legacy}，来源含糊"
            )));
        }
        let name = if container.field(modern).is_some() {
            modern
        } else {
            legacy
        };
        positioned(
            container,
            name,
            kind,
            chunk,
            "terrain-chunk",
            out,
            decoded.as_ref(),
            numeric,
            declarative,
            display,
        )?;
    }
    if let Some(decoded) = decoded {
        match decoded {
            Ok(decoded) => missing_block_visual_markers(
                raw,
                chunk,
                &decoded,
                numeric,
                declarative,
                display,
                out,
            )?,
            Err(error) => out
                .diagnostics
                .push(format!("动态方块源区块无法解码：{error}")),
        }
    }
    Ok(())
}
/// 缺少 BE 的失败标记直接指向真实调色板格，不构造不存在的 BE NBT。
fn missing_block_visual_markers(
    raw: &[u8],
    chunk: (i32, i32),
    decoded: &crate::sections::Chunk,
    numeric: &BTreeSet<(u16, u8)>,
    declarative: &BTreeSet<String>,
    display: Option<&crate::editing::Session>,
    out: &mut ChunkAnnotations,
) -> Result<(), WorldError> {
    let present = out
        .markers
        .iter()
        .filter(|marker| marker.kind == Kind::BlockEntity)
        .map(|marker| marker.position.map(|v| v as i32))
        .collect::<BTreeSet<_>>();
    let mut stamp = None;
    let base_x = chunk
        .0
        .checked_mul(16)
        .ok_or_else(|| invalid("动态方块源区块 X 溢出"))?;
    let base_z = chunk
        .1
        .checked_mul(16)
        .ok_or_else(|| invalid("动态方块源区块 Z 溢出"))?;
    for section in &decoded.sections {
        let candidates = section
            .palette
            .iter()
            .map(|state| block_visual_state(state, numeric) || declarative.contains(&state.name))
            .collect::<Vec<_>>();
        if !candidates.contains(&true) {
            continue;
        }
        for (cell, &palette) in section.indices.iter().enumerate() {
            if !candidates[palette as usize] {
                continue;
            }
            let position = [
                base_x + (cell % 16) as i32,
                i32::from(section.y) * 16 + (cell / 256) as i32,
                base_z + (cell / 16 % 16) as i32,
            ];
            if present.contains(&position) {
                continue;
            }
            // 暂存删除 / 改为普通方块后，原调色板不能继续留下悬空失败表示。
            if display.is_some_and(|session| {
                session
                    .display_change((position[0], position[1], position[2]))
                    .is_some()
            }) {
                continue;
            }
            let reason = format!(
                "动态方块 {} 在 {position:?} 缺少方块实体载荷",
                section.palette[palette as usize].name
            );
            out.diagnostics.push(reason.clone());
            out.markers.push(Marker {
                kind: Kind::BlockEntity,
                id: None,
                position: position.map(f64::from),
                path: format!("section[Y={}]/block_states/cell[{cell}]", section.y),
                source: "terrain-palette:missing-block-entity".into(),
                chunk,
                sha256: stamp.get_or_insert_with(|| digest(raw)).clone(),
                fields: 0,
                bytes: raw.len(),
                tick: None,
                priority: None,
                visual: None,
                block_state: Some(std::sync::Arc::new(
                    section.palette[palette as usize].clone(),
                )),
                visual_error: Some(reason),
                block_visual: None,
                geometry: None,
            });
        }
    }
    Ok(())
}
fn staged_block_visual_markers(
    session: &crate::editing::Session,
    chunk: (i32, i32),
    numeric: &BTreeSet<(u16, u8)>,
    declarative: &BTreeSet<String>,
    out: &mut ChunkAnnotations,
) {
    let present = out
        .markers
        .iter()
        .filter(|marker| marker.kind == Kind::BlockEntity)
        .map(|marker| marker.position.map(|v| v as i32))
        .collect::<BTreeSet<_>>();
    for ((x, y, z), state) in session.display_changes_in_chunk(chunk.0, chunk.1) {
        if !(block_visual_state(state, numeric) || declarative.contains(&state.name))
            || present.contains(&[x, y, z])
        {
            continue;
        }
        let reason = format!(
            "暂存动态方块 {} 在 {:?} 缺少方块实体载荷",
            state.name,
            [x, y, z]
        );
        let source = crate::editing::render_state(state);
        out.diagnostics.push(reason.clone());
        out.markers.push(Marker {
            kind: Kind::BlockEntity,
            id: None,
            position: [x, y, z].map(f64::from),
            path: format!("block-overlay/{x},{y},{z}"),
            source: "session:block-overlay:missing-block-entity".into(),
            chunk,
            sha256: digest(source.as_bytes()),
            fields: state.properties.len() + 1,
            bytes: source.len(),
            tick: None,
            priority: None,
            visual: None,
            block_state: Some(std::sync::Arc::new(state.clone())),
            visual_error: Some(reason),
            block_visual: None,
            geometry: None,
        });
    }
}
/// 一个后台批次复用地形 Region 头；所有标记均来自实际原始 NBT。
pub struct Reader<'a> {
    world: &'a WorldSummary,
    dimension: &'a str,
    terrain: ReadOnlyRegionCache<'a>,
    geometry: crate::chunk_display::GeometryCache,
    spawn: crate::chunk_display::SpawnCache,
    numeric_visuals: BTreeSet<(u16, u8)>,
    declarative_visuals: BTreeSet<String>,
    display: Option<&'a crate::editing::Session>,
}
impl<'a> Reader<'a> {
    pub fn new(world: &'a WorldSummary, dimension: &'a str) -> Result<Self, WorldError> {
        Ok(Self {
            world,
            dimension,
            terrain: ReadOnlyRegionCache::new(world, dimension)?,
            geometry: None,
            spawn: None,
            numeric_visuals: BTreeSet::new(),
            declarative_visuals: BTreeSet::new(),
            display: None,
        })
    }
    /// 仅接收已绑定客户端映射证明的数字动态方块状态，不假设固定数字 ID。
    pub fn set_block_visual_numeric_states(&mut self, states: BTreeSet<(u16, u8)>) {
        self.numeric_visuals = states;
    }
    /// 来自当前资源 / 插件定义的真实命名状态；用于同代源读取及缺失 BE 诊断。
    pub fn set_declarative_block_states(&mut self, states: BTreeSet<String>) {
        self.declarative_visuals = states;
    }
    /// 外部玩家目录沿同一 Reader 捕获水域与转化外观；显示固定全亮度。
    pub fn capture_appearance(
        &mut self,
        annotations: &mut ChunkAnnotations,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), WorldError> {
        light::attach(self, annotations, cancelled)
    }
    /// 与已经核对的暂存 BE 载荷共享同一 Session 代次，解析前应用真实方块修改。
    pub fn set_display_session(
        &mut self,
        session: &'a crate::editing::Session,
    ) -> Result<(), WorldError> {
        if session.original_world().root != self.world.root
            || session.original_world().data_version != self.world.data_version
            || session.dimension() != self.dimension
        {
            return Err(invalid(
                "位置外观读取与显示 Session 不属于同一世界 / 维度 / 版本",
            ));
        }
        self.display = Some(session);
        Ok(())
    }
    pub fn read(&mut self, cx: i32, cz: i32) -> Result<ChunkAnnotations, WorldError> {
        self.read_with_entity_overlay(cx, cz, None)
    }
    /// 使用同一 Session 代次的实体载荷，同时保留地形中的方块实体与计划刻。
    pub fn read_with_entity_overlay(
        &mut self,
        cx: i32,
        cz: i32,
        overlay: Option<&[u8]>,
    ) -> Result<ChunkAnnotations, WorldError> {
        self.read_with_overlays(cx, cz, overlay, None)
    }
    pub fn read_with_overlays(
        &mut self,
        cx: i32,
        cz: i32,
        entity_overlay: Option<&[u8]>,
        be_overlay: Option<&[u8]>,
    ) -> Result<ChunkAnnotations, WorldError> {
        self.read_with_layers(
            crate::chunk_display::Request {
                chunk: (cx, cz),
                layers: 0,
                heightmap: Default::default(),
            },
            entity_overlay,
            be_overlay,
        )
    }
    /// 诊断图层始终读取已保存的地形元数据；暂存实体与 BE 的显示仍使用传入代次。
    pub fn read_with_layers(
        &mut self,
        request: crate::chunk_display::Request,
        entity_overlay: Option<&[u8]>,
        be_overlay: Option<&[u8]>,
    ) -> Result<ChunkAnnotations, WorldError> {
        self.read_with_layers_filtered(request, entity_overlay, be_overlay, true)
    }
    /// 方块实体随地形显示时不必读取实体 Region；原有图层调用保持完整读取语义。
    pub fn read_with_layers_filtered(
        &mut self,
        request: crate::chunk_display::Request,
        entity_overlay: Option<&[u8]>,
        be_overlay: Option<&[u8]>,
        include_entities: bool,
    ) -> Result<ChunkAnnotations, WorldError> {
        self.read_with_layers_filtered_cancellable(
            request,
            entity_overlay,
            be_overlay,
            include_entities,
            &|| false,
        )
    }
    pub fn read_with_layers_filtered_cancellable(
        &mut self,
        request: crate::chunk_display::Request,
        entity_overlay: Option<&[u8]>,
        be_overlay: Option<&[u8]>,
        include_entities: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<ChunkAnnotations, WorldError> {
        let (cx, cz) = request.chunk;
        let mut out = ChunkAnnotations::default();
        let terrain_raw = self.terrain.read_chunk_bytes(cx, cz)?;
        if let Some(raw) = be_overlay.or(terrain_raw.as_deref()) {
            terrain(
                raw,
                (cx, cz),
                &mut out,
                &self.numeric_visuals,
                &self.declarative_visuals,
                self.display,
            )?;
        }
        if let Some(session) = self.display {
            staged_block_visual_markers(
                session,
                (cx, cz),
                &self.numeric_visuals,
                &self.declarative_visuals,
                &mut out,
            );
        }
        if let Some(raw) = &terrain_raw {
            crate::chunk_display::append(
                raw,
                self.world,
                self.dimension,
                request,
                &mut self.geometry,
                &mut self.spawn,
                &mut out,
            );
        }
        if !include_entities {
            light::attach(self, &mut out, cancelled)?;
            return Ok(out);
        }
        let storage = entity_catalog::storage(self.world, self.dimension, cx, cz)?;
        let catalog = match (storage, entity_overlay) {
            (entity_catalog::Storage::Separate(Some((label, _))), Some(raw)) => {
                Some(entity_catalog::parse(raw, &label, cx, cz)?)
            }
            (entity_catalog::Storage::Separate(None), Some(raw)) => {
                let root = parse_root(raw).map_err(invalid)?;
                let label = if root.field("Position").is_some() {
                    "entity-region:staged-missing"
                } else {
                    "legacy-main-chunk"
                };
                Some(entity_catalog::parse(raw, label, cx, cz)?)
            }
            (entity_catalog::Storage::Embedded, Some(raw)) => {
                let root = parse_root(raw).map_err(invalid)?;
                let label = if root.field("Position").is_some() {
                    "entity-region:staged-missing"
                } else {
                    "legacy-main-chunk"
                };
                Some(entity_catalog::parse(raw, label, cx, cz)?)
            }
            (entity_catalog::Storage::Separate(Some((label, raw))), None) => {
                Some(entity_catalog::parse(&raw, &label, cx, cz)?)
            }
            (entity_catalog::Storage::Separate(None), None) => terrain_raw
                .as_ref()
                .map(|raw| entity_catalog::parse(raw, "legacy-main-chunk", cx, cz))
                .transpose()?,
            (entity_catalog::Storage::Embedded, None) => terrain_raw
                .as_ref()
                .map(|raw| entity_catalog::parse(raw, "legacy-main-chunk", cx, cz))
                .transpose()?,
        };
        if let Some(catalog) = catalog {
            for entity in catalog.entities {
                let Some(mut position) = entity.pos else {
                    out.diagnostics.push(format!(
                        "区块 ({cx},{cz}) 的 {} 缺少 Pos，未生成位置标记",
                        entity.path
                    ));
                    continue;
                };
                let kind = if entity.id.as_deref().is_some_and(|id| {
                    entity_catalog::is_dropped(id)
                        || entity_catalog::is_experience_orb(id)
                        || crate::painting::is_painting(id)
                        || crate::hanging::is_frame(id)
                }) {
                    Kind::Item
                } else {
                    Kind::Entity
                };
                let visual = match entity.visual {
                    Ok(visual) => Some(std::sync::Arc::new(visual)),
                    Err(error) => {
                        out.diagnostics
                            .push(format!("实体 {} 外观数据无效：{error}", entity.path));
                        None
                    }
                };
                let mut geometry = None;
                let mut visual_error = None;
                if entity
                    .id
                    .as_deref()
                    .is_some_and(crate::painting::is_painting)
                {
                    let painting = visual
                        .as_deref()
                        .ok_or_else(|| "挂画缺少可读源载荷".to_string())
                        .and_then(|visual| {
                            let catalog = self.world.world_data.painting_catalog(
                                visual.source_version.ok_or("挂画缺少源 DataVersion")?,
                            )?;
                            crate::painting::Source::read(visual, &catalog)
                        });
                    match painting {
                        Ok(painting) => {
                            position = painting.center();
                            let (mut minimum, mut maximum) = painting.bounds();
                            for i in [0, 2] {
                                minimum[i] -= 0.5;
                                maximum[i] -= 0.5;
                            }
                            geometry = Some(Geometry::Box { minimum, maximum });
                        }
                        Err(error) => {
                            out.diagnostics
                                .push(format!("挂画 {}：{error}", entity.path));
                            visual_error = Some(error);
                        }
                    }
                }
                if entity.id.as_deref().is_some_and(crate::hanging::is_frame) {
                    let frame = visual
                        .as_deref()
                        .ok_or_else(|| "展示框缺少可读源载荷".to_string())
                        .and_then(|visual| {
                            let source = crate::hanging::Frame::read(visual)?;
                            let item = visual.source_field_optional("Item")?;
                            let is_map = item.as_ref().is_some_and(|item| {
                                crate::hanging::is_map_item(
                                    item,
                                    visual.source_version.expect("展示框来源已核实"),
                                )
                            });
                            Ok((source, is_map))
                        });
                    match frame {
                        Ok((frame, is_map)) => {
                            position = frame.center();
                            let (mut minimum, mut maximum) = frame.render_bounds(is_map);
                            for i in [0, 2] {
                                minimum[i] -= 0.5;
                                maximum[i] -= 0.5;
                            }
                            geometry = Some(Geometry::Box { minimum, maximum });
                        }
                        Err(error) => {
                            out.diagnostics
                                .push(format!("展示框 {}：{error}", entity.path));
                            visual_error = Some(error);
                        }
                    }
                }
                out.markers.push(Marker {
                    kind,
                    id: entity.id,
                    position,
                    path: entity.path,
                    source: catalog.source.clone(),
                    chunk: (cx, cz),
                    sha256: entity.sha256,
                    fields: entity.fields,
                    bytes: entity.bytes,
                    tick: None,
                    priority: None,
                    visual,
                    block_state: None,
                    visual_error,
                    block_visual: None,
                    geometry,
                });
            }
        }
        light::attach(self, &mut out, cancelled)?;
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    if category == crate::test_categories::Category::Geometry {
        tests::read_locations_across_storage_and_nested_entities();
        tests::saved_generation_and_spawn_layers();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt_document::{Field, List, Value};
    use std::{fs, path::PathBuf};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mcedit2-location-flow-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                eprintln!("位置图层测试清理失败：{error}");
            }
        }
    }
    fn compound(fields: Vec<(&str, Value)>) -> Value {
        Value::Compound(
            fields
                .into_iter()
                .map(|(name, value)| Field {
                    name: name.into(),
                    value,
                })
                .collect(),
        )
    }
    fn list(items: Vec<Value>) -> Value {
        Value::List(List {
            element_type: 10,
            items,
        })
    }
    fn bytes(root: Value) -> Vec<u8> {
        let mut out = vec![10, 0, 0];
        out.extend(root.payload().unwrap());
        out
    }
    fn entity(id: &str, passenger: Option<Value>) -> Value {
        let mut fields = vec![
            ("id", Value::String(id.into())),
            (
                "Rotation",
                Value::List(List {
                    element_type: 5,
                    items: vec![Value::Float(37.25), Value::Float(-12.5)],
                }),
            ),
            ("Age", Value::Int(-7)),
            ("IsBaby", Value::Byte(1)),
            (
                "attributes",
                list(vec![compound(vec![
                    ("id", Value::String("minecraft:generic.scale".into())),
                    ("base", Value::Double(1.5)),
                ])]),
            ),
            (
                "VillagerData",
                compound(vec![(
                    "profession",
                    Value::String("minecraft:librarian".into()),
                )]),
            ),
            (
                "Pos",
                Value::List(List {
                    element_type: 6,
                    items: vec![
                        Value::Double(-8.25),
                        Value::Double(65.),
                        Value::Double(-20.25),
                    ],
                }),
            ),
        ];
        if let Some(passenger) = passenger {
            fields.push(("Passengers", list(vec![passenger])));
        }
        compound(fields)
    }
    fn positioned(id: &str) -> Value {
        compound(vec![
            ("id", Value::String(id.into())),
            ("x", Value::Int(-9)),
            ("y", Value::Int(64)),
            ("z", Value::Int(-20)),
        ])
    }
    fn tick(id: Value) -> Value {
        compound(vec![
            ("i", id),
            ("x", Value::Int(-8)),
            ("y", Value::Int(64)),
            ("z", Value::Int(-20)),
            ("t", Value::Int(40)),
            ("p", Value::Int(-1)),
        ])
    }
    fn region(path: &std::path::Path, raw: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let count = (raw.len() + 5).div_ceil(4096);
        let mut image = vec![0u8; (2 + count) * 4096];
        let slot = (30 * 32 + 31) * 4;
        image[slot..slot + 4].copy_from_slice(&(2u32 << 8 | count as u32).to_be_bytes());
        image[8192..8196].copy_from_slice(&((raw.len() + 1) as u32).to_be_bytes());
        image[8196] = 3;
        image[8197..8197 + raw.len()].copy_from_slice(raw);
        fs::write(path, image).unwrap();
    }
    pub(super) fn saved_generation_and_spawn_layers() {
        use crate::nbt_document::{patch, Operation};
        let fixture = Fixture::new();
        let root = fixture.0.join("save");
        fs::create_dir(&root).unwrap();
        let terrain_path = root.join("region/r.-1.-1.mca");
        let world = WorldSummary {
            world_data: Default::default(),
            seed: Default::default(),
            root: root.clone(),
            level_name: None,
            data_version: Some(1343),
            storage_format: Some(19133),
            version_name: None,
            warnings: Vec::new(),
            dimensions: vec![crate::DimensionSummary {
                name: "minecraft:overworld".into(),
                region_dir: root.join("region"),
                regions: 1,
                chunks: 1,
                extension: "mca".into(),
            }],
        };
        let mut blocks = vec![0_i8; 4096];
        blocks[..256].fill(1);
        blocks[3840..].fill(1);
        for (i, id) in [(0, 7), (1, 166), (2, 8), (3, 20), (9, 152)] {
            blocks[i] = id as u8 as i8;
        }
        blocks[3840] = 7;
        blocks[3841] = 166_u8 as i8;
        blocks[256 + 4] = 8;
        blocks[512 + 5] = 1;
        blocks[256 + 8] = 20;
        let mut block_light = vec![0_i8; 2048];
        block_light[(256 + 6) / 2] = 0x78;
        let mut sky = vec![0_i8; 2048];
        sky[(256 + 128) / 2..(256 + 256) / 2].fill(-1);
        let section = |y, blocks, block_light, sky| {
            compound(vec![
                ("Y", Value::Byte(y)),
                ("Blocks", Value::ByteArray(blocks)),
                ("Data", Value::ByteArray(vec![0; 2048])),
                ("BlockLight", Value::ByteArray(block_light)),
                ("SkyLight", Value::ByteArray(sky)),
            ])
        };
        let mut upper = vec![0_i8; 4096];
        upper[3840..].fill(1);
        let mut top = vec![0_i8; 4096];
        top[3584..3840].fill(1);
        let raw = bytes(compound(vec![
            ("DataVersion", Value::Int(1343)),
            (
                "Level",
                compound(vec![
                    ("xPos", Value::Int(-1)),
                    ("zPos", Value::Int(-2)),
                    ("TerrainPopulated", Value::Byte(0)),
                    ("HeightMap", Value::IntArray(vec![1; 256])),
                    (
                        "Sections",
                        list(vec![
                            section(0, blocks, block_light, sky),
                            section(1, upper, vec![0; 2048], vec![0; 2048]),
                            section(15, top, vec![0; 2048], vec![0; 2048]),
                        ]),
                    ),
                ]),
            ),
        ]));
        let request = |layers| crate::chunk_display::Request {
            chunk: (-1, -2),
            layers,
            heightmap: Default::default(),
        };
        let read = |raw: &[u8], layers| {
            region(&terrain_path, raw);
            let before = fs::read(&terrain_path).unwrap();
            let result = Reader::new(&world, "minecraft:overworld")
                .unwrap()
                .read_with_layers_filtered(request(layers), None, None, false)
                .unwrap();
            assert_eq!(fs::read(&terrain_path).unwrap(), before);
            result
        };
        let layers = crate::chunk_display::MASK;
        let all = read(&raw, layers);
        assert!(all.diagnostics.is_empty(), "{:?}", all.diagnostics);
        let covers = |kind, x, y, z| {
            all.markers.iter().any(|m| {
                if m.kind != kind {
                    return false;
                }
                let (min, max) = m.bounds();
                let p = [-16. + f64::from(x), f64::from(y) - 0.5, -32. + f64::from(z)];
                (0..3).all(|i| p[i] >= min[i] && p[i] <= max[i])
            })
        };
        for x in 0..7 {
            assert!(!covers(Kind::SpawnDark, x, 1, 0), "无效候选 {x}");
        }
        assert!(covers(Kind::SpawnDark, 7, 1, 0));
        assert!(covers(Kind::SpawnDark, 8, 1, 0));
        assert!(!covers(Kind::SpawnDark, 9, 1, 0));
        assert!(covers(Kind::SpawnNight, 15, 1, 15));
        assert!(!covers(Kind::SpawnDark, 0, 16, 0));
        assert!(!covers(Kind::SpawnDark, 1, 16, 0));
        assert!(covers(Kind::SpawnDark, 2, 16, 0));
        assert!(!covers(Kind::SpawnDark, 2, 32, 0));
        assert!(!covers(Kind::SpawnDark, 2, 240, 0));
        assert!(covers(Kind::SpawnDark, 2, 255, 0));
        let area = |kind| {
            all.markers
                .iter()
                .filter(|m| m.kind == kind)
                .map(|m| {
                    let (a, b) = m.bounds();
                    (b[0] - a[0]) * (b[2] - a[2])
                })
                .sum::<f64>()
        };
        assert!(covers(Kind::SpawnDark, 5, 3, 0));
        assert_eq!(area(Kind::SpawnDark), 631.);
        assert_eq!(area(Kind::SpawnNight), 128.);
        assert_eq!(
            all.markers
                .iter()
                .filter(|m| m.kind == Kind::ChunkSection)
                .count(),
            3
        );
        let column = all
            .markers
            .iter()
            .find(|m| m.kind == Kind::TerrainIncomplete)
            .unwrap();
        assert_eq!(
            column.bounds(),
            ([-16.5, -0.5, -32.5], [-0.5, 255.5, -16.5])
        );
        assert_eq!(column.path, "Level/TerrainPopulated");
        let night = read(&raw, Kind::SpawnNight.bit());
        assert_eq!(night.markers.len(), 1, "平整连续夜间候选应合为一个准确面");
        assert!(night.markers.iter().all(|m| m.kind == Kind::SpawnNight));
        let overlay = crate::nbt_document::patch_batch(
            &raw,
            &[
                Operation::Replace {
                    path: "/Level/TerrainPopulated".into(),
                    value: Value::Byte(1),
                },
                Operation::Replace {
                    path: "/Level/Sections/0/BlockLight".into(),
                    value: Value::ByteArray(vec![-1; 2048]),
                },
            ],
        )
        .unwrap();
        region(&terrain_path, &raw);
        let staged = Reader::new(&world, "minecraft:overworld")
            .unwrap()
            .read_with_layers_filtered(request(layers), None, Some(&overlay), false)
            .unwrap();
        assert_eq!(
            staged.markers, all.markers,
            "源图层不得采用 BE / 暂存覆盖中的光照和状态"
        );
        assert!(read(&overlay, Kind::TerrainIncomplete.bit())
            .markers
            .is_empty());
        for operation in [
            Operation::Replace {
                path: "/DataVersion".into(),
                value: Value::Int(1631),
            },
            Operation::Replace {
                path: "/Level/Sections/0/SkyLight".into(),
                value: Value::ByteArray(vec![0; 1]),
            },
            Operation::Replace {
                path: "/Level/Sections/0/Blocks/0".into(),
                value: Value::Byte(27),
            },
        ] {
            let changed = patch(&raw, &operation).unwrap();
            // 非法铁轨 metadata 在独立的真实客户端拒绝路径下核验。
            let changed = if matches!(operation, Operation::Replace { ref path, .. } if path.ends_with("Blocks/0"))
            {
                patch(
                    &changed,
                    &Operation::Replace {
                        path: "/Level/Sections/0/Data/0".into(),
                        value: Value::Byte(6),
                    },
                )
                .unwrap()
            } else {
                changed
            };
            let result = read(
                &changed,
                Kind::SpawnDark.bit() | Kind::TerrainIncomplete.bit(),
            );
            assert_eq!(result.diagnostics.len(), 1);
            assert!(result
                .markers
                .iter()
                .all(|m| m.kind == Kind::TerrainIncomplete));
        }
        // MCRegion 的整柱高度取实际 128 格数组，状态完成后不生成橙色标记。
        let mcr = bytes(compound(vec![(
            "Level",
            compound(vec![
                ("xPos", Value::Int(-1)),
                ("zPos", Value::Int(-2)),
                ("TerrainPopulated", Value::Byte(0)),
                ("Blocks", Value::ByteArray(vec![0; 32768])),
            ]),
        )]));
        assert_eq!(
            read(&mcr, Kind::TerrainIncomplete.bit()).markers[0]
                .bounds()
                .1[1],
            127.5
        );
        // 现代 Status 使用真实 level.dat 的内联维度高度，不按已有 sections 猜测全高。
        let level = bytes(compound(vec![(
            "Data",
            compound(vec![
                ("DataVersion", Value::Int(3955)),
                (
                    "WorldGenSettings",
                    compound(vec![(
                        "dimensions",
                        compound(vec![(
                            "minecraft:overworld",
                            compound(vec![(
                                "type",
                                compound(vec![
                                    ("min_y", Value::Int(-128)),
                                    ("height", Value::Int(1024)),
                                    ("has_skylight", Value::Byte(1)),
                                ]),
                            )]),
                        )]),
                    )]),
                ),
            ]),
        )]));
        fs::write(root.join("level.dat"), level).unwrap();
        let mut modern_world = world.clone();
        modern_world.data_version = Some(3955);
        let modern = |status| {
            bytes(compound(vec![
                ("DataVersion", Value::Int(3955)),
                ("xPos", Value::Int(-1)),
                ("zPos", Value::Int(-2)),
                ("yPos", Value::Int(-8)),
                ("Status", Value::String(status)),
            ]))
        };
        for status in ["minecraft:noise", "full", "minecraft:full", "test:unknown"] {
            let raw = modern(status.into());
            region(&terrain_path, &raw);
            let result = Reader::new(&modern_world, "minecraft:overworld")
                .unwrap()
                .read_with_layers_filtered(
                    request(Kind::TerrainIncomplete.bit()),
                    None,
                    None,
                    false,
                )
                .unwrap();
            match status {
                "minecraft:noise" => assert_eq!(
                    result.markers[0].bounds(),
                    ([-16.5, -128.5, -32.5], [-0.5, 895.5, -16.5])
                ),
                "test:unknown" => assert_eq!(result.diagnostics.len(), 1),
                _ => assert!(result.markers.is_empty() && result.diagnostics.is_empty()),
            }
        }
    }
    pub(super) fn read_locations_across_storage_and_nested_entities() {
        let fixture = Fixture::new();
        let root = fixture.0.join("save");
        fs::create_dir(&root).unwrap();
        let terrain_path = root.join("region/r.-1.-1.mca");
        let entity_path = root.join("entities/r.-1.-1.mca");
        let world = WorldSummary {
            world_data: Default::default(),
            seed: Default::default(),
            root: root.clone(),
            level_name: None,
            data_version: Some(3955),
            storage_format: Some(19133),
            version_name: None,
            warnings: Vec::new(),
            dimensions: vec![crate::DimensionSummary {
                name: "minecraft:overworld".into(),
                region_dir: root.join("region"),
                regions: 1,
                chunks: 1,
                extension: "mca".into(),
            }],
        };
        let modern = bytes(compound(vec![
            ("DataVersion", Value::Int(3955)),
            ("sections", list(vec![])),
            ("xPos", Value::Int(-1)),
            ("zPos", Value::Int(-2)),
            ("block_entities", list(vec![positioned("minecraft:chest")])),
            (
                "block_ticks",
                list(vec![tick(Value::String("minecraft:oak_sapling".into()))]),
            ),
            (
                "fluid_ticks",
                list(vec![tick(Value::String("minecraft:water".into()))]),
            ),
        ]));
        region(&terrain_path, &modern);
        let chicken = compound(vec![
            ("id", Value::String("minecraft:chicken".into())),
            (
                "Pos",
                Value::List(List {
                    element_type: 6,
                    items: vec![
                        Value::Double(-8.25),
                        Value::Double(65.),
                        Value::Double(-20.25),
                    ],
                }),
            ),
            ("Riding", entity("minecraft:boat", None)),
        ]);
        let mut invalid_visual = entity("example:unknown_actor", None);
        if let Value::Compound(fields) = &mut invalid_visual {
            fields
                .iter_mut()
                .find(|f| f.name == "Rotation")
                .unwrap()
                .value = Value::List(List {
                element_type: 6,
                items: vec![Value::Double(0.), Value::Double(0.)],
            });
        }
        let entities = bytes(compound(vec![
            ("DataVersion", Value::Int(3955)),
            ("Position", Value::IntArray(vec![-1, -2])),
            (
                "Entities",
                list(vec![
                    entity("minecraft:pig", Some(chicken)),
                    entity("minecraft:item", None),
                    invalid_visual,
                    compound(vec![("id", Value::String("minecraft:zombie".into()))]),
                ]),
            ),
        ]));
        region(&entity_path, &entities);
        let source_before = [
            fs::read(&terrain_path).unwrap(),
            fs::read(&entity_path).unwrap(),
        ];
        let mut reader = Reader::new(&world, "minecraft:overworld").unwrap();
        let annotations = reader.read(-1, -2).unwrap();
        assert_eq!(annotations.markers.len(), 8);
        assert_eq!(annotations.diagnostics.len(), 2);
        let pig = annotations
            .markers
            .iter()
            .find(|m| m.id.as_deref() == Some("minecraft:pig"))
            .unwrap();
        assert_eq!(pig.position, [-8.25, 65., -20.25]);
        let visual = pig.visual.as_ref().unwrap();
        assert_eq!(visual.rotation, Some([37.25, -12.5]));
        assert_eq!(visual.appearance["Age"], Value::Int(-7));
        assert_eq!(visual.appearance["IsBaby"], Value::Byte(1));
        assert_eq!(
            visual.appearance["attributes"],
            list(vec![compound(vec![
                ("id", Value::String("minecraft:generic.scale".into())),
                ("base", Value::Double(1.5)),
            ])])
        );
        assert_eq!(
            visual.appearance["VillagerData"],
            compound(vec![(
                "profession",
                Value::String("minecraft:librarian".into())
            )])
        );
        let damaged = annotations
            .markers
            .iter()
            .find(|m| m.id.as_deref() == Some("example:unknown_actor"))
            .unwrap();
        assert!(damaged.visual.is_none());
        assert_eq!(damaged.position, pig.position);
        assert_eq!(pig.bounds(), ([-9.25, 64.5, -21.25], [-8.25, 65.5, -20.25]));
        assert!(annotations
            .markers
            .iter()
            .any(|m| m.path == "Entities/0/Passengers/0/Riding"
                && m.id.as_deref() == Some("minecraft:boat")));
        assert!(annotations
            .markers
            .iter()
            .any(|m| m.kind == Kind::Item && m.id.as_deref() == Some("minecraft:item")));
        let chest = annotations
            .markers
            .iter()
            .find(|m| m.kind == Kind::BlockEntity)
            .unwrap();
        assert_eq!(chest.bounds(), ([-9.5, 63.5, -20.5], [-8.5, 64.5, -19.5]));
        assert_eq!(
            annotations
                .markers
                .iter()
                .filter(|m| m.tick == Some(40) && m.priority == Some(-1))
                .count(),
            2
        );
        assert!(reader.read(0, 0).unwrap().markers.is_empty());
        let snapshot = fixture.0.join("entities.mcedit2entities");
        entity_catalog::export(&world, "minecraft:overworld", -1, -2, &snapshot).unwrap();
        assert_eq!(
            entity_catalog::verify(&world, "minecraft:overworld", &snapshot).unwrap(),
            entities.len()
        );
        assert_eq!(
            entity_catalog::inspect(&world, "minecraft:overworld", -1, -2)
                .unwrap()
                .entities
                .len(),
            6
        );
        assert_eq!(fs::read(&terrain_path).unwrap(), source_before[0]);
        assert_eq!(fs::read(&entity_path).unwrap(), source_before[1]);
        // 现代独立实体流同样沿用 Region 对外部 .mcc 的准确坐标读取。
        let mut stub = source_before[1].clone();
        stub[8192..8196].copy_from_slice(&1u32.to_be_bytes());
        stub[8196] = 0x83;
        fs::write(&entity_path, &stub).unwrap();
        fs::write(root.join("entities/c.-1.-2.mcc"), &entities).unwrap();
        assert_eq!(
            Reader::new(&world, "minecraft:overworld")
                .unwrap()
                .read(-1, -2)
                .unwrap()
                .markers,
            annotations.markers
        );
        // 切换为真实旧式存储布局，实体和数字计划更新都取自 Level。
        let legacy = bytes(compound(vec![(
            "Level",
            compound(vec![
                ("Sections", list(vec![])),
                ("xPos", Value::Int(-1)),
                ("zPos", Value::Int(-2)),
                ("TileEntities", list(vec![positioned("Chest")])),
                ("TileTicks", list(vec![tick(Value::Int(12))])),
                ("Entities", list(vec![entity("Item", None)])),
            ]),
        )]));
        fs::remove_file(&entity_path).unwrap();
        region(&terrain_path, &legacy);
        let old = Reader::new(&world, "minecraft:overworld")
            .unwrap()
            .read(-1, -2)
            .unwrap();
        assert_eq!(old.markers.len(), 3);
        assert!(old
            .markers
            .iter()
            .any(|m| m.kind == Kind::BlockTick && m.id.as_deref() == Some("12")));
        assert_eq!(
            old.markers.iter().filter(|m| m.kind == Kind::Item).count(),
            1
        );
        let wrong_position = bytes(compound(vec![
            ("Position", Value::IntArray(vec![0, 0])),
            ("Entities", list(vec![entity("minecraft:pig", None)])),
        ]));
        region(&entity_path, &wrong_position);
        assert!(Reader::new(&world, "minecraft:overworld")
            .unwrap()
            .read(-1, -2)
            .is_err());
        let malformed = bytes(compound(vec![
            ("xPos", Value::Int(-1)),
            ("zPos", Value::Int(-2)),
            ("block_entities", Value::String("damaged".into())),
        ]));
        region(&terrain_path, &malformed);
        assert!(Reader::new(&world, "minecraft:overworld")
            .unwrap()
            .read(-1, -2)
            .is_err());
        // 原始旧文件在整个读取流程中保持字节不变。
        assert_eq!(
            fs::read(&terrain_path).unwrap()[8197..8197 + malformed.len()],
            malformed
        );
    }
}
