//! 位置图层后台批次、不可变快照和只读选择；与地形建模独立取消。
use crate::{language::Language, App, Message};
use iced::Task;
use mcedit2_world::{
    editing::{Bounds, Session, Slice, VerticalSlice},
    scene_annotations::{ChunkAnnotations, Kind, LayerMask, Marker},
    surface::SurfaceView,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
#[path = "scene_layer_boundary.rs"]
mod boundary;
#[path = "scene_layer_overlay.rs"]
mod overlay;
pub(crate) use overlay::{segment, Overlay, ProjectionSource};
#[path = "scene_layer_planar.rs"]
mod planar;
pub(crate) use planar::{Layout as PlanarLayout, MapState, View};
type Coordinate = (i32, i32);

#[path = "scene_layer_preferences.rs"]
pub(crate) mod preferences;

#[derive(Clone, Debug)]
pub(crate) struct MarkerRef {
    pub chunk: Arc<ChunkAnnotations>,
    pub index: usize,
}
impl MarkerRef {
    pub fn marker(&self) -> &Marker {
        &self.chunk.markers[self.index]
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.chunk, &other.chunk) && self.index == other.index
            || self.marker() == other.marker() && self.chunk.lighting == other.chunk.lighting
    }
}
#[derive(Debug)]
pub(crate) struct Snapshot {
    pub key: (usize, usize),
    pub epoch: u64,
    pub markers: Vec<MarkerRef>,
    pub biomes: Arc<mcedit2_world::biome_source::Region>,
    tree: overlay::Index,
    boundary: boundary::Boundary,
}
impl Snapshot {
    /// 同一不可变快照的源段，保留源顺序和身份供增量几何编译。
    pub(crate) fn subset(
        &self,
        markers: Vec<MarkerRef>,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Self> {
        let tree = overlay::Index::new(&markers, cancelled)?;
        let boundary = boundary::Boundary::build(&markers, &tree, cancelled)?;
        Some(Self {
            key: self.key,
            epoch: self.epoch,
            biomes: Arc::clone(&self.biomes),
            markers,
            tree,
            boundary,
        })
    }
    #[cfg(test)]
    pub(crate) fn build(
        key: (usize, usize),
        epoch: u64,
        parts: impl Iterator<Item = Arc<ChunkAnnotations>>,
        clip: Option<Bounds>,
    ) -> Self {
        Arc::into_inner(
            Self::build_cancellable(key, epoch, parts, clip, Arc::default(), None, &|| false)
                .expect("无取消请求的源快照构建必须完成"),
        )
        .expect("新建测试快照仅有一个持有者")
    }
    fn build_cancellable(
        key: (usize, usize),
        epoch: u64,
        parts: impl Iterator<Item = Arc<ChunkAnnotations>>,
        clip: Option<Bounds>,
        biomes: Arc<mcedit2_world::biome_source::Region>,
        previous: Option<&Arc<Self>>,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Arc<Self>> {
        let mut markers = Vec::new();
        for chunk in parts {
            if cancelled() {
                return None;
            }
            for index in 0..chunk.markers.len() {
                if index.is_multiple_of(1024) && cancelled() {
                    return None;
                }
                let point = chunk.markers[index].center();
                // 模型可越过源格，资源几何尚未编译；最终由共享四角投影精确裁剪。
                if chunk.markers[index].block_visual.is_some()
                    || chunk.markers[index].visual.is_some()
                    || clip.is_none_or(|b| {
                        if chunk.markers[index].geometry.is_some() {
                            let (min, max) = chunk.markers[index].bounds();
                            return (0..3).all(|i| {
                                max[i] >= b.minimum[i] as f64 - 0.5
                                    && min[i] <= b.maximum[i] as f64 + 0.5
                            });
                        }
                        (0..3).all(|i| {
                            point[i] >= b.minimum[i] as f64 - 0.5
                                && point[i] <= b.maximum[i] as f64 + 0.5
                        })
                    })
                {
                    markers.push(MarkerRef {
                        chunk: Arc::clone(&chunk),
                        index,
                    });
                }
            }
        }
        if cancelled() {
            return None;
        }
        // 当前代次的可见源内容没有变化时，索引、实体依赖和拾取身份一起保留。
        // 不按值忽略源替换：同值的新 NBT 仍应更新选中对象及其源证明。
        if let Some(old) = previous.filter(|old| {
            old.same_biomes(&biomes, &markers)
                && old.key == key
                && old.epoch == epoch
                && old.markers.len() == markers.len()
                && old
                    .markers
                    .iter()
                    .zip(&markers)
                    .all(|(a, b)| Arc::ptr_eq(&a.chunk, &b.chunk) && a.index == b.index)
        }) {
            return Some(Arc::clone(old));
        }
        let tree = overlay::Index::new(&markers, cancelled)?;
        let boundary = boundary::Boundary::build(&markers, &tree, cancelled)?;
        Some(Arc::new(Self {
            key,
            epoch,
            biomes,
            markers,
            tree,
            boundary,
        }))
    }
    /// 动态方块依赖真实颜色邻域；其他实体不因无关的群系列重读而重建。
    pub(crate) fn same_biomes(
        &self,
        biomes: &mcedit2_world::biome_source::Region,
        markers: &[MarkerRef],
    ) -> bool {
        markers.iter().all(|source| {
            let marker = source.marker();
            !matches!(
                marker.block_visual.as_deref(),
                Some(mcedit2_world::scene_annotations::BlockVisual::Piston(_))
            ) || self
                .biomes
                .same_neighborhood(biomes, marker.chunk.0, marker.chunk.1)
        })
    }
    /// 所有投影共用源面的裁剪；不把斜面投影退化成轴对齐屏幕矩形。
    fn surfaces(&self, index: usize, clip: Option<boundary::Clip>) -> Vec<boundary::Face> {
        let marker = self.markers[index].marker();
        let bounds = if matches!(
            marker.geometry,
            Some(mcedit2_world::scene_annotations::Geometry::Box { .. })
        ) {
            boundary::clip_volume(marker.bounds(), clip)
        } else {
            overlay::clipped_bounds(marker, clip)
        };
        let Some(bounds) = bounds else {
            return Vec::new();
        };
        if marker.kind == Kind::TerrainIncomplete {
            self.boundary.clipped(index, marker.bounds(), bounds, clip)
        } else {
            boundary::box_faces(bounds)
        }
    }
}
#[derive(Clone)]
enum Scope {
    Quad {
        key: (usize, usize),
        root: PathBuf,
        ranges: Vec<Bounds>,
        source: Arc<()>,
    },
    Planes {
        key: (usize, usize),
        root: PathBuf,
        horizontal: Option<Arc<Slice>>,
        vertical: Option<Arc<VerticalSlice>>,
    },
    Stream {
        key: (usize, usize),
        root: PathBuf,
        nonce: u64,
    },
    Surface {
        key: (usize, usize),
        root: PathBuf,
        source: Arc<SurfaceView>,
        clip: Bounds,
    },
}
impl Scope {
    fn key(&self) -> (usize, usize) {
        match self {
            Self::Stream { key, .. }
            | Self::Surface { key, .. }
            | Self::Quad { key, .. }
            | Self::Planes { key, .. } => *key,
        }
    }
    fn root(&self) -> &PathBuf {
        match self {
            Self::Stream { root, .. }
            | Self::Surface { root, .. }
            | Self::Quad { root, .. }
            | Self::Planes { root, .. } => root,
        }
    }
    fn clip(&self) -> Option<Bounds> {
        match self {
            Self::Surface { clip, .. } => Some(*clip),
            Self::Quad { .. } => None,
            _ => None,
        }
    }
    fn same(&self, next: &Self) -> bool {
        match (self, next) {
            (
                Self::Quad {
                    key: a,
                    root: ar,
                    ranges: x,
                    source: sx,
                },
                Self::Quad {
                    key: b,
                    root: br,
                    ranges: y,
                    source: sy,
                },
            ) => a == b && ar == br && x == y && Arc::ptr_eq(sx, sy),
            (
                Self::Planes {
                    key: a,
                    root: ar,
                    horizontal: ah,
                    vertical: av,
                },
                Self::Planes {
                    key: b,
                    root: br,
                    horizontal: bh,
                    vertical: bv,
                },
            ) => a == b && ar == br && same_optional(ah, bh) && same_optional(av, bv),
            (
                Self::Stream {
                    key: a,
                    root: ar,
                    nonce: x,
                },
                Self::Stream {
                    key: b,
                    root: br,
                    nonce: y,
                },
            ) => a == b && ar == br && x == y,
            (
                Self::Surface {
                    key: a,
                    root: ar,
                    source: x,
                    clip: c,
                },
                Self::Surface {
                    key: b,
                    root: br,
                    source: y,
                    clip: d,
                },
            ) => a == b && ar == br && Arc::ptr_eq(x, y) && c == d,
            _ => false,
        }
    }
    fn same_data(&self, next: &Self) -> bool {
        if self.key() != next.key() || self.root() != next.root() {
            return false;
        }
        match (self, next) {
            (Self::Stream { .. }, Self::Stream { .. }) => true,
            (Self::Surface { source: a, .. }, Self::Surface { source: b, .. }) => Arc::ptr_eq(a, b),
            (Self::Quad { source: a, .. }, Self::Quad { source: b, .. }) => Arc::ptr_eq(a, b),
            _ => self.same(next),
        }
    }
}
fn same_optional<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}
pub(crate) enum SourceRef<'a> {
    Surface(&'a Arc<SurfaceView>),
    Horizontal(&'a Arc<Slice>),
    Vertical(&'a Arc<VerticalSlice>),
    Quad(&'a Arc<()>),
}
#[derive(Default)]
pub(crate) struct State {
    pub mask: LayerMask,
    pub heightmap: mcedit2_world::chunk_display::Heightmap,
    pub inspect: bool,
    pub show_diagnostics: bool,
    pub show_controls: bool,
    pub snapshot: Option<Arc<Snapshot>>,
    pub selected: Option<MarkerRef>,
    pub command_visuals: Option<(String, Arc<mcedit2_world::command_visuals::Visuals>)>,
    scope: Option<Scope>,
    source_changed: bool,
    read_mode: Option<(bool, bool)>,
    block_visual_numeric_states: Arc<BTreeSet<(u16, u8)>>,
    declarative_block_states: Arc<BTreeSet<String>>,
    block_visual_resources: Option<crate::textures::VisibilityIdentity>,
    epoch: u64,
    running: bool,
    failed: bool,
    cancelled: Arc<AtomicBool>,
    pending: VecDeque<Coordinate>,
    cache: BTreeMap<Coordinate, Arc<ChunkAnnotations>>,
    errors: BTreeMap<Coordinate, String>,
    players: Option<Arc<ChunkAnnotations>>,
    player_records: Option<Arc<mcedit2_world::player_catalog::Catalog>>,
    pub status: String,
}
impl State {
    pub(crate) fn is_idle(&self) -> bool {
        !self.running && self.pending.is_empty()
    }
    pub fn has_plane_sources(&self, app: &App) -> bool {
        matches!(&self.scope,Some(Scope::Planes{key,..}) if Some(*key)==app.current_key())
    }
    pub fn view_for(&self, key: Option<(usize, usize)>, source: SourceRef<'_>) -> Option<View> {
        let scope = self.scope.as_ref()?;
        let snapshot = self.snapshot.as_ref()?;
        if self.mask == 0 || key != Some(snapshot.key) || snapshot.epoch != self.epoch {
            return None;
        }
        let same = match (scope, source) {
            (Scope::Surface { source: a, .. }, SourceRef::Surface(b)) => Arc::ptr_eq(a, b),
            (Scope::Quad { source: a, .. }, SourceRef::Quad(b)) => Arc::ptr_eq(a, b),
            (
                Scope::Planes {
                    horizontal: Some(a),
                    ..
                },
                SourceRef::Horizontal(b),
            ) => Arc::ptr_eq(a, b),
            (
                Scope::Planes {
                    vertical: Some(a), ..
                },
                SourceRef::Vertical(b),
            ) => Arc::ptr_eq(a, b),
            _ => false,
        };
        same.then(|| View {
            snapshot: Arc::clone(snapshot),
            mask: self.mask,
            inspect: self.inspect,
            selected: self.selected.clone(),
        })
    }
    pub fn invalidate(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.scope = None;
        self.source_changed = false;
        self.snapshot = None;
        self.selected = None;
        self.command_visuals = None;
        self.pending.clear();
        self.cache.clear();
        self.errors.clear();
        self.players = None;
        self.player_records = None;
        self.status.clear();
        self.failed = false;
    }
    /// 新会话来源只撤销发生变化的注释段，其他不可变源段继续参与模型复用。
    pub(crate) fn change_sources(&mut self, chunks: &BTreeSet<Coordinate>) {
        let affected = chunks
            .iter()
            .flat_map(|&(cx, cz)| {
                (-1..=1).flat_map(move |dx| {
                    (-1..=1).filter_map(move |dz| Some((cx.checked_add(dx)?, cz.checked_add(dz)?)))
                })
            })
            .collect::<BTreeSet<_>>();
        self.cancelled.store(true, Ordering::Release);
        self.epoch = self.epoch.wrapping_add(1);
        self.source_changed = true;
        self.cache.retain(|chunk, _| !affected.contains(chunk));
        self.errors.retain(|chunk, _| !affected.contains(chunk));
        if self
            .selected
            .as_ref()
            .is_some_and(|m| chunks.contains(&m.marker().chunk))
        {
            self.selected = None;
            self.command_visuals = None;
        }
        self.snapshot = None;
        self.pending.clear();
        self.failed = false;
    }
    fn replan(&mut self, scope: Scope, coordinates: Vec<Coordinate>) {
        if !self.source_changed && self.scope.as_ref().is_some_and(|old| old.same(&scope)) {
            return;
        }
        self.cancelled.store(true, Ordering::Release);
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.epoch = self.epoch.wrapping_add(1);
        let wanted: HashSet<_> = coordinates.iter().copied().collect();
        let same_data = self.scope.as_ref().is_some_and(|old| {
            old.same_data(&scope)
                || self.source_changed && old.key() == scope.key() && old.root() == scope.root()
        });
        self.source_changed = false;
        if !same_data {
            self.cache.clear();
            self.errors.clear();
            self.players = None;
            self.player_records = None;
            self.selected = None;
            self.command_visuals = None;
        }
        self.cache.retain(|key, _| wanted.contains(key));
        self.errors.retain(|key, _| wanted.contains(key));
        self.pending = coordinates
            .into_iter()
            .filter(|key| !self.cache.contains_key(key) && !self.errors.contains_key(key))
            .collect();
        self.snapshot = None;
        self.failed = false;
        self.status = "正在后台读取位置图层…".into();
        self.scope = Some(scope);
    }
    fn fetch(&mut self, session: Session, entities: bool, players_enabled: bool) -> Task<Message> {
        let Some(scope) = &self.scope else {
            return Task::none();
        };
        if self.running || self.failed || self.pending.is_empty() && self.snapshot.is_some() {
            return Task::none();
        }
        let requests = self.pending.drain(..self.pending.len().min(32)).collect();
        let job = Job {
            epoch: self.epoch,
            key: scope.key(),
            clip: scope.clip(),
            session,
            requests,
            retained: self
                .cache
                .iter()
                .filter(|(_, part)| !part.markers.is_empty())
                .map(|(key, part)| (*key, Arc::clone(part)))
                .collect(),
            previous: self.snapshot.clone(),
            players: self.players.clone(),
            entities,
            players_enabled,
            block_visual_numeric_states: self.block_visual_numeric_states.clone(),
            declarative_block_states: self.declarative_block_states.clone(),
            cancelled: Arc::clone(&self.cancelled),
            layers: self.mask & mcedit2_world::chunk_display::MASK,
            heightmap: self.heightmap,
        };
        self.running = true;
        Task::perform(
            async move {
                let (epoch, key) = (job.epoch, job.key);
                let (sender, receiver) = iced::futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    if sender.send(job.read().map(Arc::new)).is_err() {
                        log::debug!("[layers] 位置图层结果接收端已关闭");
                    }
                });
                (
                    epoch,
                    key,
                    receiver
                        .await
                        .unwrap_or_else(|e| Err(format!("位置图层工作线程未返回：{e}"))),
                )
            },
            |(epoch, key, result)| Message::SceneLayersLoaded(epoch, key, result),
        )
    }
    pub fn accept(&mut self, epoch: u64, key: (usize, usize), result: Result<Arc<Batch>, String>) {
        // 每次仅允许一个后台读取；即使结果过期，也须释放该工作槽。
        self.running = false;
        if self.epoch != epoch || self.scope.as_ref().is_none_or(|s| s.key() != key) {
            return;
        }
        match result {
            Ok(batch) => {
                if let Some(records) = &batch.player_catalog {
                    self.player_records = Some(Arc::clone(records));
                }
                if let Some(markers) = &batch.players {
                    self.players = Some(Arc::clone(markers));
                }
                for (coordinate, result) in &batch.records {
                    match result {
                        Ok(part) => {
                            self.cache.insert(*coordinate, Arc::clone(part));
                        }
                        Err(error) => {
                            log::error!("[layers] 区块 {coordinate:?}：{error}");
                            self.errors.insert(*coordinate, error.clone());
                        }
                    }
                }
                if let Some(selected) = &self.selected {
                    self.selected = batch
                        .snapshot
                        .markers
                        .iter()
                        .find(|m| m.same(selected))
                        .cloned();
                }
                if self.selected.as_ref().is_none_or(|selected| {
                    self.command_visuals
                        .as_ref()
                        .is_some_and(|(hash, _)| hash != &selected.marker().sha256)
                }) {
                    self.command_visuals = None;
                }
                self.snapshot = Some(Arc::clone(&batch.snapshot));
                let diagnostics: usize = self.cache.values().map(|c| c.diagnostics.len()).sum();
                self.status = format!(
                    "已读 {} 区块，{} 个位置，待读 {}，错误 {}，诊断 {}",
                    self.cache.len(),
                    batch.snapshot.markers.len(),
                    self.pending.len(),
                    self.errors.len(),
                    diagnostics
                );
            }
            Err(error) => {
                log::error!("[layers] {error}");
                self.status = error;
                self.failed = true;
                self.pending.clear();
            }
        }
    }
    pub fn select(
        &mut self,
        snapshot: &Arc<Snapshot>,
        index: usize,
        key: Option<(usize, usize)>,
    ) -> bool {
        if !self.inspect
            || key != Some(snapshot.key)
            || snapshot.epoch != self.epoch
            || self
                .snapshot
                .as_ref()
                .is_none_or(|s| !Arc::ptr_eq(s, snapshot))
        {
            return false;
        }
        let Some(marker) = snapshot
            .markers
            .get(index)
            .filter(|m| m.marker().layers() & self.mask != 0)
        else {
            return false;
        };
        self.selected = Some(marker.clone());
        self.command_visuals = None;
        true
    }
    pub fn diagnostics(&self) -> String {
        self.errors
            .iter()
            .map(|(coordinate, error)| format!("{coordinate:?}: {error}"))
            .chain(
                self.cache
                    .values()
                    .flat_map(|part| part.diagnostics.iter().cloned()),
            )
            .take(128)
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn detail(&self, language: Language) -> Option<String> {
        let marker = self.selected.as_ref()?.marker();
        let name = label(marker.kind, language);
        let player = if marker.kind == Kind::Player {
            self.player_records
                .as_ref()
                .and_then(|c| {
                    c.players.iter().find(|p| {
                        p.sources.iter().any(|s| {
                            s.sha256 == marker.sha256
                                && s.path.display().to_string() == marker.source
                        })
                    })
                })
                .map(|p| {
                    format!(
                        "\n{}: {} · UUID: {}\n{}",
                        language.pick("名称", "Name"),
                        p.name
                            .as_deref()
                            .unwrap_or(language.pick("源数据未提供", "Not present in source")),
                        p.uuid
                            .map(mcedit2_world::player_catalog::uuid_string)
                            .unwrap_or_else(|| language
                                .pick("源数据未提供", "Not present in source")
                                .into()),
                        p.sources
                            .iter()
                            .map(|s| format!(
                                "{} · {} · SHA-256 {}",
                                s.path.display(),
                                s.nbt_path,
                                s.sha256
                            ))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let orientation = marker
            .visual
            .as_ref()
            .and_then(|v| v.rotation)
            .map(|r| {
                format!(
                    "\n{}: {:.2}°, {:.2}°",
                    language.pick("偏航 / 俯仰", "Yaw / pitch"),
                    r[0],
                    r[1]
                )
            })
            .unwrap_or_default();
        Some(
            format!(
                "{name}：{} @ ({:.3}, {:.3}, {:.3})\n{} · {} · SHA-256 {}{}",
                marker
                    .id
                    .as_deref()
                    .unwrap_or(language.pick("源数据未提供 ID", "ID absent in source")),
                marker.position[0],
                marker.position[1],
                marker.position[2],
                marker.source,
                marker.path,
                marker.sha256,
                marker
                    .tick
                    .map(|t| format!(" · t={t}, p={:?}", marker.priority))
                    .unwrap_or_default()
            ) + &orientation
                + &player
                + &self
                    .command_visuals
                    .as_ref()
                    .map(|(_, v)| {
                        if v.diagnostics.is_empty() {
                            String::new()
                        } else {
                            format!("\n{}", v.diagnostics.join("\n"))
                        }
                    })
                    .unwrap_or_default(),
        )
    }
}
pub(crate) fn label(kind: Kind, l: Language) -> &'static str {
    match kind {
        Kind::Entity => l.pick("实体位置", "Entity locations"),
        Kind::Item => l.pick("物品及悬挂实体位置", "Item and hanging entity locations"),
        Kind::BlockEntity => l.pick("方块实体位置", "Block entity locations"),
        Kind::BlockTick => l.pick("方块计划更新", "Scheduled block ticks"),
        Kind::FluidTick => l.pick("流体计划更新", "Scheduled fluid ticks"),
        Kind::Player => l.pick("玩家位置", "Player locations"),
        Kind::ChunkSection => l.pick("已保存区块段边界", "Saved chunk sections"),
        Kind::Heightmap => l.pick("已保存高度图", "Saved heightmaps"),
        Kind::SpawnDark => l.pick("刷怪候选：现有低光照", "Spawn candidates: saved low light"),
        Kind::SpawnNight => l.pick("刷怪候选：需降低天空光", "Spawn candidates: sky-dependent"),
        Kind::TerrainIncomplete => l.pick("未完成地形生成", "Unpopulated terrain"),
    }
}
pub(crate) fn command_label(
    layer: mcedit2_world::scene_annotations::CommandLayer,
    l: Language,
) -> &'static str {
    match layer {
        mcedit2_world::scene_annotations::CommandLayer::Colors => {
            l.pick("命令方块类别填色", "Command block category colors")
        }
        mcedit2_world::scene_annotations::CommandLayer::Locations => {
            l.pick("命令方块位置线框", "Command block location outlines")
        }
    }
}
pub(crate) fn toggles(app: &App) -> iced::Element<'_, Message> {
    use iced::widget::{checkbox, column, pick_list, text};
    let l = app.language;
    let mut body = column![text(l.pick("显示图层", "Display layers")).size(16)].spacing(5);
    for kind in Kind::ALL {
        body = body.push(
            checkbox(app.scene_layers.mask & kind.bit() != 0)
                .label(label(kind, l))
                .on_toggle(move |enabled| Message::SetSceneLayer(kind, enabled)),
        );
    }
    for layer in mcedit2_world::scene_annotations::CommandLayer::ALL {
        body = body.push(
            checkbox(app.scene_layers.mask & layer.bit() != 0)
                .label(command_label(layer, l))
                .on_toggle(move |enabled| Message::SetCommandLayer(layer, enabled)),
        );
    }
    if app.scene_layers.mask & Kind::Heightmap.bit() != 0 {
        body = body.push(pick_list(
            mcedit2_world::chunk_display::Heightmap::ALL,
            Some(app.scene_layers.heightmap),
            Message::SceneHeightmap,
        ));
        body = body.push(
            text(l.pick(
                "磁盘源高度图；暂存方块更改尚未写入这些数值。",
                "Saved heightmap values; staged block edits are not included.",
            ))
            .size(12),
        );
    }
    if app.scene_layers.mask & (Kind::SpawnDark.bit() | Kind::SpawnNight.bit()) != 0 {
        body = body.push(text(l.pick(
            "按已保存光照显示候选位置；暂存编辑尚未计入，也不代表生物一定能生成。",
            "Candidates use saved light; staged edits are not included. Spawning is not guaranteed.",
        )).size(12));
    }
    body.into()
}
pub(crate) fn controls(app: &App) -> iced::Element<'_, Message> {
    use iced::widget::{button, checkbox, column, scrollable, text};
    let l = app.language;
    let mut body = column![button(l.pick(
        if app.scene_layers.show_controls {
            "收起图层设置"
        } else {
            "显示图层设置"
        },
        if app.scene_layers.show_controls {
            "Hide layer settings"
        } else {
            "Display layer settings"
        }
    ))
    .on_press(Message::SceneLayerControls)]
    .spacing(5);
    if !app.scene_layers.status.is_empty() {
        body = body.push(text(&app.scene_layers.status).size(12));
    }
    if app.scene_layers.show_controls {
        body = body.push(toggles(app));
        body = body.push(
            checkbox(app.scene_layers.inspect)
                .label(l.pick("只读点选图层来源", "Inspect layer sources (read only)"))
                .on_toggle(Message::InspectSceneLayers),
        );
        body = body.push(
            button(l.pick("重新读取显示图层", "Reload display layers"))
                .on_press(Message::ReloadSceneLayers),
        );
        body = body.push(
            button(l.pick("展开/收起图层诊断", "Toggle layer diagnostics"))
                .on_press(Message::SceneLayerDiagnostics),
        );
        if app.scene_layers.show_diagnostics {
            body = body.push(
                scrollable(text(app.scene_layers.diagnostics()).size(12))
                    .height(iced::Length::Fixed(112.)),
            );
        }
    }
    if let Some(detail) = app.scene_layers.detail(l) {
        body = body.push(scrollable(text(detail).size(12)).height(iced::Length::Fixed(112.)));
    }
    body.into()
}
struct Job {
    epoch: u64,
    key: (usize, usize),
    clip: Option<Bounds>,
    session: Session,
    requests: Vec<Coordinate>,
    retained: Vec<(Coordinate, Arc<ChunkAnnotations>)>,
    previous: Option<Arc<Snapshot>>,
    players: Option<Arc<ChunkAnnotations>>,
    entities: bool,
    players_enabled: bool,
    block_visual_numeric_states: Arc<BTreeSet<(u16, u8)>>,
    declarative_block_states: Arc<BTreeSet<String>>,
    cancelled: Arc<AtomicBool>,
    layers: LayerMask,
    heightmap: mcedit2_world::chunk_display::Heightmap,
}
#[derive(Debug)]
pub(crate) struct Batch {
    records: Vec<(Coordinate, Result<Arc<ChunkAnnotations>, String>)>,
    snapshot: Arc<Snapshot>,
    player_catalog: Option<Arc<mcedit2_world::player_catalog::Catalog>>,
    players: Option<Arc<ChunkAnnotations>>,
}
impl Job {
    fn read(self) -> Result<Batch, String> {
        let mut reader = mcedit2_world::scene_annotations::Reader::new(
            self.session.original_world(),
            self.session.dimension(),
        )
        .map_err(|e| e.to_string())?;
        reader.set_block_visual_numeric_states((*self.block_visual_numeric_states).clone());
        reader.set_declarative_block_states((*self.declarative_block_states).clone());
        reader
            .set_display_session(&self.session)
            .map_err(|error| error.to_string())?;
        let mut parts = self.retained;
        let mut player_part = None;
        let players = if !self.players_enabled {
            None
        } else if let Some(markers) = self.players {
            player_part = Some(markers);
            None
        } else {
            let catalog = Arc::new(
                mcedit2_world::player_catalog::load(
                    self.session.original_world(),
                    self.session.dimension(),
                    &|| self.cancelled.load(Ordering::Acquire),
                )
                .map_err(|e| e.to_string())?,
            );
            let mut markers = ChunkAnnotations::default();
            markers
                .diagnostics
                .extend(catalog.diagnostics.iter().cloned());
            for player in &catalog.players {
                let [x, _, z] = player.position;
                let cx = (x / 16.).floor();
                let cz = (z / 16.).floor();
                if cx < i32::MIN as f64
                    || cx > i32::MAX as f64
                    || cz < i32::MIN as f64
                    || cz > i32::MAX as f64
                {
                    markers
                        .diagnostics
                        .push(format!("玩家坐标超出区块索引范围：{:?}", player.position));
                    continue;
                }
                let source = &player.sources[0];
                if let Err(error) = &player.visual {
                    markers
                        .diagnostics
                        .push(format!("玩家 {} 外观损坏：{error}", source.path.display()));
                }
                let visual = player.visual.clone().ok().map(|mut visual| {
                    if let Some(uuid) = player.uuid {
                        visual.appearance.insert(
                            "mcedit2:player_uuid".into(),
                            mcedit2_world::nbt_document::Value::IntArray(
                                uuid.chunks_exact(4)
                                    .map(|v| i32::from_be_bytes(v.try_into().unwrap()))
                                    .collect(),
                            ),
                        );
                    }
                    Arc::new(visual)
                });
                markers.markers.push(Marker {
                    kind: Kind::Player,
                    id: None,
                    position: player.position,
                    path: source.nbt_path.clone(),
                    source: source.path.display().to_string(),
                    chunk: (cx as i32, cz as i32),
                    sha256: source.sha256.clone(),
                    fields: match &source.raw {
                        mcedit2_world::nbt_document::Value::Compound(v) => v.len(),
                        _ => 0,
                    },
                    bytes: source.nbt_bytes,
                    tick: None,
                    priority: None,
                    visual,
                    block_state: None,
                    visual_error: None,
                    block_visual: None,
                    geometry: None,
                });
            }
            let markers = Arc::new(markers);
            player_part = Some(Arc::clone(&markers));
            Some(catalog)
        };
        if let Some(markers) = &mut player_part {
            reader
                .capture_appearance(Arc::make_mut(markers), &|| {
                    self.cancelled.load(Ordering::Acquire)
                })
                .map_err(|error| error.to_string())?;
            for diagnostic in &markers.diagnostics {
                log::error!("[players] {diagnostic}");
            }
        }
        let mut records = Vec::new();
        for (cx, cz) in self.requests {
            if self.cancelled.load(Ordering::Acquire) {
                return Err("位置图层读取已取消".into());
            }
            let result = (|| {
                let entities = if self.entities {
                    self.session.staged_entity_chunk(cx, cz)?
                } else {
                    None
                };
                let block_entities = self.session.staged_be_chunk(cx, cz)?;
                reader.read_with_layers_filtered_cancellable(
                    mcedit2_world::chunk_display::Request {
                        chunk: (cx, cz),
                        layers: self.layers,
                        heightmap: self.heightmap,
                    },
                    entities,
                    block_entities,
                    self.entities,
                    &|| self.cancelled.load(Ordering::Acquire),
                )
            })()
            .map(Arc::new)
            .map_err(|e: mcedit2_world::WorldError| e.to_string());
            if let Ok(part) = &result {
                for diagnostic in &part.diagnostics {
                    log::error!("[layers] {diagnostic}");
                }
                if !part.markers.is_empty() {
                    parts.push(((cx, cz), Arc::clone(part)));
                }
            }
            records.push(((cx, cz), result));
        }
        if self.cancelled.load(Ordering::Acquire) {
            return Err("位置图层读取已取消".into());
        }
        // 读取优先级可以随相机变化；最终源序只由区块坐标及原 NBT 列表决定。
        // 玩家目录是独立来源，固定追加在地形 / 实体 Region 来源之后。
        parts.sort_unstable_by_key(|(coordinate, _)| *coordinate);
        let coordinates: BTreeSet<_> = parts
            .iter()
            .flat_map(|(_, part)| &part.markers)
            .filter(|marker| {
                matches!(
                    marker.block_visual.as_deref(),
                    Some(mcedit2_world::scene_annotations::BlockVisual::Piston(_))
                )
            })
            .map(|marker| marker.chunk)
            .collect();
        let mut biomes = if coordinates.is_empty() {
            Arc::default()
        } else {
            mcedit2_world::surface::inspect_biome_chunks(
                &self.session,
                coordinates.into_iter(),
                &|| self.cancelled.load(Ordering::Acquire),
            )
            .map_err(|error| error.to_string())?
        };
        if let Some(previous) = &self.previous {
            Arc::make_mut(&mut biomes).reuse_columns(&previous.biomes);
        }
        let player_markers = player_part.clone();
        let snapshot = Snapshot::build_cancellable(
            self.key,
            self.epoch,
            parts.into_iter().map(|(_, part)| part).chain(player_part),
            self.clip,
            biomes,
            self.previous.as_ref(),
            &|| self.cancelled.load(Ordering::Acquire),
        )
        .ok_or_else(|| "图层索引构建已取消".to_owned())?;
        Ok(Batch {
            records,
            player_catalog: players,
            players: player_markers,
            snapshot,
        })
    }
}
impl App {
    pub(super) fn maintain_scene_layers(&mut self) -> Task<Message> {
        let quad = if self.show_advanced && self.advanced_section == crate::AdvancedSection::Quad {
            self.quad
                .source_scope()
                .filter(|(key, _)| Some(*key) == self.current_key())
        } else {
            None
        };
        let planes = if self.show_advanced && self.advanced_section == crate::AdvancedSection::Maps
        {
            let horizontal = self
                .slice_grid
                .as_ref()
                .filter(|(wi, di, _)| Some((*wi, *di)) == self.current_key())
                .map(|(_, _, slice)| Arc::clone(slice));
            let vertical = self
                .vertical_grid
                .as_ref()
                .filter(|(wi, di, _)| Some((*wi, *di)) == self.current_key())
                .map(|(_, _, slice)| Arc::clone(slice));
            (horizontal.is_some() || vertical.is_some()).then_some((horizontal, vertical))
        } else {
            None
        };
        let disabled = self.scene_layers.mask == 0 && self.texture_bank.is_none();
        if disabled || (!self.visible_scene_is_current() && quad.is_none() && planes.is_none()) {
            // 场景换源期间暂时没有 Surface；保留已按改动范围裁撤的源段，等新窗口发布。
            let transition = !disabled
                && self.scene_layers.source_changed
                && self
                    .scene_layers
                    .scope
                    .as_ref()
                    .is_some_and(|scope| Some(scope.key()) == self.current_key());
            if self.scene_layers.scope.is_some() && !transition {
                self.scene_layers.invalidate();
            }
            return Task::none();
        }
        let mode = (
            self.entity_models.enabled
                || self.scene_layers.mask & (Kind::Entity.bit() | Kind::Item.bit()) != 0,
            self.entity_models.enabled || self.scene_layers.mask & Kind::Player.bit() != 0,
        );
        let block_visual_resources = self.scene_texture_bank().map(|bank| bank.visibility_id());
        let mut definitions_changed = false;
        if self.scene_layers.block_visual_resources != block_visual_resources {
            let declarative = self
                .scene_texture_bank()
                .map(|bank| bank.declarative_block_states())
                .unwrap_or_default();
            let numeric = self
                .scene_texture_bank()
                .map(|bank| bank.block_visual_numeric_states())
                .unwrap_or_default();
            self.scene_layers.block_visual_resources = block_visual_resources;
            if self.scene_layers.declarative_block_states.as_ref() != &declarative
                || self.scene_layers.block_visual_numeric_states.as_ref() != &numeric
            {
                definitions_changed = true;
                self.scene_layers.declarative_block_states = Arc::new(declarative);
                self.scene_layers.block_visual_numeric_states = Arc::new(numeric);
            }
        }
        if definitions_changed || self.scene_layers.read_mode != Some(mode) {
            self.scene_layers.invalidate();
            self.scene_layers.read_mode = Some(mode);
        }
        let Some(key) = self.current_key() else {
            return Task::none();
        };
        let root = self.worlds[key.0].root.clone();
        let scope = if let Some((_, ranges)) = quad {
            Scope::Quad {
                key,
                root,
                ranges,
                source: self.quad.source_generation(),
            }
        } else if let Some((horizontal, vertical)) = planes {
            Scope::Planes {
                key,
                root,
                horizontal,
                vertical,
            }
        } else if self.uses_world_mesh() {
            let Some(stream) = &self.spectator_stream else {
                return Task::none();
            };
            let Some(nonce) = stream.source_plan_nonce() else {
                return Task::none();
            };
            Scope::Stream { key, root, nonce }
        } else {
            let Some((_, _, source)) = &self.surface else {
                return Task::none();
            };
            let clip = if let Some(window) = self.selected_window() {
                let mut clip = window.bounds;
                if let Some(top) = window.cut {
                    clip.maximum[1] = clip.maximum[1].min(top);
                }
                clip
            } else {
                let endpoint = |start: i32, len: u16| {
                    len.checked_sub(1)
                        .and_then(|len| start.checked_add(i32::from(len)))
                };
                let (Some(x), Some(z)) = (
                    endpoint(source.x, source.width),
                    endpoint(source.z, source.depth),
                ) else {
                    let error = "位置图层源窗口坐标溢出";
                    log::error!("[layers] {error}");
                    self.scene_layers.status = error.into();
                    return Task::none();
                };
                Bounds {
                    minimum: [source.x, source.y_min, source.z],
                    maximum: [x, source.y_max, z],
                }
            };
            Scope::Surface {
                key,
                root,
                source: Arc::clone(source),
                clip,
            }
        };
        if self.scene_layers.source_changed
            || self
                .scene_layers
                .scope
                .as_ref()
                .is_none_or(|old| !old.same(&scope))
        {
            let coordinates = match &scope {
                Scope::Stream { .. } => self
                    .spectator_stream
                    .as_ref()
                    .unwrap()
                    .source_coordinates()
                    .collect(),
                Scope::Surface { source, .. } if !source.source_tiles.is_empty() => source
                    .source_tiles
                    .iter()
                    .filter(|tile| tile.source_present)
                    .map(|tile| (tile.chunk_x, tile.chunk_z))
                    .collect(),
                Scope::Surface { clip, .. } => (clip.minimum[0].div_euclid(16)
                    ..=clip.maximum[0].div_euclid(16))
                    .flat_map(|x| {
                        (clip.minimum[2].div_euclid(16)..=clip.maximum[2].div_euclid(16))
                            .map(move |z| (x, z))
                    })
                    .collect(),
                Scope::Quad { ranges, .. } => ranges
                    .iter()
                    .flat_map(|clip| {
                        (clip.minimum[0].div_euclid(16)..=clip.maximum[0].div_euclid(16)).flat_map(
                            move |x| {
                                (clip.minimum[2].div_euclid(16)..=clip.maximum[2].div_euclid(16))
                                    .map(move |z| (x, z))
                            },
                        )
                    })
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                Scope::Planes {
                    horizontal,
                    vertical,
                    ..
                } => horizontal
                    .iter()
                    .map(|slice| slice.bounds)
                    .chain(vertical.iter().map(|slice| slice.bounds))
                    .flat_map(|clip| {
                        (clip.minimum[0].div_euclid(16)..=clip.maximum[0].div_euclid(16)).flat_map(
                            move |x| {
                                (clip.minimum[2].div_euclid(16)..=clip.maximum[2].div_euclid(16))
                                    .map(move |z| (x, z))
                            },
                        )
                    })
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            };
            self.scene_layers.replan(scope, coordinates);
        }
        if self.scene_layers.running
            || self.scene_layers.failed
            || self.scene_layers.pending.is_empty() && self.scene_layers.snapshot.is_some()
        {
            return Task::none();
        }
        let session = match self.display_session(key) {
            Ok(session) => session.clone(),
            Err(error) => {
                log::error!("[layers] 无法建立位置图层读取：{error}");
                self.scene_layers.status = error;
                return Task::none();
            }
        };
        self.scene_layers.fetch(session, mode.0, mode.1)
    }
}

#[cfg(test)]
#[path = "scene_layer_tests.rs"]
pub(crate) mod tests;
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}
