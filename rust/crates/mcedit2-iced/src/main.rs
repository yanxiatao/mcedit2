#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Native Rust + iced desktop with legacy MCEdit2 positional world arguments.
mod about;
mod advanced_pages;
mod analysis_ui;
mod appearance_results;
#[cfg(test)]
mod application_probe;
mod atlas_inspector;
mod auto_assets;
mod banner_geometry;
mod billboard;
mod block_definitions;
mod block_definitions_ui;
mod block_edit;
mod block_forms;
mod block_names;
mod block_picker;
mod block_picker_icon;
mod block_replacement_ui;
mod block_visibility;
mod brush_cursor;
mod brush_stroke;
mod canvas_raster;
mod chunk_mesh_cache;
mod client_assets;
mod client_installations;
mod command_visuals;
mod console;
mod container_ui;
mod crash_log;
mod deletion_ui;
mod desktop_state;
mod dialog_paths;
mod dialogs;
mod display_options;
mod drag_drop;
mod editor_options;
mod entry;
mod experience_geometry;
mod file_input;
mod file_ui;
mod floating_panels;
mod flood_fill;
mod fluid_material;
mod font_glyphs;
mod font_resources;
mod font_unihex;
mod frame_geometry;
mod frustum;
mod gpu_mesh;
mod gpu_scene;
mod guardian_eye;
#[cfg(test)]
#[path = "../../../tests/support/hanging_fixture.rs"]
mod hanging_fixture;
mod history_ui;
mod hover_query;
#[cfg(all(test, debug_assertions))]
mod input_batches;
mod inventory_grid;
mod inventory_ui;
mod isometric;
mod item_appearance;
mod item_catalog;
mod item_generated;
mod item_geometry;
mod item_glint;
mod item_icons;
mod item_presets;
mod item_specials;
mod item_tints;
mod item_world;
mod java_random;
mod language;
mod launcher_roots;
mod layer_renderer;
mod legacy_chest_context;
mod legacy_render;
mod library;
mod log_view;
mod map_editor;
#[cfg(test)]
#[path = "../../../tests/support/map_fixture.rs"]
mod map_fixture;
mod map_frame;
mod map_geometry;
mod map_refresh;
mod material_mips;
mod material_mode;
mod mesh_async;
#[cfg(test)]
#[path = "../../../tests/support/metadata_fixture.rs"]
mod metadata_fixture;
mod metadata_pages;
mod metadata_refresh;
#[cfg(test)]
mod metadata_tests;
mod mixed_save;
mod model_occlusion;
mod models;
mod name_geometry;
mod native_profile;
mod nbt_documents;
mod nbt_editor;
mod nbt_search;
mod nbt_tree;
#[cfg(test)]
#[path = "../../../tests/support/object_fixture.rs"]
mod object_fixture;
mod object_inspector;
#[cfg(test)]
mod object_tests;
mod orthographic_framing;
mod orthographic_world;
mod pack_sources;
mod painting_geometry;
mod palette;
mod pending_import;
mod picking;
mod piston_geometry;
mod placement;
mod placement_target;
mod player_browser;
mod player_editor;
mod player_profile;
mod player_skin;
mod position_input;
mod projected_entities;
mod radix_sort;
mod region_recovery;
mod render_viewport;
mod render_wake;
mod render_work;
mod resource_bindings;
mod save_postprocess;
mod saved_sessions;
mod section_culling;
mod selection_clipboard;
mod selection_contour;
mod selection_edit;
mod selection_handle;
mod selection_preferences;
mod sign_geometry;
#[cfg(test)]
mod sign_render_tests;
mod sign_text;
mod single_instance;
mod skull_geometry;
mod skull_skin;
mod stream_readers;
mod structure_edit;
mod structure_exchange;
mod structure_scene;
mod structure_slice;
mod structure_workspace;
mod tab_bar;
mod transfer_edit;
mod transform_controls;
mod view_state;
#[path = "gpu_visibility.rs"]
mod visibility_index;
mod visual_schema;
mod workspace_tabs;
mod workspace_ui;
mod world_browser;
mod world_hover;
mod world_info;
mod world_save;
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    #[cfg(debug_assertions)]
    if category == crate::test_categories::Category::Navigation {
        input_batches::verify();
    }
    hover_query::run_test_category(category);
    tests::run_test_category(category);
    auto_assets::run_test_category(category);
    block_names::run_test_category(category);
    canvas_raster::run_test_category(category);
    chunk_mesh_cache::run_test_category(category);
    frustum::run_test_category(category);
    gpu_mesh::run_test_category(category);
    gpu_scene::run_test_category(category);
    isometric::run_test_category(category);
    legacy_render::run_test_category(category);
    material_mode::run_test_category(category);
    mesh_async::run_test_category(category);
    models::run_test_category(category);
    orthographic_world::run_test_category(category);
    palette::run_test_category(category);
    picking::run_test_category(category);
    picking_integration_tests::run_test_category(category);
    transfer_edit::run_test_category(category);
    structure_exchange::run_test_category(category);
    block_edit::run_test_category(category);
    brush_stroke::run_test_category(category);
    brush_cursor::run_test_category(category);
    plugin_ui::run_test_category(category);
    metadata_tests::run_test_category(category);
    selection_clipboard::run_test_category(category);
    selection_edit::run_test_category(category);
    inventory_ui::run_test_category(category);
    item_icons::run_test_category(category);
    if category == test_categories::Category::Rendering {
        selection_contour::run_tests();
    }
    selection_handle::run_test_category(category);
    object_tests::run_test_category(category);
    workspace_ui::run_test_category(category);
    workspace_tabs::run_test_category(category);
    map_editor::run_test_category(category);
    quad_view::run_test_category(category);
    repeating_uv::run_test_category(category);
    resource_budget::run_test_category(category);
    resource_ids::run_test_category(category);
    resource_models::run_test_category(category);
    root_transform::run_test_category(category);
    slice_canvas::run_test_category(category);
    special_gltf::run_test_category(category);
    world_mesh::run_test_category(category);
    scene_selection::run_test_category(category);
    scene_layers::run_test_category(category);
    entity_geometry::run_test_category(category);
    sign_render_tests::run_test_category(category);
    special_mtl::run_test_category(category);
    special_obj::run_test_category(category);
    spectator::run_test_category(category);
    spectator_stream::run_test_category(category);
    static_triangle_uv::run_test_category(category);
    textures::run_test_category(category);
    top_down::run_test_category(category);
    view_navigation::run_test_category(category);
}

mod keyboard_bindings;
mod lighting;
mod mesh_geometry;
mod minimap;
mod navigation_bindings;
#[cfg(test)]
mod picking_integration_tests;
mod plugin_context;
mod plugin_ui;
mod quad_view;
mod repeating_uv;
mod resource_budget;
mod resource_ids;
mod resource_models;
mod root_transform;
mod runtime_rates;
mod sky_background;
mod slice_canvas;
#[cfg(test)]
mod special_gltf;
mod special_gltf_types;
mod view_compass;
mod work_plane;
mod world_camera;
mod world_mesh;
mod world_ruler;
#[cfg(not(test))]
use special_gltf_types as special_gltf;
mod armor_geometry;
mod camera_distance;
mod chunk_operations_ui;
mod chunk_outline;
mod elytra_geometry;
mod entity_fire;
mod entity_geometry;
mod entity_visibility;
mod equipment_geometry;
mod scene_layers;
mod scene_selection;
mod source_visibility;
mod special_mtl;
mod special_obj;
mod spectator;
mod spectator_stream;
mod startup_script_ui;
mod static_triangle_uv;
mod terrain_budget;
mod textures;
mod toolbar;
mod top_down;
mod view_navigation;
use gpu_scene::GpuScene;
use gpu_scene::SceneSource;
use iced::widget::{button, canvas, column, container, row, scrollable, text, text_input};
use iced::{Element, Length, Task, Theme};
use isometric::{Face, IsometricSurface};
use language::Language;
use mcedit2_world::block_lookup::{parse_xyz, read_world_block};
use mcedit2_world::editing::{
    parse_bounds, BeScalarPreview, Bounds, BrushShape, Preview, Session, Slice, SliceCell,
    VerticalSlice,
};
use mcedit2_world::far_view::{self, FarChunkPatch, FarChunkRequest};
use mcedit2_world::region_plan::NativeRegionCandidate;
use mcedit2_world::region_transaction::validate_region_group;
use mcedit2_world::surface::SurfaceView;
use mcedit2_world::{compatibility_label, inspect_input, launch, WorldSummary};
use slice_canvas::{Map2D, Plane};
use spectator::{Camera, SpectatorScene};
use spectator_stream::Stream as SpectatorStream;
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use top_down::TopDown;

/// Snapshot-only read dispatch: the iced thread must never block on
/// decoding original chunks, and results must not cross a world/dimension
/// switch, a newer request, or an in-memory edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NearMode {
    Surface,
    Volume,
    Horizontal,
    Vertical,
}
type SceneAssets = Option<std::sync::Arc<textures::TextureBank>>;
type SurfaceRead = (SurfaceView, SceneAssets, bool, block_visibility::Layers);
type SourceRequest = ((usize, usize), NearMode, Bounds, Option<i32>);
type ResourceRequest = (SourceRequest, source_visibility::Identity);
struct NearJob {
    nonce: u64,
    key: (usize, usize),
    mode: NearMode,
    bounds: Bounds,
    cut: Option<i32>,
    source: Session,
    assets: Option<std::sync::Arc<textures::TextureBank>>,
    show_textures: bool,
    block_layers: block_visibility::Layers,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
#[derive(Debug, Clone)]
enum NearResult {
    Surface(Result<SurfaceRead, String>),
    Horizontal(Result<Slice, String>),
    Vertical(Result<VerticalSlice, String>),
    WorkerStopped,
}
impl NearJob {
    /// 后台读取与回归验证共用同一条实际世界读取路径。
    fn read(&self) -> NearResult {
        let cancelled = || self.cancelled.load(std::sync::atomic::Ordering::Acquire);
        if cancelled() {
            return NearResult::WorkerStopped;
        }
        let b = self.bounds;
        match self.mode {
            NearMode::Surface | NearMode::Volume => {
                let width = (b.maximum[0] as i64 - b.minimum[0] as i64 + 1) as u8;
                let depth = (b.maximum[2] as i64 - b.minimum[2] as i64 + 1) as u8;
                let mut visibility =
                    source_visibility::Reader::new(self.assets.clone(), self.show_textures)
                        .with_layers(self.block_layers);
                let read = if self.mode == NearMode::Volume {
                    mcedit2_world::surface::inspect_volume_with_visibility(
                        &self.source,
                        mcedit2_world::surface::VolumeRequest {
                            x: b.minimum[0],
                            z: b.minimum[2],
                            vertical: b.minimum[1]..=b.maximum[1],
                            width,
                            depth,
                            top_cut: self.cut,
                        },
                        &mut visibility,
                        &cancelled,
                    )
                } else {
                    // Overhead/orthographic views must inspect the full
                    // world height, not the spectator's 48-block fly window.
                    // Otherwise a camera above terrain reports zero blocks.
                    // 扫描源 Section 的全部可编码高度，保留自定义维度的高低地形。
                    let (ymin, ymax) = (-2048, 2047);
                    mcedit2_world::surface::inspect_surface_with_visibility(
                        &self.source,
                        mcedit2_world::surface::VolumeRequest {
                            x: b.minimum[0],
                            z: b.minimum[2],
                            vertical: ymin..=ymax,
                            width,
                            depth,
                            top_cut: None,
                        },
                        &mut visibility,
                        &cancelled,
                    )
                };
                NearResult::Surface(
                    read.map(|view| {
                        (
                            view,
                            self.assets.clone(),
                            self.show_textures,
                            self.block_layers,
                        )
                    })
                    .map_err(|e| e.to_string()),
                )
            }
            NearMode::Horizontal => {
                NearResult::Horizontal(self.source.inspect_slice(b).map_err(|e| e.to_string()))
            }
            NearMode::Vertical => NearResult::Vertical(
                self.source
                    .inspect_vertical_slice(b)
                    .map_err(|e| e.to_string()),
            ),
        }
    }
}
fn dispatch_near(job: NearJob) -> iced::Task<Message> {
    iced::Task::perform(
        async move {
            let (nonce, key) = (job.nonce, job.key);
            let (sender, receiver) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                if sender.send(job.read()).is_err() {
                    log::debug!("[view] 只读窗口结果接收端已关闭");
                }
            });
            (
                nonce,
                key,
                receiver.await.unwrap_or(NearResult::WorkerStopped),
            )
        },
        |(nonce, key, result)| Message::NearLoaded(nonce, key, result),
    )
}

/// World directory/level.dat indexing also belongs off the iced UI loop.
/// Probe one verified source chunk for a plausible terrain-centered first view.
fn initial_scene_height(world: &WorldSummary, index: usize) -> i32 {
    let Some(dim) = world.dimensions.get(index) else {
        return 64;
    };
    let Ok(Some(chunk)) = mcedit2_world::block_lookup::read_world_chunk(world, &dim.name, 0, 0)
    else {
        return 64;
    };
    for y in chunk.sections.iter().rev().flat_map(|section| {
        let base = i32::from(section.y) * 16;
        (base..base + 16).rev()
    }) {
        if let Some(state) = chunk.block(8, y, 8) {
            if !matches!(
                state.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "numeric:0"
            ) {
                return y;
            }
        }
    }
    64
}
fn dispatch_open(nonce: u64, path: PathBuf) -> iced::Task<Message> {
    iced::Task::perform(
        async move {
            let (sender, receiver) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let result = inspect_input(&path)
                    .map(|world| {
                        let height = initial_scene_height(&world, 0);
                        let locations = view_state::locations(&world);
                        (world, height, locations)
                    })
                    .map_err(|e| e.to_string());
                let _ = sender.send(result);
            });
            (
                nonce,
                receiver
                    .await
                    .unwrap_or_else(|_| Err("Original world indexing worker stopped".into())),
            )
        },
        |(nonce, result)| Message::OpenFinished(nonce, result),
    )
}
/// 用户明确检查的候选镜像集合；仅生成预览不会修改磁盘。
#[derive(Debug, Clone)]
struct SaveIntent {
    key: (usize, usize),
    timestamp: u32,
    candidates: save_postprocess::Candidates,
    policy: mcedit2_world::chunk_postprocess::Policy,
    be_mode: bool,
}
#[derive(Debug, Clone)]
struct SaveOutcome {
    backups: Vec<PathBuf>,
    files: usize,
    history: Option<mcedit2_world::history_source::Archive>,
}
impl SaveOutcome {
    fn report(&self) -> String {
        if self.files == 0 {
            "内容与磁盘一致，已更新保存点，无需写入文件。".into()
        } else {
            format!(
                "已保存 {} 个文件，保留 {} 份原始备份。",
                self.files,
                self.backups.len()
            )
        }
    }
}
fn persist_intent(
    world: &WorldSummary,
    dimension: &str,
    intent: SaveIntent,
    prepared: &mut Option<mcedit2_world::mixed_transaction::PendingSummary>,
) -> Result<SaveOutcome, String> {
    use mcedit2_world::mixed_transaction;
    if let Some(pending) =
        mixed_transaction::pending_summary(world).map_err(|error| error.to_string())?
    {
        return Err(format!(
            "已有事务 {}，请先完成或回滚",
            pending.marker.display()
        ));
    }
    let has_be = intent.candidates.requires_nbt_confirmation();
    if intent.be_mode != has_be {
        return Err("保存确认方式与候选内容不一致".into());
    }
    let candidates = intent.candidates.into_mixed(dimension);
    if candidates.is_empty() {
        return Ok(SaveOutcome {
            backups: Vec::new(),
            files: 0,
            history: None,
        });
    }
    mixed_transaction::prepare(
        world,
        &candidates,
        false,
        if has_be { "SAVE_BE_NBT" } else { "SAVE" },
    )
    .map_err(|error| error.to_string())?;
    let pending = mixed_transaction::pending_summary(world)
        .map_err(|error| error.to_string())?
        .ok_or("保存准备后缺少实际事务记录")?;
    *prepared = Some(pending.clone());
    let (files, history) = mixed_transaction::finish_with_history(world, false, Some(&pending))
        .map_err(|error| error.to_string())?;
    Ok(SaveOutcome {
        backups: pending.backups,
        files,
        history: Some(history),
    })
}
fn matches_save_preview(current: &save_postprocess::Candidates, intent: &SaveIntent) -> bool {
    current.same(&intent.candidates)
}
fn parse_region_name(name: &str) -> Result<(i32, i32, &str), String> {
    let mut parts = name.trim().split('.');
    if parts.next() != Some("r") {
        return Err("Use r.X.Z.mca or r.X.Z.mcr".into());
    }
    let rx = parts
        .next()
        .ok_or("Missing Region X")?
        .parse::<i32>()
        .map_err(|_| "Region X must be a signed integer")?;
    let rz = parts
        .next()
        .ok_or("Missing Region Z")?
        .parse::<i32>()
        .map_err(|_| "Region Z must be a signed integer")?;
    let ext = parts.next().ok_or("Missing Region extension")?;
    if !matches!(ext, "mca" | "mcr") || parts.next().is_some() {
        return Err("Only original r.X.Z.mca or r.X.Z.mcr filenames are accepted".into());
    }
    Ok((rx, rz, ext))
}
#[cfg(test)]
fn slice_cell_label(cell: &SliceCell) -> String {
    match cell {
        SliceCell::MissingChunk => "·".into(),
        SliceCell::MissingSection => "?".into(),
        SliceCell::Present {
            original_block_entity: true,
            ..
        } => "@".into(),
        SliceCell::Present { state, .. } => state
            .name
            .rsplit(':')
            .next()
            .unwrap_or("?")
            .chars()
            .next()
            .unwrap_or('?')
            .to_ascii_uppercase()
            .to_string(),
    }
}
fn shifted_slice(bounds: Bounds, dx: i32, dy: i32, dz: i32) -> Result<String, String> {
    let add = |v: i32, delta: i32| {
        v.checked_add(delta)
            .ok_or_else(|| "Slice pan coordinate overflow".to_owned())
    };
    let first = (
        add(bounds.minimum[0], dx)?,
        add(bounds.minimum[1], dy)?,
        add(bounds.minimum[2], dz)?,
    );
    let last = (
        add(bounds.maximum[0], dx)?,
        add(bounds.maximum[1], dy)?,
        add(bounds.maximum[2], dz)?,
    );
    Bounds::new(first, last).map_err(|e| e.to_string())?;
    Ok(format!(
        "{},{},{};{},{},{}",
        first.0, first.1, first.2, last.0, last.1, last.2
    ))
}

fn parse_scene_window(input: &str) -> Result<Bounds, String> {
    let (first, last) = input
        .split_once(';')
        .ok_or("3D scene requires X1,Y1,Z1;X2,Y2,Z2")?;
    if last.contains(';') {
        return Err("Too many 3D viewport separators".into());
    }
    let a = mcedit2_world::block_lookup::parse_xyz(first).map_err(|e| e.to_string())?;
    let b = mcedit2_world::block_lookup::parse_xyz(last).map_err(|e| e.to_string())?;
    let minimum = [a.0.min(b.0), a.1.min(b.1), a.2.min(b.2)];
    let maximum = [a.0.max(b.0), a.1.max(b.1), a.2.max(b.2)];
    for v in [minimum, maximum] {
        Bounds::new((v[0], v[1], v[2]), (v[0], v[1], v[2])).map_err(|e| e.to_string())?;
    }
    for (index, maximum_size) in [(0, 64), (1, 48), (2, 64)] {
        if maximum[index] as i64 - minimum[index] as i64 + 1 > maximum_size {
            return Err("3D view is limited to 64x64x48 source cells".into());
        }
    }
    // 三维选区可交给预算化编辑；旧剪贴板等工具仍使用各自的工作量限制。
    // The strict read-only 196608-source-cell limit is enforced by the backend.
    Ok(Bounds { minimum, maximum })
}
fn shifted_scene(input: &str, dx: i32, dy: i32, dz: i32) -> Result<String, String> {
    let area = parse_scene_window(input)?;
    let shift = |p: [i32; 3]| -> Result<[i32; 3], String> {
        Ok([
            p[0].checked_add(dx).ok_or("Scene X overflow")?,
            p[1].checked_add(dy).ok_or("Scene Y overflow")?,
            p[2].checked_add(dz).ok_or("Scene Z overflow")?,
        ])
    };
    let a = shift(area.minimum)?;
    let b = shift(area.maximum)?;
    let text = format!("{},{},{};{},{},{}", a[0], a[1], a[2], b[0], b[1], b[2]);
    parse_scene_window(&text)?;
    Ok(text)
}

/// Only exact same source chunk + requested LOD can be reused. Never
/// use a cached distant sample for editor selection or source provenance.
const FAR_CACHE_PATCHES: usize = 4096;
const FAR_CACHE_VOXELS: usize = 90_000;
type FarCacheKey = (i32, i32, u8);
struct FarProgress {
    nonce: u64,
    key: (usize, usize),
    center: (i32, i32),
    radius: u16,
    /// Immutable source snapshot: only display navigation can reuse it;
    /// any staging/undo/commit invalidates this entire FarProgress.
    source: std::sync::Arc<Session>,
    remaining: VecDeque<FarChunkRequest>,
    total: usize,
    complete: usize,
    errors: usize,
    cache: BTreeMap<FarCacheKey, std::sync::Arc<FarChunkPatch>>,
    cache_order: VecDeque<FarCacheKey>,
    cached_voxels: usize,
}
fn far_cache_key(req: FarChunkRequest) -> FarCacheKey {
    (req.cx, req.cz, req.span)
}
impl FarProgress {
    fn remember(&mut self, req: FarChunkRequest, patch: &std::sync::Arc<FarChunkPatch>) {
        let key = far_cache_key(req);
        if self.cache.contains_key(&key) {
            return;
        }
        // A large original chunk cannot starve future distant reuse.
        if patch.voxels.len() > FAR_CACHE_VOXELS {
            return;
        }
        self.cached_voxels += patch.voxels.len();
        self.cache.insert(key, std::sync::Arc::clone(patch));
        self.cache_order.push_back(key);
        while self.cache.len() > FAR_CACHE_PATCHES || self.cached_voxels > FAR_CACHE_VOXELS {
            let Some(old) = self.cache_order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.cache.remove(&old) {
                self.cached_voxels = self.cached_voxels.saturating_sub(evicted.voxels.len());
            }
        }
    }
}
#[derive(PartialEq, Eq)]
struct RenderSignature {
    /// World selection, source snapshot address and sampled source counts.
    source: Option<(usize, usize, usize, usize, usize, usize)>,
    geometry: Option<textures::VisibilityIdentity>,
    block_layers: block_visibility::Layers,
    camera: u8,
    yaw: i32,
    pitch: i32,
    zoom: u32,
    pan_x: u32,
    pan_y: u32,
    show_textures: bool,
    entity_frame: Option<usize>,
}
#[derive(Default)]
struct App {
    single_instance: Option<single_instance::Server>,
    atlas_inspector: atlas_inspector::State,
    about: about::State,
    native_profile: Option<native_profile::State>,
    view_state: view_state::State,
    desktop: desktop_state::State,
    world_browser: world_browser::State,
    file_input: file_input::State,
    library: library::State,
    history: history_ui::State,
    analysis: analysis_ui::State,
    block_replacement: block_replacement_ui::State,
    block_definitions: block_definitions_ui::State,
    block_picker: block_picker::State,
    log_view: log_view::State,
    deletion: deletion_ui::State,
    chunk_operations: chunk_operations_ui::State,
    startup_script: startup_script_ui::State,
    tools_pane: workspace_ui::ToolsPane,
    drag_drop: drag_drop::State,
    structures: structure_workspace::State,
    main_window: Option<iced::window::Id>,
    floating_panels: floating_panels::State,
    minimap: minimap::State,
    work_plane: work_plane::State,
    nbt_search: nbt_search::State,
    save_postprocess: save_postprocess::Panel,
    save_cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    save_planning: bool,
    save_worker: bool,
    world_save: world_save::State,
    source_locations: BTreeMap<PathBuf, mcedit2_world::player_catalog::Locations>,
    map_editor: map_editor::Panel,
    metadata_pages: metadata_pages::State,
    mixed_save: mixed_save::State,
    region_recovery: region_recovery::State,
    player_browser: player_browser::Panel,
    transfer: transfer_edit::State,
    pending_import: pending_import::State,
    placement: placement::State,
    placement_targets: std::sync::Arc<placement_target::Cache>,
    block_edit: block_edit::State,
    map_refresh: map_refresh::State,
    structure_creation: structure_edit::CreationPanel,
    plugins: plugin_ui::Panel,
    quad: quad_view::Panel,
    path: String,
    language: Language,
    show_advanced: bool,
    advanced_section: AdvancedSection,
    workspace_pane: workspace_ui::Pane,
    editor_tool: editor_options::Tool,
    workspace: workspace_tabs::State,
    shortcuts: keyboard_bindings::State,
    input_epoch: u64,
    surface_area: String,
    surface_exposed: bool,
    /// 用户已应用的源窗口与输入框草稿分开；相机导航不能改写它。
    selected_scene: Option<scene_selection::Window>,
    /// 只在窗口改变时定位透视相机；切换投影后保留用户飞行位置和方向。
    window_camera: Option<scene_selection::Window>,
    surface_cut_top: Option<i32>,
    surface_frame: bool,
    scene_layers: scene_layers::State,
    entity_models: entity_geometry::State,
    metadata_refresh: metadata_refresh::State,
    nbt_documents: nbt_documents::State,
    selection_clipboard: selection_clipboard::State,
    item_clipboard: Option<std::sync::Arc<mcedit2_world::inventory::Stack>>,
    selection_edit: selection_edit::State,
    nbt_path: String,
    nbt_editable: bool,
    nbt_status: String,
    nbt_prefill: Option<nbt_documents::Prefill>,
    nbt_group: Vec<mcedit2_world::nbt_file::Candidate>,
    nbt_group_key: Option<(usize, usize)>,
    nbt_group_busy: bool,
    object_inspector: object_inspector::State,
    be_tree_preview: Option<mcedit2_world::editing::BeNbtPreview>,
    be_tree_operation: String,
    be_tree_status: String,
    entity_clone_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    entity_edit_preview: Option<mcedit2_world::editing::EntityPreview>,
    entity_edit_delta: String,
    entity_edit_status: String,
    nbt_group_status: String,
    mixed_phrase: String,
    mixed_busy: bool,
    mixed_status: String,
    mixed_key: Option<(usize, usize)>,
    mixed_sources: BTreeMap<(usize, usize), std::sync::Arc<()>>,
    surface_axes: bool,
    surface_chunk_grid: bool,
    surface_section_guides: bool,
    show_textures: bool,
    use_gpu: bool,
    show_top_down: bool,
    spectator: bool,
    spectator_camera: Camera,
    spectator_keys: std::collections::BTreeSet<char>,
    spectator_last_frame: Option<std::time::Instant>,
    spectator_cache:
        std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<spectator::StreamMesh>>>>,
    spectator_presented:
        std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<spectator::StreamMesh>>>>,
    spectator_presented_view: spectator::PresentedViewCache,
    spectator_pending: std::sync::Arc<std::sync::Mutex<bool>>,
    /// Last completed GPU mesh and the source origin under which it was
    /// generated. Display it while asynchronous streaming prepares the next.
    spectator_fallback: Option<spectator::CompletedFrame>,
    spectator_chunk_cache: std::sync::Arc<std::sync::Mutex<chunk_mesh_cache::ChunkMeshCache>>,
    spectator_stream: Option<SpectatorStream>,
    spectator_stream_view: Option<std::sync::Arc<SurfaceView>>,
    spectator_stream_snapshot: Option<std::sync::Arc<spectator_stream::SceneSnapshot>>,
    spectator_stream_near: Option<usize>,
    spectator_stream_nonce: u64,
    spectator_radius: u16,
    spectator_sampled: bool,
    gpu_quality: gpu_mesh::Quality,
    texture_bank: Option<std::sync::Arc<textures::TextureBank>>,
    /// 当前显示源读取时使用的资源；新资源完成读取前不混用裁面与材质。
    surface_assets: Option<std::sync::Arc<textures::TextureBank>>,
    /// 与源可见面共同发布；等待重读时继续使用该源的实际显示方式。
    surface_textures: bool,
    surface_block_layers: block_visibility::Layers,
    auto_assets_attempted: std::collections::BTreeSet<(usize, PathBuf)>,
    manual_textures: bool,
    texture_load_nonce: u64,
    texture_cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    texture_loading: bool,
    texture_budget: String,
    texture_blocks_only: bool,
    texture_diagnostics: bool,
    /// Persist projected faces across reactive iced view rebuilds. Only
    /// geometry/source/camera invalidates it; hover and PNG ticks do not.
    render_face_cache: isometric::FrameCache<isometric::RenderFace>,
    render_framing_cache: orthographic_framing::Cache,
    world_orthographic_framing: orthographic_world::Cache,
    render_canvas_cache: canvas_raster::Cache,
    render_gpu_cache: isometric::FrameCache<gpu_mesh::Vertex>,
    projected_ready:
        std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<projected_entities::Rendered>>>>,
    grid_main: std::sync::Arc<projected_entities::GridCache>,
    grid_horizontal: std::sync::Arc<projected_entities::GridCache>,
    grid_vertical: std::sync::Arc<projected_entities::GridCache>,
    grid_revision: u64,
    projected_presented:
        std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<projected_entities::Rendered>>>>,
    render_gpu_pending: std::sync::Arc<std::sync::Mutex<Option<iced::Size>>>,
    texture_path: String,
    texture_report: String,
    texture_warning: String,
    surface_camera: u8,
    surface_yaw: i32,
    surface_pitch: i32,
    surface_navigation: view_navigation::Viewport,
    top_down_navigation: view_navigation::Viewport,
    slice_navigation: view_navigation::Viewport,
    vertical_navigation: view_navigation::Viewport,
    selected_surface: Option<(i32, i32, i32)>,
    selected_surface_face: Option<Face>,
    corner_a: Option<(i32, i32, i32)>,
    selection_plane: Option<selection_handle::SelectionBox>,
    selection_shape: mcedit2_world::editing::SelectionShape,
    corner_b: Option<(i32, i32, i32)>,
    surface_hover: Option<(i32, i32, i32)>,
    player_camera_view: Option<((usize, usize), bool)>,
    world_hover: world_hover::State,
    surface: Option<(usize, usize, std::sync::Arc<SurfaceView>)>,
    far: Option<FarProgress>,
    far_nonce: u64,
    near_nonce: u64,
    near_job: Option<NearJob>,
    near_cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    surface_request: Option<SourceRequest>,
    resource_reload_attempt: Option<ResourceRequest>,
    near_busy: bool,
    near_active_mode: Option<NearMode>,
    open_nonce: u64,
    open_busy: bool,
    open_pending: Option<(u64, PathBuf)>,
    surface_report: String,
    slice_grid: Option<(usize, usize, std::sync::Arc<Slice>)>,
    vertical_grid: Option<(usize, usize, std::sync::Arc<VerticalSlice>)>,
    vertical_area: String,
    vertical_report: String,
    /// 已扫描元数据目录；位置为后台任务的稳定身份，已打开标签由 workspace 持有。
    worlds: Vec<WorldSummary>,
    current: Option<usize>,
    dimension_index: usize,
    query_coordinates: String,
    block_result: String,
    selected_block: Option<block_names::Selection>,
    slice_area: String,
    slice_result: String,
    edit_area: String,
    edit_target: String,
    edit_filter: String,
    be_scalar_field: String,
    be_scalar_value: String,
    be_snapshot_path: String,
    entity_snapshot_path: String,
    be_journal_path: String,
    be_batch_path: String,
    plugin_manifest_path: String,
    region_batch_path: String,
    clone_destination: String,
    fill_seed: String,
    patch_path: String,
    clipboard_destination: String,
    exchange: structure_exchange::State,
    save_phrase: String,
    recover_region: String,
    recovery_phrase: String,
    save_preview: Option<SaveIntent>,
    be_save_phrase: String,
    be_prepared: Option<(usize, usize, BeScalarPreview)>,
    edit_report: String,
    sessions: BTreeMap<(usize, usize), Session>,
    prepared: Option<(usize, usize, Preview)>,
    messages: Vec<String>,
    debug: bool,
    log: bool,
    verbosity: u8,
    readonly: bool,
    readonly_global: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum AdvancedSection {
    #[default]
    Overview,
    Viewer,
    BlockEntities,
    Chunks,
    ChunkOperations,
    Search,
    Analysis,
    Replacement,
    BlockDefinitions,
    Maps,
    Quad,
    Editing,
    Files,
    Players,
    WorldInfo,
    MapsEditor,
    Structures,
    Plugins,
    Save,
}
impl AdvancedSection {
    fn label(self, language: Language) -> &'static str {
        let (zh, en) = match self {
            Self::Overview => ("存档信息", "World information"),
            Self::Viewer => ("三维显示设置", "3D display settings"),
            Self::BlockEntities => ("NBT 与实体", "NBT & entities"),
            Self::Search => ("NBT 查找与替换", "Find & replace NBT"),
            Self::Analysis => ("选区分析", "Analyze selection"),
            Self::Replacement => ("批量方块替换", "Replace blocks"),
            Self::BlockDefinitions => ("自定义方块模型", "Configure block models"),
            Self::Chunks => ("区块属性与 NBT", "Chunk properties & NBT"),
            Self::ChunkOperations => ("区块操作", "Chunk operations"),
            Self::Maps => ("二维与纵向切片", "2D & vertical maps"),
            Self::Quad => ("四视图", "Four views"),
            Self::Editing => ("方块编辑", "Block editing"),
            Self::Files => ("NBT 文件", "NBT files"),
            Self::Players => ("玩家", "Players"),
            Self::WorldInfo => ("世界属性", "World properties"),
            Self::MapsEditor => ("地图物品", "Map items"),
            Self::Structures => ("结构导入导出", "Structures"),
            Self::Plugins => ("插件", "Plugins"),
            Self::Save => ("保存与恢复", "Save & recovery"),
        };
        language.pick(zh, en)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvancedFile {
    BeBatch,
    BeJournal,
    EntitySnapshot,
    BeSnapshot,
    Structure(mcedit2_world::structure_exchange::Format, bool),
    RegionPlan,
    PluginManifest,
    Patch,
    LightTraits,
    MapImage,
}
impl AdvancedFile {
    fn filter(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::BeBatch
            | Self::BeJournal
            | Self::RegionPlan
            | Self::PluginManifest
            | Self::LightTraits => ("JSON", &["json"]),
            Self::EntitySnapshot => ("Entity snapshot", &["mcedit2entities"]),
            Self::BeSnapshot => ("Block entity snapshot", &["mcedit2be"]),
            Self::Structure(format, _) => (format.label(), format.extensions()),
            Self::Patch => ("MCEdit2 journal", &["mcedit2patch"]),
            Self::MapImage => ("图片", &["png", "jpg", "jpeg", "gif", "bmp"]),
        }
    }
}
#[derive(Debug, Clone)]
enum Message {
    WorkspacePresentation(workspace_ui::presentation::Action),
    SingleInstance(single_instance::Event),
    AtlasInspector(atlas_inspector::Action),
    About(about::Action),
    WorldBrowser(world_browser::Action),
    NativeProfile(native_profile::Event),
    ViewState(view_state::Action),
    Desktop(desktop_state::Action),
    FloatingPanel(floating_panels::Action),
    DragDrop(drag_drop::Action),
    DragDelivered(drag_drop::Proof, Box<Message>),
    WindowInput(iced::window::Id, Box<Message>),
    ToolWindow(
        iced::window::Id,
        Option<keyboard_bindings::Owner>,
        Box<Message>,
    ),
    Library(library::Action),
    History(history_ui::Action),
    Analysis(analysis_ui::Action),
    BlockReplacement(block_replacement_ui::Action),
    BlockDefinitions(block_definitions_ui::Action),
    BlockPicker(block_picker::Action),
    LogView(log_view::Action),
    SelectionDelete(deletion_ui::Action),
    ChunkOperations(chunk_operations_ui::Action),
    NbtSearch(nbt_search::Action),
    SavePostprocess(save_postprocess::Action),
    WorldSave(world_save::Action),
    RegionRecovery(region_recovery::Action),
    CancelSavePlanning,
    SavePrepared(
        (usize, usize),
        std::sync::Arc<()>,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        Result<(SaveIntent, String), String>,
    ),
    CloseRequested(iced::window::Id),
    Workspace(workspace_tabs::Action),
    TabBar(tab_bar::Action),
    MainMenu(workspace_ui::menu::Action),
    WorkspaceLayout(workspace_ui::layout::Action),
    KeyboardBindings(keyboard_bindings::Action),
    NavigationInput(navigation_bindings::Action),
    StickyInput(navigation_bindings::sticky::Action),
    Minimap(minimap::Action),
    WorkPlane(work_plane::Action),
    CameraPerspective(bool),
    WorkspaceKeyboard(iced::keyboard::Event),
    WorkspaceIdle,
    /// 已确认应用请求的下一步唤醒；不关闭文档，由外层 update 推进请求队列。
    PluginApplicationContinue,
    MapEditor(map_editor::Action),
    Transfer(transfer_edit::Action),
    Exchange(structure_exchange::Action),
    BlockEdit(block_edit::Action),
    MapRefresh(map_refresh::Ready),
    Brush(brush_stroke::Action),
    PendingClipboard(std::sync::Arc<mcedit2_world::clipboard::BlockClipboard>),
    PendingClipboardLoad,
    PendingClipboardReload(pending_import::Reload),
    PendingLoaded(
        u64,
        (usize, usize),
        std::sync::Arc<()>,
        Result<
            (
                mcedit2_world::editing::PreparedStructure,
                std::sync::Arc<pending_import::Placement>,
                std::sync::Arc<mcedit2_world::clipboard::BlockClipboard>,
                std::sync::Arc<mcedit2_world::editing::Session>,
            ),
            String,
        >,
    ),
    PendingMove(i32, i32, i32),
    Placement(placement::Action),
    Flood(flood_fill::Action),
    PendingRotate,
    PendingCancel,
    StructureCreation(structure_edit::Action),
    MetadataReady(metadata_refresh::Ready),
    NbtPath(String),
    NbtEditable(bool),
    NbtOpen,
    NbtOpenScoped(nbt_documents::OpenRequest),
    NbtOpened(
        u64,
        (usize, usize),
        Result<mcedit2_world::nbt_file::FileDocument, String>,
    ),
    Nbt(u64, nbt_editor::Action),
    NbtWorkspace(nbt_documents::Action),
    SelectionClipboard(selection_clipboard::Action),
    SelectionEdit(selection_edit::Action),
    SelectionAll(selection_edit::ExtentAction),
    PlayerBrowser(player_browser::Action),
    PlayerJump,
    PlayerInventory,
    PlayerAttributes,
    PlayerMoveToCamera,
    PlayerRotateToCamera,
    WorldInfo,
    ObjectInspector(object_inspector::Action),
    EntityDelta(String),
    EntityPrepareDelete,
    EntityPrepareTranslate,
    EntityPrepareClone,
    EntityClonePrepared(
        (usize, usize),
        scene_layers::MarkerRef,
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        Result<mcedit2_world::editing::EntityPreview, String>,
    ),
    EntityApplyPreview,
    BeTreeOperation(String),
    BeTreePrepare,
    BeTreeApply,
    NbtGroupPrepare,
    NbtGroupFinish,
    NbtGroupRollback,
    NbtGroupDone(
        (usize, usize),
        u8,
        Result<(usize, Vec<mcedit2_world::nbt_file::SaveIdentity>), String>,
    ),
    MixedPhrase(String),
    MixedPrepare,
    MixedFinish,
    MixedRollback,
    MixedReviewed(mixed_save::Reviewed),
    MixedDone((usize, usize), u8, Result<mixed_save::Completed, String>),
    Quad(quad_view::Action),
    Plugin(plugin_ui::Action),
    StartupScript(startup_script_ui::Action),
    Path(String),
    /// 原生路径与文件识别请求代次；手工路径 / 最近来源使用 None。
    Open(PathBuf, Option<u64>),
    OpenFinished(
        u64,
        Result<(WorldSummary, i32, mcedit2_world::player_catalog::Locations), String>,
    ),
    PickWorld,
    FileInput(file_input::Action),
    StructureWorkspace(u64, structure_workspace::Action),
    WorldChosen(Option<PathBuf>),
    ChooseTextureFile,
    ChooseTextureDirectory,
    NativeDialog(dialogs::Action),
    ToggleLanguage,
    ToggleAdvanced,
    AdvancedSection(AdvancedSection),
    WorkspacePane(workspace_ui::Pane),
    EditorTool(editor_options::Tool),
    WorkspaceView(workspace_ui::View),
    ChooseAdvancedFile(AdvancedFile, bool),
    Select(usize),
    Coordinates(String),
    Dimension(usize),
    TexturePath(String),
    TextureBudget(String),
    TextureAllPng(bool),
    SpectatorSampled(bool),
    TextureDiagnostics,
    LoadTextures,
    CancelTextureLoad,
    TextureProgress(u64, String),
    TextureWarning(u64, String),
    TextureBaseLoaded(u64, Option<(usize, PathBuf)>, textures::TextureBank),
    UnloadTextures,
    ToggleTextures,
    RefreshInstanceAssets,
    ToggleGpu,
    CycleGpuQuality,
    ToggleTopDown,
    AutoAssetsLoaded(
        usize,
        PathBuf,
        u64,
        Option<Result<(auto_assets::AssetStack, textures::TextureBank), String>>,
    ),
    TextureLoadFinished(
        u64,
        PathBuf,
        Result<(textures::TextureBank, Option<auto_assets::AssetStack>), String>,
    ),
    ToggleSpectator,
    SpectatorLook(f32, f32),
    SpectatorKey(iced::keyboard::Event),
    SpectatorTick(std::time::Instant),
    WorldMeshPresented,
    RuntimeRatesRefresh,
    SetSceneLayer(mcedit2_world::scene_annotations::Kind, bool),
    SetCommandLayer(mcedit2_world::scene_annotations::CommandLayer, bool),
    SceneHeightmap(mcedit2_world::chunk_display::Heightmap),
    InspectSceneLayers(bool),
    SceneLayerDiagnostics,
    SceneLayerControls,
    ReloadSceneLayers,
    SceneLayersLoaded(
        u64,
        (usize, usize),
        Result<std::sync::Arc<scene_layers::Batch>, String>,
    ),
    SceneMarkerSelected(std::sync::Arc<scene_layers::Snapshot>, usize),
    ShowEntityModels(bool),
    EntityModelVisibility(entity_visibility::Layer, bool),
    ShowOnlinePlayerSkins(bool),
    PlayerSkinLoaded(
        u64,
        [u8; 16],
        Result<std::sync::Arc<player_skin::Appearance>, String>,
    ),
    HeadSkinLoaded(
        u64,
        skull_skin::Key,
        Result<std::sync::Arc<skull_skin::Appearance>, String>,
    ),
    ProjectedEntitySelected(
        std::sync::Arc<projected_entities::Rendered>,
        scene_layers::MarkerRef,
    ),
    EntityModelsLoaded(u64, Result<std::sync::Arc<entity_geometry::Frame>, String>),
    EntityModelSelected(
        std::sync::Arc<spectator::StreamMesh>,
        scene_layers::MarkerRef,
    ),
    SpectatorFocusLost,
    SpectatorRadius(u16),
    CameraDistance(camera_distance::Action),
    SpectatorStreamBatch(u64, u64, stream_readers::Delivery),
    SpectatorStreamPlan(u64, u64, Result<spectator_stream::PreparedPlan, String>),
    SurfaceArea(String),
    InspectSurface,
    InspectVolume,
    ReloadScene,
    FollowWorld,
    RotateSurface,
    PickSurfaceFace(i32, i32, i32, Face),
    SpectatorPick(picking::Request),
    Shift3D(i32, i32, i32),
    Center3DOnPicked,
    ScenePreset(u8),
    StartFarView(u16),
    ShiftFarView(i32, i32),
    FarBatchLoaded(
        u64,
        Result<Vec<(FarChunkRequest, Result<FarChunkPatch, String>)>, String>,
    ),
    NearLoaded(u64, (usize, usize), NearResult),
    Toggle3DFrame,
    Toggle3DAxes,
    Toggle3DChunkGrid,
    Toggle3DSections,
    Cut3DTop(i32),
    Reset3DCut,
    Set3DCornerA,
    Set3DCornerB,
    Use3DBox,
    Clear3DBox,
    ZoomSurface(i8),
    Navigate(view_navigation::Target, view_navigation::Action),
    PanSurface(i32, i32),
    ResetSurfaceCamera,
    OrbitSurface(i32),
    PitchSurface(i32),
    OrbitPitchSurface(i32, i32),
    HoverSurface(view_navigation::Target, Option<(i32, i32, i32)>),
    WorldHover(world_hover::Action),
    BeScalarField(String),
    BeScalarValue(String),
    PreviewBeScalar,
    StageBeScalar,
    UndoBeScalar,
    RedoBeScalar,
    InspectStagedBe,
    InspectBeTree,
    BeSnapshotPath(String),
    ExportOriginalBe,
    VerifyOriginalBe,
    EntitySnapshotPath(String),
    InspectEntities,
    ExportEntities,
    VerifyEntities,
    BeJournalPath(String),
    ExportBeJournal,
    ImportBeJournal,
    BeBatchPath(String),
    PreviewBeBatch,
    StageBeBatch,
    PluginManifestPath(String),
    PreviewPlugin,
    InspectBlock,
    InspectOriginalBlockEntity,
    SliceText(String),
    InspectSlice,
    PickSlice(i32, i32, i32),
    MoveSlice(i32, i32, i32),
    VerticalText(String),
    InspectVertical,
    PickVertical(usize, usize),
    SelectionText(String),
    TargetText(String),
    FilterText(String),
    CloneDestination(String),
    PreviewClone,
    PreviewMove,
    FillSeed(String),
    PreviewFill,
    PreviewBrush(BrushShape),
    PatchPath(String),
    ExportPatch,
    ImportPatch,
    AnalyzeNativePatch,
    AnalyzeRegionPlan,
    ClipboardDestination(String),
    PreviewBeRegion,
    PrepareWorldSave,
    SavePhrase(String),
    ConfirmWorldSave,
    RegionBatchPath(String),
    PreviewRegionBatch,
    ExportRegionBatch,
    VerifyRegionBatch,
    PrepareBeSave,
    BeSavePhrase(String),
    ConfirmBeSave,
    RecoverRegion(String),
    RecoveryPhrase(String),
    FinalizeRegion,
    RollbackRegion,
    FinalizeRegionGroup,
    RollbackRegionGroup,
    PreviewEdit,
    ApplyEdit,
    UndoEdit,
    RedoEdit,
    DiscardEdits,
}
impl App {
    fn scene_block_layers(&self) -> block_visibility::Layers {
        if self.surface.is_some() {
            self.surface_block_layers
        } else {
            self.entity_models.visibility.block_layers()
        }
    }
    fn scene_show_textures(&self) -> bool {
        if self.surface.is_some() {
            self.surface_textures
        } else {
            self.show_textures
        }
    }
    fn scene_texture_bank(&self) -> Option<&std::sync::Arc<textures::TextureBank>> {
        if self.surface.is_some() {
            self.surface_assets.as_ref()
        } else {
            self.texture_bank.as_ref()
        }
    }
    /// 每份资源与显示方式只自动重读一次实际源窗口；不重复尝试或应用输入框草稿。
    fn ensure_resource_source(&mut self) {
        if self.far.is_some() {
            self.surface_assets = self.texture_bank.clone();
            self.surface_textures = self.show_textures;
            self.surface_block_layers = self.entity_models.visibility.block_layers();
            return;
        }
        let Some((key, mode, bounds, cut)) = self
            .surface_request
            .filter(|(key, _, _, _)| Some(*key) == self.current_key())
        else {
            return;
        };
        let assets = source_visibility::identity(
            self.texture_bank.as_ref(),
            self.show_textures,
            self.entity_models.visibility.block_layers(),
        );
        if self.surface.is_some()
            && source_visibility::identity(
                self.surface_assets.as_ref(),
                self.surface_textures,
                self.surface_block_layers,
            ) == assets
        {
            return;
        }
        let request = ((key, mode, bounds, cut), assets);
        if self.resource_reload_attempt.as_ref() == Some(&request) || self.near_busy {
            return;
        }
        if let Some(job) = &mut self.near_job {
            if matches!(job.mode, NearMode::Surface | NearMode::Volume) {
                job.assets = self.texture_bank.clone();
                job.show_textures = self.show_textures;
                job.block_layers = self.entity_models.visibility.block_layers();
                self.resource_reload_attempt = Some(request);
            }
            return;
        }
        match self.enqueue_near(key, mode, bounds, cut) {
            Ok(()) => {
                self.resource_reload_attempt = Some(request);
                self.surface_report = "资源几何已改变，正在后台重读已应用的源窗口…".into();
            }
            Err(error) => {
                self.resource_reload_attempt = Some(request);
                log::error!("[source-resources] {error}");
                self.surface_report = format!("资源对应的源窗口重读失败：{error}");
            }
        }
    }
    fn offer_block_preview(&mut self, preview: (usize, usize, Preview)) {
        block_edit::offer(self, (preview.0, preview.1), preview.2);
    }
    fn clear_block_preview(&mut self) {
        self.block_edit.cancel();
        self.prepared = None;
    }
    /// 只读的视图来源；连续工具单独核对候选链，其余编辑与保存使用正式会话。
    fn display_session(&mut self, key: (usize, usize)) -> Result<&Session, String> {
        self.current_session(key)?;
        Ok(
            if let Some(draft) = self.block_edit.draft.as_ref().filter(|d| d.key == key) {
                &draft.session
            } else if let Some((_, candidate)) = self
                .transfer
                .candidate
                .as_ref()
                .filter(|(source, _)| *source == key)
            {
                candidate
            } else if let Some(candidate) = self.plugins.candidate(key) {
                candidate
            } else if let Some(candidate) = self.nbt_search.candidate(key) {
                candidate
            } else if let Some(candidate) = self.object_inspector.candidate(key) {
                candidate
            } else {
                &self.sessions[&key]
            },
        )
    }
    fn display_source_revision(&self) -> Option<mcedit2_world::edit_sources::SourceRevision> {
        let key = self.current_key()?;
        self.block_edit
            .draft
            .as_ref()
            .filter(|d| d.key == key)
            .map(|d| d.session.display_source())
            .or_else(|| {
                self.transfer
                    .candidate
                    .as_ref()
                    .filter(|(source, _)| *source == key)
                    .map(|(_, session)| session.display_source())
            })
            .or_else(|| self.plugins.candidate(key).map(Session::display_source))
            .or_else(|| self.nbt_search.candidate(key).map(Session::display_source))
            .or_else(|| {
                self.object_inspector
                    .candidate(key)
                    .map(Session::display_source)
            })
            .or_else(|| self.sessions.get(&key).map(Session::display_source))
    }
    /// 候选场景可观察与导航，但不能作为已暂存方块供工具拾取。
    fn reviewing_block_preview(&mut self) -> bool {
        if self.block_edit.draft.is_some()
            || self.transfer.candidate.is_some()
            || self.nbt_search.reviewing()
            || self.object_inspector.reviewing()
            || self.plugins.candidate_active()
        {
            self.block_result = "正在查看尚未暂存的候选场景，请先暂存或取消预览再选取方块。".into();
            true
        } else {
            false
        }
    }
    fn restart_asset_request(&mut self) {
        self.texture_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.texture_cancelled = Default::default();
        self.texture_warning.clear();
        self.texture_load_nonce = self.texture_load_nonce.wrapping_add(1);
    }
    fn render_signature(&self) -> RenderSignature {
        RenderSignature {
            source: self.surface.as_ref().map(|(wi, di, view)| {
                (
                    *wi,
                    *di,
                    std::sync::Arc::as_ptr(view) as usize,
                    view.voxels.len(),
                    view.source_tiles.len(),
                    view.source_chunks,
                )
            }),
            geometry: self.scene_texture_bank().map(|bank| bank.visibility_id()),
            block_layers: self.scene_block_layers(),
            camera: self.surface_camera,
            yaw: self.surface_yaw,
            pitch: self.surface_pitch,
            zoom: self.surface_navigation.zoom.to_bits(),
            pan_x: self.surface_navigation.pan.x.to_bits(),
            pan_y: self.surface_navigation.pan.y.to_bits(),
            show_textures: self.scene_show_textures(),
            entity_frame: self
                .entity_models
                .frame
                .as_ref()
                .filter(|frame| {
                    frame.displays(self.entity_models.enabled, self.entity_models.visibility)
                        && frame.show_textures == self.scene_show_textures()
                        && Some(frame.snapshot.key) == self.current_key()
                        && self
                            .scene_layers
                            .snapshot
                            .as_ref()
                            .is_some_and(|s| std::sync::Arc::ptr_eq(s, &frame.snapshot))
                })
                .map(|frame| std::sync::Arc::as_ptr(frame) as usize),
        }
    }
    /// Keep a previously published view visible while a replacement snapshot
    /// for the same world/dimension is being read. Never carry it into a
    /// different world, even while a new indexing job is pending.
    fn visible_scene_is_current(&self) -> bool {
        self.surface
            .as_ref()
            .is_some_and(|(wi, di, _)| self.current_key() == Some((*wi, *di)))
    }
    /// 不同世界可启用不同数据包，即使属于同一实例也必须重新绑定颜色来源。
    fn reset_assets_for_world(&mut self, next: &WorldSummary) {
        let changed = self
            .current
            .and_then(|i| self.worlds.get(i))
            .is_some_and(|old| {
                old.root != next.root || old.world_data.origin != next.world_data.origin
            });
        if changed {
            self.restart_asset_request();
            self.texture_loading = false;
            self.texture_bank = None;
            self.manual_textures = false;
            self.texture_path.clear();
            self.texture_report.clear();
            self.auto_assets_attempted.clear();
        }
    }
    fn invalidate_near(&mut self) {
        self.near_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.near_nonce = self.near_nonce.wrapping_add(1);
        self.near_job = None;
    }
    /// 显示资源只改变三维源遮挡，不能取消不依赖资源的已提交切片读取。
    fn invalidate_resource_reads(&mut self) {
        let mode = self
            .near_job
            .as_ref()
            .map(|job| job.mode)
            .or(self.near_active_mode);
        if matches!(mode, Some(NearMode::Surface | NearMode::Volume)) {
            self.invalidate_near();
        }
    }
    /// 退役旧来源的世界网格和拾取状态，观察位置与等轴构图继续保留。
    fn retire_spectator_sources(&mut self) {
        mesh_async::cancel_spectator(&self.spectator_chunk_cache);
        self.spectator_chunk_cache = Default::default();
        self.spectator_stream = None;
        self.spectator_stream_view = None;
        self.spectator_stream_snapshot = None;
        self.spectator_fallback = None;
        self.spectator_stream_near = None;
        self.spectator_cache = Default::default();
        self.spectator_pending = Default::default();
        self.spectator_presented = Default::default();
        self.spectator_presented_view = Default::default();
    }
    fn clear_display_views(&mut self) {
        self.far = None;
        self.surface = None;
        self.resource_reload_attempt = None;
        self.slice_grid = None;
        self.vertical_grid = None;
        self.surface_hover = None;
        self.world_hover.clear();
        self.selected_block = None;
        self.grid_main.cancel();
        self.grid_horizontal.cancel();
        self.grid_vertical.cancel();
        self.grid_revision = self.grid_revision.wrapping_add(1);
    }
    /// 切换候选或真实场景时保留选框与观察位置，撤销所有旧来源的读取和拾取。
    fn refresh_scene_source(
        &mut self,
        chunks: &std::collections::BTreeSet<(i32, i32)>,
    ) -> iced::Task<Message> {
        let incremental = !chunks.is_empty()
            && self.uses_world_mesh()
            && self
                .spectator_stream
                .as_ref()
                .is_some_and(|stream| Some(stream.key) == self.current_key());
        let specs = self.map_refresh.remember(
            self.current_key(),
            self.slice_grid.as_ref().map(|(_, _, s)| s.bounds),
            self.vertical_grid.as_ref().map(|(_, _, s)| s.bounds),
        );
        let request = self.surface_request;
        self.invalidate_near();
        self.near_cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Some(key) = self.current_key() {
            let source = self
                .display_session(key)
                .expect("预览场景已有源会话")
                .clone();
            if let Some(stream) = self.spectator_stream.as_mut().filter(|s| s.key == key) {
                stream.replace_source(std::sync::Arc::new(source), chunks);
            }
        }
        mesh_async::cancel_spectator(&self.spectator_chunk_cache);
        if !incremental {
            self.spectator_stream_view = None;
            self.spectator_stream_snapshot = None;
            self.spectator_stream_near = None;
            self.spectator_fallback = None;
            self.spectator_cache = Default::default();
            self.spectator_pending = Default::default();
            self.spectator_presented = Default::default();
            self.spectator_presented_view = Default::default();
        }
        self.scene_layers.change_sources(chunks);
        let displayed_surface = incremental.then(|| self.surface.take()).flatten();
        self.clear_display_views();
        self.surface = displayed_surface;
        if let Some((key, mode, bounds, cut)) = request.filter(|r| Some(r.0) == self.current_key())
        {
            if let Err(error) = self.enqueue_near(key, mode, bounds, cut) {
                log::error!("[scene-source] 场景切换失败：{error}");
                self.surface_report = error;
            }
        }
        iced::Task::batch([
            quad_view::source_changed(self),
            map_refresh::start(self, specs),
        ])
    }
    /// 会话变更撤销未确认事务，视图更新范围由统一来源快照决定。
    fn invalidate_edit_previews(&mut self) {
        self.nbt_search.invalidate();
        self.object_inspector.invalidate();
        self.entity_clone_cancel
            .store(true, std::sync::atomic::Ordering::Release);
        transfer_edit::cancel(self);
        self.pending_import.cancel();
        plugin_ui::clear_structure(self);
        plugin_ui::invalidate(&mut self.plugins);
        self.clear_block_preview();
        self.be_prepared = None;
        self.save_preview = None;
        self.save_phrase.clear();
        self.be_save_phrase.clear();
    }
    fn enqueue_near(
        &mut self,
        key: (usize, usize),
        mode: NearMode,
        bounds: Bounds,
        cut: Option<i32>,
    ) -> Result<(), String> {
        let source = self.display_session(key)?.clone();
        self.near_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.near_cancelled = Default::default();
        if matches!(mode, NearMode::Surface | NearMode::Volume) {
            self.surface_request = Some((key, mode, bounds, cut));
            self.resource_reload_attempt = None;
        }
        self.near_nonce = self.near_nonce.wrapping_add(1);
        self.near_job = Some(NearJob {
            nonce: self.near_nonce,
            key,
            mode,
            bounds,
            cut,
            source,
            assets: self.texture_bank.clone(),
            show_textures: self.show_textures,
            block_layers: self.entity_models.visibility.block_layers(),
            cancelled: std::sync::Arc::clone(&self.near_cancelled),
        });
        Ok(())
    }
    /// 完整源快照已进入显示路径后，回收交接期间暂存的旧网格和图集。
    fn retire_spectator_fallback(&mut self) {
        if self.spectator_fallback.is_none() {
            return;
        }
        let presented = self
            .spectator_presented
            .lock()
            .expect("编辑器场景状态锁被毒化")
            .clone();
        let Some(presented) = presented.filter(|mesh| mesh.complete) else {
            return;
        };
        let current = if let Some(snapshot) = &self.spectator_stream_snapshot {
            snapshot
                .ready
                .lock()
                .expect("编辑器场景状态锁被毒化")
                .clone()
        } else {
            self.spectator_cache
                .lock()
                .expect("编辑器场景状态锁被毒化")
                .clone()
        };
        if current
            .as_ref()
            .is_some_and(|mesh| spectator::StreamMesh::same_terrain(mesh, &presented))
        {
            // 工作线程提交且显示路径已接管后，旧原点、旧图集与离场网格不再需要。
            self.spectator_fallback = None;
            log::debug!("[mesh-cache] 已释放场景交接使用的旧完整帧");
        }
    }
    /// 仅在源窗口或已核对的区块批次变化时创建新快照；相机单独更新 uniform。
    fn maintain_spectator_stream(&mut self) -> iced::Task<Message> {
        self.retire_spectator_fallback();
        if !self.uses_world_mesh() || self.show_advanced {
            return iced::Task::none();
        }
        // 资源更新时继续显示和补齐旧来源，等它对应的新近景就绪再交接。
        // scene_texture_bank 和 surface_* 使用同一已提交近景的资源快照。
        if self.spectator {
            self.spectator_camera.limit_player_fov();
        }
        let Some((wi, di, near)) = self.surface.as_ref() else {
            return iced::Task::none();
        };
        if self.current_key() != Some((*wi, *di)) {
            return iced::Task::none();
        }
        let key = (*wi, *di);
        let near = std::sync::Arc::clone(near);
        let near_identity = std::sync::Arc::as_ptr(&near) as usize;
        let (location, yaw) = match self.world_stream_pose() {
            Ok(pose) => pose,
            Err(error) => {
                log::error!("[view] 世界流式加载中心无效：{error}");
                self.surface_report = error;
                return iced::Task::none();
            }
        };
        let radius = if self.spectator_radius == 0 {
            spectator_stream::DEFAULT_RADIUS
        } else {
            self.spectator_radius
        };
        let mut newly_created = false;
        let assets = self.scene_texture_bank().cloned();
        let scene_atlas = assets.as_ref().and_then(|bank| bank.atlas()).cloned();
        if self.spectator_stream.as_ref().is_none_or(|s| {
            s.key != key
                || !s.matches_resources(
                    assets.as_ref(),
                    self.surface_textures,
                    self.surface_block_layers,
                )
        }) {
            let session = match self.display_session(key) {
                Ok(s) => std::sync::Arc::new(s.clone()),
                Err(e) => {
                    self.surface_report = e;
                    return iced::Task::none();
                }
            };
            self.spectator_stream_nonce = self.spectator_stream_nonce.wrapping_add(1);
            self.spectator_stream = match SpectatorStream::new(
                key,
                session,
                location,
                radius,
                yaw,
                self.spectator_stream_nonce,
                self.spectator_sampled,
            ) {
                Ok(s) => {
                    Some(s.with_resources(assets, self.surface_textures, self.surface_block_layers))
                }
                Err(e) => {
                    log::error!("[terrain] 启动区块流失败：{e}");
                    self.surface_report = e;
                    None
                }
            };
            self.spectator_stream_near = None;
            self.spectator_stream_view = None;
            newly_created = true;
        }
        let Some(stream) = self.spectator_stream.as_mut() else {
            return iced::Task::none();
        };
        let plan_task = if newly_created {
            stream.replan(location, radius, yaw, self.spectator_stream_nonce)
        } else if stream.requires_replan(location, radius) {
            self.spectator_stream_nonce = self.spectator_stream_nonce.wrapping_add(1);
            stream.replan(location, radius, yaw, self.spectator_stream_nonce)
        } else {
            stream.orient(yaw)
        };
        // CPU 建模尚在运行时合并来源，避免反复取消；GPU 上传独立持有不可变来源，
        // 不再要求下一批 CPU 建模等上一批 GPU 全部交接完成。
        let mesh_in_flight = *self
            .spectator_pending
            .lock()
            .expect("编辑器场景状态锁被毒化");
        let gpu_in_flight = spectator_stream::serial_reference() && {
            let cpu_ready = self
                .spectator_stream_snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .ready
                        .lock()
                        .expect("编辑器场景状态锁被毒化")
                        .clone()
                })
                .or_else(|| {
                    self.spectator_cache
                        .lock()
                        .expect("编辑器场景状态锁被毒化")
                        .clone()
                });
            cpu_ready
                .as_ref()
                .filter(|mesh| mesh.complete)
                .is_some_and(|mesh| {
                    self.spectator_presented
                        .lock()
                        .expect("编辑器场景状态锁被毒化")
                        .as_ref()
                        .is_none_or(|old| !spectator::StreamMesh::same_terrain(old, mesh))
                })
        };
        if (stream.dirty || self.spectator_stream_near != Some(near_identity))
            && !mesh_in_flight
            && (!spectator_stream::serial_reference() || !gpu_in_flight || stream.edit_refresh)
            && stream.ready_to_publish_edit()
        {
            let ready = self
                .spectator_cache
                .lock()
                .expect("编辑器场景状态锁被毒化")
                .as_ref()
                .cloned();
            let streamed = self.spectator_stream_snapshot.as_ref().and_then(|old| {
                old.ready
                    .lock()
                    .expect("编辑器场景状态锁被毒化")
                    .as_ref()
                    .cloned()
            });
            if let Some(old_surface) = self.spectator_stream_view.as_ref() {
                if ready.is_some() || streamed.is_some() {
                    self.spectator_fallback = Some(spectator::CompletedFrame {
                        vertices: std::sync::Arc::new(Vec::new()),
                        stream_mesh: self
                            .spectator_presented
                            .lock()
                            .expect("编辑器场景状态锁被毒化")
                            .clone()
                            .filter(|mesh| mesh.complete)
                            .or(streamed)
                            .or(ready),
                        surface: std::sync::Arc::clone(old_surface),
                        atlas: scene_atlas.clone(),
                    });
                }
            }
            // The UI only publishes Arc references to source patches.
            // The GPU worker composes exact seams away from the event loop.
            self.spectator_stream_snapshot = Some(std::sync::Arc::new(
                stream.snapshot(std::sync::Arc::clone(&near)),
            ));
            self.spectator_stream_view = Some(near);
            self.spectator_stream_near = Some(near_identity);
            stream.dirty = false;
            stream.edit_refresh = false;
            // The old GPU mesh worker retains orphaned immutable cache Arcs
            // and cannot overwrite a newly streamed scene's triangle buffer.
            self.spectator_cache = std::sync::Arc::new(std::sync::Mutex::new(None));
            self.spectator_pending = std::sync::Arc::new(std::sync::Mutex::new(false));
        }
        iced::Task::batch([plan_task, stream.fetch()])
    }
    fn recenter_spectator(&mut self) {
        if !self.spectator
            || self.near_busy
            || self.near_job.is_some()
            || self.selected_window().is_some()
        {
            return;
        }
        let Some(key) = self.current_key() else {
            return;
        };
        let [x, y, z] = self.spectator_camera.position;
        let Some((wi, di, source)) = self.surface.as_ref() else {
            return;
        };
        if (*wi, *di) != key || source.width == 0 || source.depth == 0 {
            return;
        }
        let bounds = Bounds {
            minimum: [source.x, source.y_min, source.z],
            maximum: [
                source.x + i32::from(source.width) - 1,
                source.y_max,
                source.z + i32::from(source.depth) - 1,
            ],
        };
        if x >= bounds.minimum[0] as f64 + 8.0
            && x <= bounds.maximum[0] as f64 - 8.0
            && z >= bounds.minimum[2] as f64 + 8.0
            && z <= bounds.maximum[2] as f64 - 8.0
            && y >= bounds.minimum[1] as f64 + 8.0
            && y <= bounds.maximum[1] as f64 - 8.0
        {
            return;
        }
        let new_bounds = view_state::source_around([x, y, z]);
        // 完整流式列已覆盖近景和目标时，飞行无需重读同一批方块、换原点或重建邻接网格。
        // 两处都核对：旧近景一旦离开显示范围便重新定位，避免远处留下独立近景几何；
        // 锚点最多保留 256 格，持续长途移动仍及时重定位，保护 f32 局部坐标精度。
        if self.uses_world_mesh()
            && [x, y, z]
                .iter()
                .zip(bounds.minimum)
                .all(|(position, origin)| (*position - f64::from(origin)).abs() <= 256.)
            && self.spectator_stream.as_ref().is_some_and(|stream| {
                stream.key == key
                    && stream.matches_resources(
                        self.scene_texture_bank(),
                        self.scene_show_textures(),
                        self.scene_block_layers(),
                    )
                    && stream.covers_exact(bounds)
                    && stream.covers_exact(new_bounds)
            })
        {
            return;
        }
        let target = scene_selection::coordinates(new_bounds);
        // At dimension height limits, clamping may not move the source
        // window any farther. Never spin an identical disk read per tick.
        if target == self.surface_area {
            return;
        }
        if self
            .enqueue_near(key, NearMode::Volume, new_bounds, None)
            .is_ok()
        {
            self.surface_area = target;
            self.surface_report =
                "Loading additional original world sections for free flight.".into();
        }
    }
    fn inspect_surface(&mut self, exposed_volume: bool) {
        self.read_scene_input(exposed_volume, false);
    }
    fn near_loaded(&mut self, nonce: u64, key: (usize, usize), result: NearResult) {
        // Exactly one reader runs at a time, including when its result is
        // obsolete. Release the slot before the nonce/identity check, so the
        // newest queued request can start even after a stale completion.
        self.near_busy = false;
        self.near_active_mode = None;
        // An earlier worker may finish after another world, region view,
        // staging operation or changed input. Its data must never publish.
        if nonce != self.near_nonce || self.current_key() != Some(key) {
            return;
        }
        match result {
            NearResult::Surface(result) => {
                self.surface_report = match result {
                    Ok((scene, assets, show_textures, block_layers)) => {
                        if source_visibility::identity(
                            self.surface_assets.as_ref(),
                            self.surface_textures,
                            self.surface_block_layers,
                        ) != source_visibility::identity(
                            assets.as_ref(),
                            show_textures,
                            block_layers,
                        ) {
                            transfer_edit::resources_changed(self);
                            if self.pending_import.resources_changed() {
                                self.edit_report = "显示资源已改变，请重新构建待放置预览".into();
                            }
                            plugin_ui::resources_changed(self);
                        }
                        let report=format!(
                            "初始观察窗口：{} 暴露方块，{} 未知列，{} 源区块。完整高度地形见流式区块统计。",
                            scene.voxels.len(),scene.missing_columns,scene.source_chunks);
                        self.grid_main.cancel();
                        self.grid_horizontal.cancel();
                        self.grid_vertical.cancel();
                        self.grid_revision = self.grid_revision.wrapping_add(1);
                        self.surface = Some((key.0, key.1, std::sync::Arc::new(scene)));
                        self.surface_assets = assets;
                        self.surface_textures = show_textures;
                        self.surface_block_layers = block_layers;
                        if self.spectator && !self.show_top_down {
                            if let Some(window) = self.selected_window().filter(|w| !w.volume) {
                                self.ensure_window_camera(window);
                            }
                        }
                        report
                    }
                    Err(e) => format!("3D source read refused: {e}"),
                };
            }
            NearResult::Horizontal(result) => {
                self.slice_result = match result {
                    Ok(slice) => {
                        let width = slice.rows.first().map_or(0, Vec::len);
                        let height = slice.rows.len();
                        let legend = if width <= 16 && height <= 16 {
                            format!("\n{}", slice.text_map())
                        } else {
                            String::new()
                        };
                        self.grid_main.cancel();
                        self.grid_horizontal.cancel();
                        self.grid_vertical.cancel();
                        self.grid_revision = self.grid_revision.wrapping_add(1);
                        self.slice_grid = Some((key.0, key.1, std::sync::Arc::new(slice)));
                        format!(
                            "Interactive {width}x{height} original 2D map. Gray=air, purple=missing chunk, amber=missing Section, gold=BE.{legend}")
                    }
                    Err(e) => format!("Horizontal source read refused: {e}"),
                };
            }
            NearResult::Vertical(result) => {
                self.vertical_report = match result {
                    Ok(slice) => {
                        let width = slice.rows.first().map_or(0, Vec::len);
                        let height = slice.rows.len();
                        let legend = if width <= 16 && height <= 16 {
                            format!("\n{}", slice.text_map())
                        } else {
                            String::new()
                        };
                        self.grid_main.cancel();
                        self.grid_horizontal.cancel();
                        self.grid_vertical.cancel();
                        self.grid_revision = self.grid_revision.wrapping_add(1);
                        self.vertical_grid = Some((key.0, key.1, std::sync::Arc::new(slice)));
                        format!("Interactive {width}x{height} vertical source map. Highest Y first; unknown source cells are not editable.{legend}")
                    }
                    Err(e) => format!("Vertical source read refused: {e}"),
                };
            }
            NearResult::WorkerStopped => {
                self.surface_report =
                    "Background source read worker stopped; no partial data accepted.".into();
                self.slice_result = self.surface_report.clone();
                self.vertical_report = self.surface_report.clone();
            }
        }
    }
    fn selection_box(&self) -> Option<Bounds> {
        let a = self.corner_a?;
        let b = self.corner_b?;
        picking::selection_bounds([a.0, a.1, a.2], [b.0, b.1, b.2]).ok()
    }
    fn set_3d_corner(&mut self, first: bool) {
        let Some(pos) = self.selected_surface else {
            self.surface_report =
                "Click a visible original 3D voxel before setting a corner.".into();
            return;
        };
        if first {
            self.corner_a = Some(pos);
        } else {
            self.corner_b = Some(pos);
        }
        self.surface_report=match (self.corner_a,self.corner_b){
            (Some(a),Some(b))=>match picking::selection_bounds([a.0,a.1,a.2],[b.0,b.1,b.2]){
                Ok(bounds)=>format!("3D selection A={a:?}, B={b:?}; {} original block positions. Use box to edit only after review.",
                    bounds.volume().unwrap_or(0)),
                Err(e)=>format!("3D box coordinates are invalid: {e}"),
            },
            _=>format!("3D {} corner set to {pos:?}; select the other corner.",
                if first{"A"}else{"B"}),
        };
    }
    fn use_3d_box(&mut self) {
        let Some(bounds) = self.selection_box() else {
            self.surface_report = "请先设置两个有效的三维选框角点。".into();
            return;
        };
        let a = bounds.minimum;
        let b = bounds.maximum;
        if let Err(error) = Bounds::spatial((a[0], a[1], a[2]), (b[0], b[1], b[2])) {
            self.edit_area.clear();
            self.clear_block_preview();
            self.be_prepared = None;
            self.save_preview = None;
            self.surface_report = format!("选框保留显示，但暂不能用于此编辑批次：{error}");
            return;
        }
        self.edit_area = format!("{},{},{};{},{},{}", a[0], a[1], a[2], b[0], b[1], b[2]);
        self.save_preview = None;
        self.clear_block_preview();
        self.be_prepared = None;
        self.surface_report=format!("Copied {}-cell original XYZ box to edit bounds; no blocks staged. Preview before Apply.",
            bounds.volume().unwrap_or(0));
    }
    fn move_cutaway(&mut self, delta: i32) {
        self.cut_scene(Some(delta));
    }
    fn shift_3d(&mut self, dx: i32, dy: i32, dz: i32) {
        let input = self
            .selected_window()
            .map(|w| scene_selection::coordinates(w.bounds))
            .unwrap_or_else(|| self.surface_area.clone());
        match shifted_scene(&input, dx, dy, dz) {
            Ok(text) => {
                self.surface_area = text;
                self.surface_cut_top = self.surface_cut_top.map(|y| y.saturating_add(dy));
                self.inspect_surface(self.surface_exposed);
            }
            Err(e) => self.surface_report = format!("3D navigation refused: {e}"),
        }
    }
    fn center_3d(&mut self) {
        let pos = self
            .selected_surface
            .or_else(|| mcedit2_world::block_lookup::parse_xyz(&self.query_coordinates).ok());
        let Some((x, y, z)) = pos else {
            self.surface_report = "Pick an original XYZ before centering.".into();
            return;
        };
        let cx = x.div_euclid(16) * 16;
        let cz = z.div_euclid(16) * 16;
        let y0 = y.saturating_sub(8);
        let y1 = y0.saturating_add(15);
        let next = format!("{cx},{y0},{cz};{},{y1},{}", cx + 15, cz + 15);
        match parse_scene_window(&next) {
            Ok(_) => {
                if !self.spectator && self.uses_world_mesh() {
                    if let Err(error) =
                        self.center_world_orthographic([x as f64, y as f64, z as f64])
                    {
                        log::error!("[view] 世界等轴定位被拒绝：{error}");
                        self.surface_report = error;
                        return;
                    }
                }
                if self.spectator {
                    self.spectator_camera.position =
                        [x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5];
                }
                self.surface_area = next;
                self.surface_cut_top = None;
                self.inspect_surface(self.surface_exposed);
            }
            Err(e) => self.surface_report = format!("3D center refused: {e}"),
        }
    }
    fn start_far_view(
        &mut self,
        radius: u16,
        requested: Option<(i32, i32)>,
    ) -> iced::Task<Message> {
        self.invalidate_near();
        let previous = self.far.take();
        self.far_nonce = self.far_nonce.wrapping_add(1);
        self.grid_main.cancel();
        self.grid_horizontal.cancel();
        self.grid_vertical.cancel();
        self.grid_revision = self.grid_revision.wrapping_add(1);
        self.surface = None;
        self.surface_cut_top = None;
        self.surface_hover = None;
        self.world_hover.clear();
        self.selected_surface = None;
        self.selected_surface_face = None;
        let Some(key) = self.current_key() else {
            self.surface_report = "Select a Java world for distant terrain.".into();
            return iced::Task::none();
        };
        let center = requested
            .or_else(|| {
                parse_xyz(&self.query_coordinates)
                    .ok()
                    .map(|(x, _, z)| (x, z))
            })
            .or_else(|| {
                parse_scene_window(&self.surface_area).ok().and_then(|v| {
                    let x = (v.minimum[0] as i64 + v.maximum[0] as i64) / 2;
                    let z = (v.minimum[2] as i64 + v.maximum[2] as i64) / 2;
                    Some((i32::try_from(x).ok()?, i32::try_from(z).ok()?))
                })
            })
            .unwrap_or((0, 0));
        // Pan/recenter keeps the immutable source snapshot so overlapping
        // chunk requests can reuse their already-decoded data. Mutations
        // clear self.far before entering this method.
        let compatible = previous.filter(|prior| prior.key == key);
        let source = if let Some(prior) = compatible.as_ref() {
            std::sync::Arc::clone(&prior.source)
        } else {
            match self.display_session(key) {
                Ok(session) => std::sync::Arc::new(session.clone()),
                Err(e) => {
                    self.surface_report = e;
                    return iced::Task::none();
                }
            }
        };
        let (y_min, y_max) = (-2048, 2047);
        let plan = match far_view::plan_far_chunks(center.0, center.1, radius) {
            Ok(plan) => plan,
            Err(e) => {
                self.surface_report = e.to_string();
                return iced::Task::none();
            }
        };
        let mut view = match far_view::empty_far_view(center.0, center.1, y_min, y_max, radius) {
            Ok(view) => view,
            Err(e) => {
                self.surface_report = e.to_string();
                return iced::Task::none();
            }
        };
        let biomes = std::sync::Arc::make_mut(&mut view.biomes);
        biomes.set_seed(source.original_world().seed);
        if let Ok(geometry) = source
            .original_world()
            .world_data
            .geometry(source.dimension())
        {
            biomes.set_geometry(geometry);
        }
        let (cache, order, cached_voxels) = compatible.map_or_else(
            || (BTreeMap::new(), VecDeque::new(), 0),
            |prior| (prior.cache, prior.cache_order, prior.cached_voxels),
        );
        // If the user pan moves a far camera, fill the new view with cached
        // authentic source patches before scheduling fresh region reads.
        let mut pending = VecDeque::new();
        let mut reused = 0;
        for request in plan {
            if let Some(patch) = cache.get(&far_cache_key(request)) {
                far_view::append_chunk(&mut view, (**patch).clone());
                reused += 1;
            } else {
                pending.push_back(request);
            }
        }
        let total = pending.len() + reused;
        self.grid_main.cancel();
        self.grid_horizontal.cancel();
        self.grid_vertical.cancel();
        self.grid_revision = self.grid_revision.wrapping_add(1);
        self.surface = Some((key.0, key.1, std::sync::Arc::new(view)));
        self.surface_assets = self.texture_bank.clone();
        self.surface_textures = self.show_textures;
        self.surface_block_layers = self.entity_models.visibility.block_layers();
        self.surface_request = None;
        self.surface_exposed = false;
        self.selected_scene = None;
        self.spectator = false;
        self.spectator_keys.clear();
        self.selected_block = None;
        self.clear_stream_scene();
        self.window_camera = None;
        self.surface_report=format!(
            "Streaming {radius} chunk radius: {reused} cached tiles, {} pending source reads. No entities or edits.",
            pending.len());
        self.far = Some(FarProgress {
            nonce: self.far_nonce,
            key,
            center,
            radius,
            source,
            total,
            complete: reused,
            errors: 0,
            remaining: pending,
            cache,
            cache_order: order,
            cached_voxels,
        });
        self.next_far_batch()
    }
    fn next_far_batch(&mut self) -> iced::Task<Message> {
        let Some(progress) = self.far.as_mut() else {
            return iced::Task::none();
        };
        if progress.remaining.is_empty() {
            return iced::Task::none();
        }
        let take = progress.remaining.len().min(far_view::batch_size_for(
            progress.remaining.front().unwrap().span,
        ));
        let requests: Vec<_> = progress.remaining.drain(..take).collect();
        let (nonce, key) = (progress.nonce, progress.key);
        let source = std::sync::Arc::clone(&progress.source);
        let (min_y, max_y) = match self.surface.as_ref() {
            Some((wi, di, view)) if (*wi, *di) == key => (view.y_min, view.y_max),
            _ => return iced::Task::none(),
        };
        // Task::perform executes Futures; synchronous region reads inside
        // the future would monopolize its executor. Move only bounded
        // read-only I/O onto an OS worker and await its one-shot response.
        iced::Task::perform(
            async move {
                let (sender, receiver) = iced::futures::channel::oneshot::channel();
                std::thread::Builder::new()
                    .name("mcedit2-far-source".into())
                    .spawn(move || {
                        let results = (|| {
                            let mut reader = source.source_reader().map_err(|e| e.to_string())?;
                            Ok(requests
                                .into_iter()
                                .map(|req| {
                                    let result = far_view::inspect_far_chunk_cached(
                                        &source,
                                        &mut reader,
                                        req,
                                        min_y,
                                        max_y,
                                    )
                                    .map_err(|e| format!("chunk {},{}: {e}", req.cx, req.cz));
                                    (req, result)
                                })
                                .collect::<Vec<_>>())
                        })();
                        if sender.send(results).is_err() {
                            log::debug!("[terrain-far] 读取完成时接收端已释放");
                        }
                    })
                    .map_err(|e| format!("无法启动远景读取线程：{e}"))?;
                receiver
                    .await
                    .map_err(|e| format!("远景读取线程未返回结果：{e}"))?
            },
            move |results| Message::FarBatchLoaded(nonce, results),
        )
    }
    fn finish_far_batch(
        &mut self,
        nonce: u64,
        results: Result<Vec<(FarChunkRequest, Result<FarChunkPatch, String>)>, String>,
    ) -> iced::Task<Message> {
        let current = self.current_key();
        let Some(progress) = self.far.as_mut() else {
            return iced::Task::none();
        };
        if progress.nonce != nonce || current != Some(progress.key) {
            return iced::Task::none();
        }
        let Some((wi, di, view)) = self.surface.as_mut() else {
            return iced::Task::none();
        };
        if (*wi, *di) != progress.key {
            return iced::Task::none();
        }
        let results = match results {
            Ok(results) => results,
            Err(error) => {
                progress.errors += 1;
                progress.remaining.clear();
                log::error!("[terrain-far] 批次读取失败：{error}");
                self.surface_report = error;
                return iced::Task::none();
            }
        };
        for (request, result) in results {
            progress.complete += 1;
            match result {
                Ok(patch) => {
                    let cached = std::sync::Arc::new(patch);
                    progress.remember(request, &cached);
                    far_view::append_chunk(std::sync::Arc::make_mut(view), (*cached).clone());
                }
                Err(e) => {
                    progress.errors += 1;
                    log::error!(
                        "[terrain-far] 原始区块 {},{} 读取失败：{e}",
                        request.cx,
                        request.cz
                    );
                }
            }
        }
        self.surface_report=format!(
            "Far terrain: {}/{} chunk tiles, {} source errors, radius {} chunks. Distant LOD is never an editable block.",
            progress.complete,progress.total,progress.errors,progress.radius);
        self.next_far_batch()
    }
    fn scene_preset(&mut self, side: u8) {
        if !matches!(side, 16 | 32 | 64) {
            return;
        }
        let current = match parse_scene_window(&self.surface_area) {
            Ok(bounds) => bounds,
            Err(e) => {
                self.surface_report = format!("Cannot resize viewport: {e}");
                return;
            }
        };
        // Preserve the existing low corner and current Y minimum. Enlarging
        // the view never stages an edit or increases the edit batch limit.
        let min = current.minimum;
        let far = match (
            min[0].checked_add(side as i32 - 1),
            min[2].checked_add(side as i32 - 1),
        ) {
            (Some(x), Some(z)) => (x, z),
            _ => {
                self.surface_report = "Viewport coordinate overflow".into();
                return;
            }
        };
        let height = if side >= 32 { 48 } else { 32 };
        let Some(y1) = min[1].checked_add(height - 1) else {
            self.surface_report = "Viewport Y overflow".into();
            return;
        };
        let next = format!(
            "{},{},{};{},{},{}",
            min[0], min[1], min[2], far.0, y1, far.1
        );
        match parse_scene_window(&next) {
            Ok(_) => {
                self.surface_area = next;
                self.surface_cut_top = None;
                self.inspect_surface(self.surface_exposed);
            }
            Err(e) => self.surface_report = format!("Viewport resize refused: {e}"),
        }
    }
    fn pick_surface(&mut self, x: i32, y: i32, z: i32) -> bool {
        if self.reviewing_block_preview() {
            return false;
        }
        self.selected_block = None;
        let Some((wi, di, view)) = self.surface.as_ref() else {
            self.selected_surface = None;
            self.selected_surface_face = None;
            self.block_result.clear();
            self.surface_report = "3D view changed; reload before editing.".into();
            log::debug!("[pick] 没有可供选择的精确源窗口 ({x},{y},{z})");
            return false;
        };
        let voxel = view
            .voxels
            .iter()
            .find(|v| v.x == x && v.y == y && v.z == z && v.span == 1);
        let Some(voxel) = voxel.filter(|_| self.current_key() == Some((*wi, *di))) else {
            self.grid_main.cancel();
            self.grid_horizontal.cancel();
            self.grid_vertical.cancel();
            self.grid_revision = self.grid_revision.wrapping_add(1);
            self.surface = None;
            self.block_result.clear();
            self.selected_surface = None;
            self.selected_surface_face = None;
            self.surface_report = "3D view changed; reload before editing.".into();
            log::debug!("[pick] 投影源窗口已改变或没有精确源方块 ({x},{y},{z})");
            return false;
        };
        self.block_result = format!(
            "源快照 ({x},{y},{z}) => {} {:?}{}",
            voxel.state.name,
            voxel.state.properties,
            if voxel.staged { " · 已暂存" } else { "" }
        );
        self.selected_block = Some(block_names::Selection {
            key: (*wi, *di),
            position: (x, y, z),
            state: voxel.state.as_ref().clone(),
        });
        self.query_coordinates = format!("{x}, {y}, {z}");
        self.selected_surface = Some((x, y, z));
        self.selected_surface_face = None;
        self.edit_area = format!("{x},{y},{z};{x},{y},{z}");
        self.be_prepared = None;
        self.save_preview = None;
        self.clear_block_preview();
        self.surface_report=format!(
            "Selected exact original 3D voxel ({x},{y},{z}). Edits still require normal guarded preview and Apply."
        );
        true
    }
    fn pick_spectator(&mut self, request: picking::Request) -> bool {
        if self.reviewing_block_preview() {
            return false;
        }
        let result =
            (|| -> Result<(), String> {
                if self.current_key() != Some(request.key) {
                    return Err("世界或维度已切换，请重新点选".into());
                }
                let presented = self
                    .spectator_presented
                    .lock()
                    .expect("编辑器场景状态锁被毒化")
                    .clone()
                    .ok_or("尚无已显示的源网格")?;
                if !std::sync::Arc::ptr_eq(&presented, &request.mesh) {
                    return Err("显示网格已更新，请重新点选".into());
                }
                let owner = request
                    .hit
                    .owner
                    .as_ref()
                    .ok_or("远景采样没有可编辑的单个源方块")?;
                let anchor = match &request.intent {
                    picking::Intent::Box(anchor) => Some(anchor),
                    _ => None,
                };
                let mut description = String::new();
                let mut created_source = false;
                for witness in std::iter::once(owner).chain(anchor) {
                    let [x, y, z] = witness.position;
                    let source = self
                        .current_session(request.key)?
                        .visible_block((x, y, z))
                        .map_err(|e| e.to_string())?;
                    match &source {
                        mcedit2_world::block_lookup::Lookup::Present(block)
                            if block.state == *witness.state => {}
                        mcedit2_world::block_lookup::Lookup::Present(_) => {
                            return Err(format!("源方块 ({x},{y},{z}) 与画面不同，请等待重新建模"))
                        }
                        _ => return Err(format!("源区块或 Section 不可用：({x},{y},{z})")),
                    }
                    if description.is_empty() {
                        created_source = self.current_session(request.key)?.creation_origin(
                            x.div_euclid(16),
                            z.div_euclid(16),
                            y,
                        );
                        description = source.description();
                        if created_source {
                            description.push_str(" · 尚未保存的新建区块或区段");
                        }
                    }
                }
                let [x, y, z] = owner.position;
                self.selected_surface = Some((x, y, z));
                self.selected_surface_face = Some(request.hit.face);
                self.query_coordinates = format!("{x}, {y}, {z}");
                self.edit_area = format!("{x},{y},{z};{x},{y},{z}");
                self.clear_block_preview();
                self.be_prepared = None;
                self.save_preview = None;
                self.block_result = description;
                self.selected_block = Some(block_names::Selection {
                    key: request.key,
                    position: (x, y, z),
                    state: (*owner.state).clone(),
                });
                match request.intent {
                    picking::Intent::Point => {
                        self.corner_a = None;
                        self.corner_b = None;
                    }
                    picking::Intent::CornerA => self.corner_a = Some((x, y, z)),
                    picking::Intent::CornerB => self.corner_b = Some((x, y, z)),
                    picking::Intent::Box(anchor) => {
                        let [a, b, c] = anchor.position;
                        self.corner_a = Some((a, b, c));
                        self.corner_b = Some((x, y, z));
                    }
                }
                self.surface_report =
                    format!(
                "已选中{}方块 ({x},{y},{z})，轴向 {:?}，模型法向 {:.2?}；编辑仍需预览与暂存。",
                if created_source {"新建暂存"} else {"源"}, request.hit.face, request.hit.normal
            );
                if self.corner_a.is_some() && self.corner_b.is_some() {
                    self.use_3d_box();
                }
                log::debug!(
                    "[pick] 已验证{}方块 ({x},{y},{z})，{}；A={:?}，B={:?}",
                    if created_source {
                        "新建暂存"
                    } else {
                        "源"
                    },
                    owner.state.name,
                    self.corner_a,
                    self.corner_b
                );
                Ok(())
            })();
        if let Err(error) = result {
            log::error!("[pick] {error}");
            self.surface_report = format!("3D 点选未接受：{error}");
            false
        } else {
            true
        }
    }
    fn inspect_vertical_view(&mut self) {
        self.invalidate_near();
        self.grid_main.cancel();
        self.grid_horizontal.cancel();
        self.grid_vertical.cancel();
        self.grid_revision = self.grid_revision.wrapping_add(1);
        self.vertical_grid = None;
        self.vertical_report = match (self.current_key(), parse_bounds(&self.vertical_area)) {
            (None, _) => "Select a Java world first.".into(),
            (_, Err(e)) => format!("Invalid vertical area: {e}"),
            (Some(key), Ok(bounds)) => {
                match self.enqueue_near(key, NearMode::Vertical, bounds, None) {
                    Ok(()) => "Reading vertical source plane in a read-only worker.".into(),
                    Err(e) => format!("Vertical source refused: {e}"),
                }
            }
        };
    }
    fn pick_vertical(&mut self, row: usize, col: usize) {
        if self.reviewing_block_preview() {
            return;
        }
        let Some((wi, di, plane)) = self.vertical_grid.as_ref() else {
            return;
        };
        if self.current_key() != Some((*wi, *di)) {
            self.grid_main.cancel();
            self.grid_horizontal.cancel();
            self.grid_vertical.cancel();
            self.grid_revision = self.grid_revision.wrapping_add(1);
            self.vertical_grid = None;
            self.vertical_report = "Vertical plane stale; re-inspect selected dimension.".into();
            return;
        }
        let Some((x, y, z)) = plane.coordinate(row, col) else {
            return;
        };
        if !matches!(
            plane.rows.get(row).and_then(|line| line.get(col)),
            Some(SliceCell::Present { .. })
        ) {
            self.vertical_report = "Cannot pick missing original chunk/Section.".into();
            return;
        }
        self.edit_area = format!("{x},{y},{z};{x},{y},{z}");
        self.query_coordinates = format!("{x}, {y}, {z}");
        if let Some(SliceCell::Present { state, .. }) =
            plane.rows.get(row).and_then(|line| line.get(col))
        {
            self.selected_block = Some(block_names::Selection {
                key: (*wi, *di),
                position: (x, y, z),
                state: state.clone(),
            });
        }
        self.be_prepared = None;
        self.save_preview = None;
        self.clear_block_preview();
        self.block_result=format!("Selected vertical plane original block at ({x},{y},{z}); inspect the source Block Entity before editing.");
    }
    fn inspect_slice_view(&mut self) {
        self.invalidate_near();
        self.grid_main.cancel();
        self.grid_horizontal.cancel();
        self.grid_vertical.cancel();
        self.grid_revision = self.grid_revision.wrapping_add(1);
        self.slice_grid = None;
        self.slice_result = match (self.current_key(), parse_bounds(&self.slice_area)) {
            (None, _) => "Select an indexed Java world first.".into(),
            (_, Err(e)) => format!("Invalid 2D slice bounds: {e}"),
            (Some(key), Ok(bounds)) => {
                match self.enqueue_near(key, NearMode::Horizontal, bounds, None) {
                    Ok(()) => "Reading horizontal source slice in a read-only worker.".into(),
                    Err(e) => format!("Horizontal source refused: {e}"),
                }
            }
        };
    }
    fn pan_slice(&mut self, dx: i32, dy: i32, dz: i32) {
        match parse_bounds(&self.slice_area)
            .map_err(|e| e.to_string())
            .and_then(|area| shifted_slice(area, dx, dy, dz))
        {
            Ok(next) => {
                self.slice_area = next;
                self.inspect_slice_view();
            }
            Err(e) => self.slice_result = format!("Cannot move the slice: {e}"),
        }
    }
    fn pick_slice(&mut self, x: i32, y: i32, z: i32) {
        if self.reviewing_block_preview() {
            return;
        }
        let Some((world, dimension, slice)) = self.slice_grid.as_ref() else {
            self.slice_result = "No current interactive slice to select.".into();
            return;
        };
        if self.current_key() != Some((*world, *dimension)) {
            self.grid_main.cancel();
            self.grid_horizontal.cancel();
            self.grid_vertical.cancel();
            self.grid_revision = self.grid_revision.wrapping_add(1);
            self.slice_grid = None;
            self.slice_result = "World or dimension switched; reload the slice.".into();
            return;
        }
        if !(slice.bounds.minimum[0]..=slice.bounds.maximum[0]).contains(&x)
            || !(slice.bounds.minimum[2]..=slice.bounds.maximum[2]).contains(&z)
            || y != slice.bounds.minimum[1]
        {
            self.slice_result = "Clicked coordinates outside original slice.".into();
            return;
        }
        let row = (z - slice.bounds.minimum[2]) as usize;
        let col = (x - slice.bounds.minimum[0]) as usize;
        let Some(cell) = slice.rows.get(row).and_then(|cells| cells.get(col)) else {
            self.slice_result = "Slice contains no block at selected coordinate.".into();
            return;
        };
        let detail = match cell {
            SliceCell::MissingChunk => "Missing original chunk; edits prohibited".into(),
            SliceCell::MissingSection => "Missing original Section; edits prohibited".into(),
            SliceCell::Present {
                state,
                original_block_entity: true,
                staged_creation,
                ..
            } => format!(
                "{} (Block Entity: replacement prohibited){}",
                mcedit2_world::editing::render_state(state),
                if *staged_creation {
                    " [未保存的创建区域]"
                } else {
                    ""
                }
            ),
            SliceCell::Present {
                state,
                original_block_entity: false,
                staged_creation,
                ..
            } => format!(
                "{}{}",
                mcedit2_world::editing::render_state(state),
                if *staged_creation {
                    " [未保存的创建区域]"
                } else {
                    ""
                }
            ),
        };
        self.selected_block = match cell {
            SliceCell::Present { state, .. } => Some(block_names::Selection {
                key: (*world, *dimension),
                position: (x, y, z),
                state: state.clone(),
            }),
            _ => None,
        };
        self.query_coordinates = format!("{x}, {y}, {z}");
        self.edit_area = format!("{x},{y},{z};{x},{y},{z}");
        self.be_prepared = None;
        self.clear_block_preview();
        self.save_preview = None;
        self.save_phrase.clear();
        self.block_result = format!("Selected interactive 2D cell ({x},{y},{z}): {detail}");
    }
    fn current_world_context(&self) -> Result<(usize, usize, WorldSummary, String), String> {
        let (wi, di) = self
            .current_key()
            .ok_or("Select an original world and dimension")?;
        let world = self
            .worlds
            .get(wi)
            .ok_or("Selected world disappeared")?
            .clone();
        let dimension = world
            .dimensions
            .get(di)
            .ok_or("Selected dimension disappeared")?
            .name
            .clone();
        Ok((wi, di, world, dimension))
    }
    fn prepare_save(&mut self) -> iced::Task<Message> {
        self.prepare_save_mode(false)
    }
    fn prepare_be_save(&mut self) -> iced::Task<Message> {
        self.prepare_save_mode(true)
    }
    fn prepare_save_mode(&mut self, require_nbt: bool) -> iced::Task<Message> {
        if self.save_worker {
            return iced::Task::none();
        }
        self.save_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        self.save_cancelled = Default::default();
        self.save_planning = false;
        self.save_preview = None;
        self.save_phrase.clear();
        self.be_save_phrase.clear();
        self.be_prepared = None;
        self.clear_block_preview();
        let result = (|| -> Result<_, String> {
            if self.readonly {
                return Err("-view forbids writing Minecraft world files.".into());
            }
            let (wi, di, world, dimension) = self.current_world_context()?;
            if mixed_save::needs_joint(self, &world.root) {
                return Err(
                    "此存档还有其他维度、NBT 文件或地图修改，请使用保存页上方的联合保存".into(),
                );
            }
            let timestamp = u32::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_secs(),
            )
            .map_err(|_| "系统时间超出 Region 时间戳范围")?;
            let request = self.save_postprocess.request(&world, &dimension);
            let session = self.current_session((wi, di))?.clone();
            Ok(((wi, di), world, session, request, timestamp))
        })();
        let (key, world, session, request, timestamp) = match result {
            Ok(v) => v,
            Err(error) => {
                log::error!("[save] {error}");
                self.edit_report = format!("保存预检被拒绝：{error}");
                return iced::Task::none();
            }
        };
        self.save_planning = true;
        self.save_worker = true;
        self.edit_report = "正在后台核验源区块并处理高度 / 光照；尚未写入世界。".into();
        let token = self.save_cancelled.clone();
        let worker = token.clone();
        let revision = session.revision();
        iced::Task::perform(
            nbt_background(move || {
                if worker.load(std::sync::atomic::Ordering::Acquire) {
                    return Err("保存预检已取消".into());
                }
                let policy = request.policy(&world)?;
                let plans = save_postprocess::Candidates::prepare_auto_cancellable(
                    &session,
                    &policy,
                    timestamp,
                    || worker.load(std::sync::atomic::Ordering::Acquire),
                )?;
                let be_mode = plans.requires_nbt_confirmation();
                if require_nbt && !be_mode {
                    return Err("当前会话没有需要 NBT 确认的实体或方块实体修改".into());
                }
                if let Some(native) = plans.native().filter(|native| !native.is_empty()) {
                    validate_region_group(&world, &world.dimensions[key.1].name, native)
                        .map_err(|e| e.to_string())?;
                }
                if worker.load(std::sync::atomic::Ordering::Acquire) {
                    return Err("保存预检已取消".into());
                }
                let (regions, chunks, bytes) = plans.stats();
                let phrase = if be_mode { "SAVE_BE_NBT" } else { "SAVE" };
                let report=format!("保存预检：{regions} 个文件，涉及 {chunks} 个区块，{bytes} 字节候选（包含需要重算的邻块）。本次尚未修改文件。关闭 Minecraft 后输入 {phrase} 确认保存。");
                Ok((
                    SaveIntent {
                        key,
                        timestamp,
                        candidates: plans,
                        policy,
                        be_mode,
                    },
                    report,
                ))
            }),
            move |result| Message::SavePrepared(key, revision.clone(), token.clone(), result),
        )
    }
    fn confirm_be_save(&mut self) -> iced::Task<Message> {
        world_save::confirm(self, true)
    }
    fn confirm_save(&mut self) -> iced::Task<Message> {
        world_save::confirm(self, false)
    }
    fn recover_region(&mut self, rollback: bool) -> Task<Message> {
        self.save_preview = None;
        self.save_phrase.clear();
        self.clear_block_preview();
        let confirmation = if rollback { "ROLLBACK" } else { "FINALIZE" };
        if self.readonly || self.recovery_phrase != confirmation {
            self.edit_report =
                format!("Enter {confirmation} exactly; -view forbids recovery writes.");
            return Task::none();
        }
        self.recovery_phrase.clear();
        let scope = parse_region_name(&self.recover_region).map(|(x, z, extension)| {
            region_recovery::Scope::Single {
                x,
                z,
                extension: extension.to_owned(),
            }
        });
        match scope {
            Ok(scope) => region_recovery::run(self, scope, rollback),
            Err(error) => {
                log::error!("[region-recovery] {error}");
                self.edit_report = format!("Region 恢复未执行：{error}");
                Task::none()
            }
        }
    }
    fn recover_group(&mut self, rollback: bool) -> Task<Message> {
        let confirmation = if rollback {
            "ROLLBACK_GROUP"
        } else {
            "FINALIZE_GROUP"
        };
        if self.readonly || self.recovery_phrase != confirmation {
            self.edit_report = format!("请输入 {confirmation}；只读模式不能修改恢复记录。");
            return Task::none();
        }
        self.save_preview = None;
        self.clear_block_preview();
        self.recovery_phrase.clear();
        let context = (|| -> Result<_, String> {
            let (_, _, world, _) = self.current_world_context()?;
            let pending = mcedit2_world::mixed_transaction::pending_summary(&world)
                .map_err(|e| e.to_string())?;
            Ok(pending)
        })();
        let pending = match context {
            Ok(value) => value,
            Err(error) => {
                log::error!("[group-recovery] {error}");
                self.edit_report = format!("保存组恢复未完成：{error}");
                return Task::none();
            }
        };
        if let Some(pending) = pending {
            return mixed_save::recover(self, pending, rollback);
        }
        region_recovery::run(self, region_recovery::Scope::Group, rollback)
    }
    fn boot(
        args: launch::Arguments,
        desktop_enabled: bool,
    ) -> Result<(Self, iced::Task<Message>), String> {
        let (view_state, saved) = view_state::State::load()?;
        let desktop = desktop_state::State::new(saved, desktop_enabled);
        let library = library::State::restore(&desktop.saved.library);
        let world_browser = world_browser::State::restore(&desktop.saved.world_browser);
        let language = desktop.language();
        let initial_radius = if desktop_enabled {
            desktop.saved.camera_distance.radius
        } else {
            spectator_stream::DEFAULT_RADIUS
        };
        let mut app = App {
            view_state,
            nbt_search: nbt_search::State::load()?,
            block_definitions: block_definitions_ui::State::load()?,
            desktop,
            world_browser,
            library,
            main_window: None,
            minimap: minimap::State::default(),
            work_plane: work_plane::State::default(),
            language,
            debug: args.debug,
            log: args.log,
            verbosity: args.verbosity,
            readonly: args.view,
            readonly_global: args.view,
            query_coordinates: "0, 64, 0".into(),
            edit_area: "0, 64, 0; 0, 64, 0".into(),
            slice_area: "0, 64, 0; 15, 64, 15".into(),
            edit_target: "minecraft:stone".into(),
            clone_destination: "0, 64, 0".into(),
            fill_seed: "0, 64, 0".into(),
            clipboard_destination: "0, 64, 0".into(),
            vertical_area: "0,-64,0;15,-49,0".into(),
            surface_area: "0,-64,0;15,-49,15".into(),
            surface_pitch: 35,
            surface_frame: true,
            surface_axes: true,
            surface_chunk_grid: true,
            surface_section_guides: false,
            show_textures: true,
            spectator_radius: initial_radius,
            spectator_sampled: false,
            use_gpu: true,
            ..Default::default()
        };
        desktop_state::restore_panels(&mut app);
        brush_stroke::restore(&mut app);
        selection_preferences::restore(&mut app);
        let mut initial_y = 64;
        let mut tasks = Vec::new();
        let mut active = None;
        let mut files = false;
        for path in &args.worlds {
            let path = std::path::absolute(path).map_err(|error| error.to_string())?;
            let kind = match file_input::classify(&path) {
                Ok(kind) => kind,
                Err(error) => {
                    let error = format!("{}：{error}", path.display());
                    app.messages.push(error.clone());
                    tasks.push(file_input::report(&mut app, error));
                    continue;
                }
            };
            match kind {
                file_input::Kind::Structure(format) => {
                    tasks.push(structure_workspace::update(
                        &mut app,
                        0,
                        structure_workspace::Action::Open(path, format),
                    ));
                    active = app.structures.active.map(tab_bar::Id::Structure);
                    files = false;
                    continue;
                }
                file_input::Kind::Nbt => {
                    let Some(world) = app.current else {
                        let error = format!("打开 {} 前请先在参数中指定所属世界", path.display());
                        app.messages.push(error.clone());
                        tasks.push(file_input::report(&mut app, error));
                        continue;
                    };
                    app.structures.active = None;
                    app.show_advanced = true;
                    app.advanced_section = AdvancedSection::Files;
                    app.nbt_path = path.to_string_lossy().into_owned();
                    tasks.push(nbt_documents::open_path(&mut app, path));
                    active = Some(tab_bar::Id::World(world));
                    files = true;
                    continue;
                }
                file_input::Kind::World => (),
            }
            match inspect_input(&path) {
                Ok(world) => {
                    log::info!(
                        "打开启动参数中的世界：{}；DataVersion {:?}",
                        world.root.display(),
                        world.data_version
                    );
                    app.structures.active = None;
                    let locations = view_state::locations(&world);
                    app.source_locations.insert(world.root.clone(), locations);
                    app.dimension_index = app.initial_dimension(&world);
                    initial_y = initial_scene_height(&world, app.dimension_index);
                    let index = if let Some(index) =
                        app.worlds.iter().position(|old| old.root == world.root)
                    {
                        index
                    } else {
                        app.worlds.push(world);
                        app.worlds.len() - 1
                    };
                    app.workspace.open(index);
                    app.current = Some(index);
                    active = Some(tab_bar::Id::World(index));
                    files = false;
                    app.path = path.to_string_lossy().into_owned();
                    desktop_state::opened(&mut app, index);
                    app.block_result.clear();
                    app.slice_result.clear();
                    app.clear_block_preview();
                    app.edit_report.clear();
                }
                Err(e) => {
                    let error = format!("{}：{e}", path.display());
                    app.messages.push(error.clone());
                    tasks.push(file_input::report(&mut app, error));
                }
            }
        }
        app.structures.active = None;
        if app.current_key().is_some() {
            app.open_default_scene(initial_y);
            let current = app.current.expect("已经选择初始存档");
            desktop_state::opened(&mut app, current);
            app.remember_view();
            selection_edit::remember(&mut app);
        }
        if let Some(job) = app.near_job.take() {
            app.near_busy = true;
            app.near_active_mode = Some(job.mode);
            tasks.push(dispatch_near(job));
        }
        if let Some(active) = active {
            tasks.push(workspace_tabs::activate_tab(&mut app, active));
        }
        if files {
            app.show_advanced = true;
            app.advanced_section = AdvancedSection::Files;
            workspace_tabs::remember(&mut app);
        }
        desktop_state::remember(&mut app);
        if let Some(source) = args.eval {
            let source = source
                .into_string()
                .map_err(|_| "-eval 源码必须是有效 UTF-8")?;
            tasks.push(startup_script_ui::enqueue(&mut app, source)?);
        }
        Ok((app, iced::Task::batch(tasks)))
    }

    /// Read-only first scene uses the existing single bounded background reader.
    fn open_default_scene(&mut self, terrain_y: i32) {
        if self.current_key().is_none() {
            return;
        }
        self.selected_scene = None;
        self.window_camera = None;
        self.world_orthographic_framing = Default::default();
        self.surface_navigation = Default::default();
        self.top_down_navigation = Default::default();
        self.slice_navigation = Default::default();
        self.vertical_navigation = Default::default();
        self.surface_camera = 0;
        self.surface_yaw = 0;
        self.surface_pitch = 35;
        self.surface_cut_top = None;
        self.show_advanced = false;
        self.show_top_down = false;
        self.spectator = true;
        self.use_gpu = true;
        self.surface_frame = false;
        self.surface_axes = false;
        self.surface_chunk_grid = false;
        self.surface_section_guides = false;
        if let Some(key) = self.current_key() {
            self.quad.restore_navigation(key, None);
        }
        self.spectator_keys.clear();
        self.spectator_stream = None;
        self.spectator_stream_view = None;
        self.spectator_stream_near = None;
        self.spectator_chunk_cache = Default::default();
        if self.spectator_radius == 0 {
            self.spectator_radius = spectator_stream::DEFAULT_RADIUS;
        }
        self.restore_initial_view(terrain_y);
    }
    fn current_key(&self) -> Option<(usize, usize)> {
        self.current.map(|index| (index, self.dimension_index))
    }
    fn current_session(&mut self, key: (usize, usize)) -> Result<&mut Session, String> {
        if !self.sessions.contains_key(&key) {
            let world = self
                .worlds
                .get(key.0)
                .ok_or("Selected world is no longer open")?
                .clone();
            let dimension = world
                .dimensions
                .get(key.1)
                .ok_or("Select an indexed original world dimension")?
                .name
                .clone();
            let session = Session::new(world, dimension).map_err(|e| e.to_string())?;
            self.sessions.insert(key, session);
        }
        self.bind_entity_contexts();
        self.sessions
            .get_mut(&key)
            .ok_or_else(|| "Missing editor session".into())
    }

    /// 每个维度共享其他已修改维度的实体来源，投影由会话按本地代次缓存。
    fn bind_entity_contexts(&mut self) {
        let peers = self
            .sessions
            .iter_mut()
            .filter_map(|(key, session)| {
                session.has_pending_entity_changes().then(|| {
                    (
                        *key,
                        session.original_world().root.clone(),
                        session.identity_peer_snapshot(),
                    )
                })
            })
            .collect::<Vec<_>>();
        for (key, session) in &mut self.sessions {
            let sources = peers
                .iter()
                .filter(|(peer_key, root, _)| {
                    peer_key != key && root == &session.original_world().root
                })
                .map(|(_, _, peer)| std::sync::Arc::clone(peer))
                .collect();
            session.set_identity_peers(sources);
        }
    }
}
/// A read-only near-view publication must not evict the completed spectator
/// mesh. maintain_spectator_stream will snapshot it with its original origin
/// before swapping in the refreshed combined source.
fn must_reset_spectator_mesh(
    before: &RenderSignature,
    after: &RenderSignature,
    read_only_near_publish: bool,
) -> bool {
    before.geometry != after.geometry
        || before.block_layers != after.block_layers
        || before.show_textures != after.show_textures
        || (before.source != after.source && !read_only_near_publish)
}
async fn nbt_background<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let result = work();
        if let Err(error) = &result {
            log::error!("[nbt] 后台任务失败：{error}");
        }
        if tx.send(result).is_err() {
            log::debug!("[nbt] 后台结果接收端已关闭");
        }
    });
    rx.await.map_err(|e| format!("NBT 后台任务未返回：{e}"))?
}
fn update(app: &mut App, message: Message) -> iced::Task<Message> {
    if let Message::DragDrop(action) = message {
        return drag_drop::update(app, action);
    }
    app.bind_entity_contexts();
    if matches!(&message, Message::CloseRequested(id) if app.main_window == Some(*id)) {
        app.about.open = false;
    }
    if matches!(
        &message,
        Message::TargetText(_)
            | Message::FillSeed(_)
            | Message::StructureWorkspace(
                _,
                structure_workspace::Action::Replacement(_)
                    | structure_workspace::Action::FloodSeed(_)
            )
    ) {
        flood_fill::parameters_changed(app);
    }
    let placement_change = placement::remember(app, &message);
    if matches!(&message, Message::ReloadScene) {
        app.world_hover.invalidate();
    }
    let hover_owner = (app.current_key(), app.structures.active);
    let task = update_inner(app, message);
    app.bind_entity_contexts();
    if hover_owner != (app.current_key(), app.structures.active) {
        app.surface_hover = None;
        app.world_hover.clear();
    }
    placement::maintain(app, placement_change);
    block_picker::maintain(app);
    keyboard_bindings::sync(app);
    drag_drop::maintain(app);
    tab_bar::maintain(app);
    history_ui::maintain(app);
    analysis_ui::maintain(app);
    block_definitions_ui::maintain(app);
    chunk_operations_ui::maintain(app);
    selection_edit::maintain_extent(app);
    structure_workspace::selection::maintain(app);
    player_editor::remember_camera_view(app);
    minimap::maintain(app);
    desktop_state::remember(app);
    placement_target::maintain(app);
    let completion_task = transfer_edit::maintain_completed(app);
    let structure_import_task = structure_workspace::import::maintain(app);
    let structure_completion_task = structure_workspace::import::maintain_history(app);
    iced::Task::batch([
        task,
        drag_drop::focus_task(app),
        completion_task,
        structure_completion_task,
        startup_script_ui::maintain(app),
        file_input::maintain(app),
        plugin_ui::application::maintain(app),
        metadata_pages::maintain(app),
        metadata_refresh::maintain(app),
        world_hover::maintain(app),
        world_browser::maintain(app),
        atlas_inspector::maintain(app),
        plugin_ui::reload::maintain(app),
        log_view::maintain(app),
        nbt_search::flush(app),
        object_inspector::flush(app),
        object_inspector::ray::maintain(app),
        object_inspector::cell::maintain(app),
        navigation_bindings::sticky::flush(app),
        structure_import_task,
        structure_workspace::plugin::maintain(app),
        plugin_ui::maintain_tool(app),
        structure_workspace::maintain_nbt(app),
        structure_workspace::brush::maintain(app),
    ])
}
fn update_inner(app: &mut App, message: Message) -> iced::Task<Message> {
    workspace_tabs::sync_readonly(app);
    let task = update_routed(app, message);
    workspace_tabs::sync_readonly(app);
    task
}
fn update_routed(app: &mut App, message: Message) -> iced::Task<Message> {
    let message = match message {
        Message::DragDelivered(proof, message) => {
            let task = update_inner(app, *message);
            drag_drop::delivered(app, &proof);
            return task;
        }
        Message::ToolWindow(id, owner, message) => {
            return floating_panels::ui(app, id, owner, *message)
        }
        Message::WindowInput(id, message) => {
            if app.main_window == Some(id) {
                *message
            } else {
                return floating_panels::input(app, id, *message);
            }
        }
        Message::FloatingPanel(action) => return floating_panels::update(app, action),
        Message::DragDrop(action) => return drag_drop::update(app, action),
        Message::CloseRequested(id) if app.floating_panels.owns(id) => {
            return floating_panels::update(app, floating_panels::Action::Close(id))
        }
        message => message,
    };
    if let Message::Open(_, epoch) = &message {
        if !file_input::allows_world_open(app, *epoch) {
            return file_input::report(
                app,
                "正在按参数顺序打开文件；请完成或取消后续打开，再打开其他世界",
            );
        }
    }
    let _native_span = iced_wgpu::window::frame_statistics::span("app_update");
    app.remember_view();
    workspace_tabs::remember(app);
    selection_edit::remember(app);
    let message = match message {
        Message::SpectatorTick(at) if app.structures.active.is_some() => {
            return structure_workspace::flight_tick(app, at)
        }
        Message::Desktop(action) => return desktop_state::update(app, action),
        Message::NativeDialog(action) => return dialogs::update(app, action),
        Message::SingleInstance(event) => return single_instance::update(app, event),
        Message::About(action) => return about::update(app, action),
        Message::AtlasInspector(action) => return atlas_inspector::update(app, action),
        Message::WorldBrowser(action) => return world_browser::update(app, action),
        Message::Library(action) => return library::update(app, action),
        Message::History(action) => return history_ui::update(app, action),
        Message::Analysis(action) => return analysis_ui::update(app, action),
        Message::LogView(action) => return log_view::update(app, action),
        Message::FileInput(action) => return file_input::update(app, action),
        Message::StructureWorkspace(id, action) => {
            return structure_workspace::update(app, id, action)
        }
        Message::Workspace(action) => return workspace_tabs::update(app, action),
        Message::TabBar(action) => return tab_bar::update(app, action),
        Message::MainMenu(action) => return workspace_ui::menu::update(app, action),
        Message::WorkspaceLayout(action) => return workspace_ui::layout::update(app, action),
        Message::WorkspacePresentation(action) => {
            return workspace_ui::presentation::update(app, action)
        }
        Message::KeyboardBindings(action) => return keyboard_bindings::update(app, action),
        Message::NavigationInput(action) => return navigation_bindings::update(app, action),
        Message::Minimap(action) => return minimap::update(app, action),
        Message::WorkPlane(action) => return work_plane::update(app, action),
        Message::CameraPerspective(perspective) => {
            desktop_state::camera_projection(app, perspective);
            return iced::Task::none();
        }
        Message::StickyInput(action) => return navigation_bindings::sticky::update(app, action),
        Message::WorkspaceKeyboard(event) => return workspace_tabs::keyboard(app, event),
        Message::CloseRequested(id) => {
            return workspace_tabs::update(
                app,
                workspace_tabs::Action::Close(workspace_tabs::CloseTarget::Window(id)),
            )
        }
        message => message,
    };
    let player_source_view = matches!(
        &message,
        Message::ViewState(view_state::Action::PlayerSource(..))
    );
    let location_jump = match view_state::dimension_jump(app, &message) {
        Ok(jump) => jump,
        Err(error) => {
            log::error!("[view-state] 定位失败：{error}");
            app.view_state.status = error;
            return iced::Task::none();
        }
    };
    let message = if let Some((index, _)) = &location_jump {
        Message::Dimension(*index)
    } else {
        message
    };
    let before = app.render_signature();
    let source_revision = app
        .current_key()
        .map(|key| (key, app.sessions.get(&key).map(Session::revision)));
    let source_content = app
        .current_key()
        .and_then(|key| app.sessions.get(&key))
        .map(Session::display_source);
    let source_was_displayed = app.surface.is_some()
        || app
            .surface_request
            .is_some_and(|r| Some(r.0) == app.current_key());
    let display_source = app.display_source_revision();
    let was_flying =
        navigation_bindings::perspective_active(app) && has_flight_input(&app.spectator_keys);
    let was_world_mesh = app.uses_world_mesh();
    let was_world_visible = !app.show_top_down
        && !app.show_advanced
        && ((app.spectator && app.use_gpu) || was_world_mesh);
    if matches!(
        &message,
        Message::ToggleTopDown
            | Message::ToggleSpectator
            | Message::ToggleGpu
            | Message::ToggleAdvanced
            | Message::AdvancedSection(_)
            | Message::WorkspaceView(_)
            | Message::CycleGpuQuality
    ) {
        mesh_async::cancel_isometric(&app.render_gpu_pending);
        app.quad.cancel_projection();
    }
    let publishing_source_material = matches!(&message,
        Message::NearLoaded(nonce,key,NearResult::Surface(Ok((_,assets,show_textures,block_layers))))
            if *nonce==app.near_nonce && Some(*key)==app.current_key()
                && source_visibility::identity(assets.as_ref(),*show_textures,*block_layers)!=source_visibility::identity(app.surface_assets.as_ref(),app.surface_textures,app.surface_block_layers));
    let publishing_material = publishing_source_material
        || matches!(
            &message,
            Message::TextureLoadFinished(..)
                | Message::AutoAssetsLoaded(..)
                | Message::TextureBaseLoaded(..)
                | Message::UnloadTextures
                | Message::ToggleTextures
        );
    let old_complete = if publishing_material {
        let mesh = app
            .spectator_cache
            .lock()
            .expect("编辑器场景状态锁被毒化")
            .as_ref()
            .cloned();
        let streamed = app.spectator_stream_snapshot.as_ref().and_then(|old| {
            old.ready
                .lock()
                .expect("编辑器场景状态锁被毒化")
                .as_ref()
                .cloned()
        });
        app.spectator_stream_view.as_ref().and_then(|surface| {
            (mesh.is_some() || streamed.is_some()).then(|| spectator::CompletedFrame {
                vertices: std::sync::Arc::new(Vec::new()),
                stream_mesh: app
                    .spectator_presented
                    .lock()
                    .expect("编辑器场景状态锁被毒化")
                    .clone()
                    .filter(|mesh| mesh.complete)
                    .or(streamed)
                    .or(mesh),
                surface: std::sync::Arc::clone(surface),
                atlas: app.scene_texture_bank().and_then(|b| b.atlas()).cloned(),
            })
        })
    } else {
        None
    };
    let read_only_near_publish =
        matches!(&message, Message::NearLoaded(..) | Message::MapRefresh(..));
    if matches!(
        &message,
        Message::SelectionText(_)
            | Message::Use3DBox
            | Message::PickSlice(..)
            | Message::PickVertical(..)
            | Message::PickSurfaceFace(..)
            | Message::SpectatorPick(..)
    ) {
        plugin_ui::invalidate(&mut app.plugins);
    }
    // Source mutations or stale-view navigation revoke all outstanding
    // near-field worker snapshots. Rendering-only camera/texture events do
    // not revoke an otherwise valid in-flight read.
    if matches!(
        &message,
        Message::Open(..)
            | Message::Select(_)
            | Message::Dimension(_)
            | Message::WorkspaceIdle
            | Message::ConfirmWorldSave
            | Message::ConfirmBeSave
            | Message::FinalizeRegion
            | Message::RollbackRegion
            | Message::FinalizeRegionGroup
            | Message::RollbackRegionGroup
            | Message::SliceText(_)
            | Message::VerticalText(_)
    ) {
        app.invalidate_near();
        app.quad.invalidate();
    }
    let invalidate_stream = matches!(
        &message,
        Message::Open(..)
            | Message::Select(_)
            | Message::Dimension(_)
            | Message::WorkspaceIdle
            | Message::ConfirmWorldSave
            | Message::ConfirmBeSave
            | Message::FinalizeRegion
            | Message::RollbackRegion
            | Message::FinalizeRegionGroup
            | Message::RollbackRegionGroup
    );
    if invalidate_stream
        || matches!(
            &message,
            Message::ReloadScene
                | Message::SceneMarkerSelected(..)
                | Message::ProjectedEntitySelected(..)
                | Message::EntityModelSelected(..)
        )
    {
        app.entity_clone_cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }
    if invalidate_stream || matches!(&message, Message::ReloadScene) {
        app.scene_layers.invalidate();
    }
    if invalidate_stream {
        app.nbt_search.invalidate();
        app.object_inspector.invalidate();
        app.clear_block_preview();
        if matches!(
            &message,
            Message::Open(..) | Message::Select(_) | Message::Dimension(_) | Message::WorkspaceIdle
        ) {
            app.surface_request = None;
            app.map_refresh = Default::default();
            app.resource_reload_attempt = None;
        }
        if matches!(
            &message,
            Message::Open(..) | Message::Select(_) | Message::Dimension(_) | Message::WorkspaceIdle
        ) {
            transfer_edit::view_changed(app);
        } else {
            transfer_edit::cancel(app);
        }
        app.pending_import.cancel();
        plugin_ui::clear_structure(app);
        app.selected_block = None;
        plugin_ui::invalidate(&mut app.plugins);
        app.retire_spectator_sources();
    }
    let brush_parameters = matches!(
        &message,
        Message::TargetText(_) | Message::BlockEdit(block_edit::Action::BiomeTarget(_))
    ) || matches!(&message,Message::Brush(action) if !matches!(action,brush_stroke::Action::Finished(_)));
    let previous_key = app.current_key();
    // 区块检查借用源点选，但不改写用户的方块编辑范围或三维角点。
    let inspection_selection = (app.object_inspector.picking_chunks
        && matches!(
            &message,
            Message::SpectatorPick(..)
                | Message::PickSurfaceFace(..)
                | Message::PickSlice(..)
                | Message::PickVertical(..)
                | Message::Quad(quad_view::Action::Pick(..) | quad_view::Action::Scene(..))
        ))
    .then(|| (app.edit_area.clone(), app.corner_a, app.corner_b));
    let task = update_impl(app, message);
    if let Some((area, a, b)) = inspection_selection {
        app.edit_area = area;
        app.corner_a = a;
        app.corner_b = b;
    }

    if brush_parameters {
        brush_stroke::remember(app);
    }
    if previous_key != app.current_key() {
        app.surface_hover = None;
        app.world_hover.clear();
        app.object_inspector.picking_chunks = false;
        app.save_cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        app.save_planning = false;
    }
    if let Some((_, location)) = location_jump {
        match if player_source_view {
            app.player_view(&location)
        } else {
            app.locate(&location)
        } {
            Ok(()) => app.view_state.status = "已切换维度并定位到存档源位置".into(),
            Err(error) => {
                log::error!("[view-state] 跨维度定位失败：{error}");
                app.view_state.status = error;
            }
        }
    }
    app.remember_view();
    app.ensure_resource_source();
    selection_edit::changed_source(app, previous_key);
    if let Some((key, revision)) = source_revision {
        if app.current_key() == Some(key)
            && app.sessions.get(&key).is_some_and(|session| {
                revision.as_ref().map_or_else(
                    || session.has_pending_changes(),
                    |before| !std::sync::Arc::ptr_eq(before, &session.revision()),
                )
            })
        {
            app.save_cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            app.save_planning = false;
            // 界面历史同样更换事务身份，但没有改变原稿；不能撤销本轮刚开始的放置。
            let changed = source_content.as_ref().map_or_else(
                || app.sessions[&key].has_pending_changes(),
                |before| {
                    !matches!(
                        app.sessions[&key].display_source().changes_since(before),
                        mcedit2_world::edit_sources::SourceChanges::Unchanged
                    )
                },
            );
            if changed {
                app.invalidate_edit_previews();
            }
        }
    }
    plugin_ui::maintain_preview(app);
    let source_task = if previous_key == app.current_key() {
        use mcedit2_world::edit_sources::SourceChanges;
        let changes = match (display_source.as_ref(), app.display_source_revision()) {
            (Some(before), Some(after)) => after.changes_since(before),
            (Some(_), None) => SourceChanges::NewSource,
            (None, Some(_)) if source_was_displayed => SourceChanges::NewSource,
            _ => SourceChanges::Unchanged,
        };
        match changes {
            SourceChanges::Unchanged => iced::Task::none(),
            SourceChanges::Chunks(chunks) => app.refresh_scene_source(&chunks),
            SourceChanges::NewSource => {
                app.retire_spectator_sources();
                app.scene_layers.invalidate();
                app.refresh_scene_source(&Default::default())
            }
        }
    } else {
        iced::Task::none()
    };
    let flying =
        navigation_bindings::perspective_active(app) && has_flight_input(&app.spectator_keys);
    if !flying {
        app.spectator_last_frame = None;
    } else if !was_flying {
        app.spectator_last_frame = Some(std::time::Instant::now());
    }
    if was_world_mesh != app.uses_world_mesh() {
        mesh_async::cancel_spectator(&app.spectator_chunk_cache);
        // 流式完整世界与限定窗口使用不同的地形源，不能共用已提交帧。
        app.spectator_stream = None;
        app.spectator_stream_view = None;
        app.spectator_stream_snapshot = None;
        app.spectator_stream_near = None;
        app.spectator_fallback = None;
        app.spectator_cache = Default::default();
        app.spectator_pending = Default::default();
        app.spectator_presented = Default::default();
        app.spectator_presented_view = Default::default();
    }
    let world_visible = !app.show_top_down
        && !app.show_advanced
        && ((app.spectator && app.use_gpu) || app.uses_world_mesh());
    if was_world_visible && !world_visible {
        mesh_async::cancel_spectator(&app.spectator_chunk_cache);
    }
    let after = app.render_signature();
    if before.source != after.source || before.geometry != after.geometry {
        app.render_framing_cache = Default::default();
    }
    if before != after {
        if before.geometry != after.geometry
            || before.block_layers != after.block_layers
            || before.show_textures != after.show_textures
            || before.entity_frame != after.entity_frame
        {
            if before.geometry != after.geometry {
                app.quad.reset_framing();
            }
            app.quad.reset_projection();
        }
        if publishing_material && (before.source == after.source || publishing_source_material) {
            // The previous snapshot's UVs belong to its original atlas.
            // Start an entirely new immutable mesh generation for the new
            // resource pack while preserving the last completed frame.
            app.spectator_stream_near = None;
            app.spectator_stream_snapshot = None;
            if let Some(stream) = app.spectator_stream.as_mut() {
                stream.dirty = true;
            }
        }
        if must_reset_spectator_mesh(&before, &after, read_only_near_publish) {
            app.spectator_cache = std::sync::Arc::new(std::sync::Mutex::new(None));
            app.spectator_pending = std::sync::Arc::new(std::sync::Mutex::new(false));
            // For a resource-only change within the same source scene, keep
            // the last complete mesh AND its exact atlas until the worker
            // commits the replacement. World/dimension changes still reset.
            app.spectator_fallback = if publishing_material
                && (before.source == after.source || publishing_source_material)
                && !invalidate_stream
            {
                old_complete.or_else(|| app.spectator_fallback.take())
            } else {
                None
            };
        }
        // Isolate the new camera caches from workers holding old Arcs.
        mesh_async::cancel_isometric(&app.render_gpu_pending);
        app.render_face_cache = std::sync::Arc::new(std::sync::Mutex::new(None));
        app.render_canvas_cache = Default::default();
        app.render_gpu_cache = std::sync::Arc::new(std::sync::Mutex::new(None));
        app.render_gpu_pending = std::sync::Arc::new(std::sync::Mutex::new(None));
        app.grid_main.cancel();
        app.grid_horizontal.cancel();
        app.grid_vertical.cancel();
        app.grid_revision = app.grid_revision.wrapping_add(1);
        app.projected_ready = Default::default();
        app.projected_presented = Default::default();
    }
    let mut tasks = vec![
        task,
        source_task,
        block_edit::maintain(app),
        map_editor::maintain(app),
        quad_view::maintain_resources(app),
    ];
    if !app.near_busy {
        if let Some(job) = app.near_job.take() {
            app.near_busy = true;
            app.near_active_mode = Some(job.mode);
            if matches!(job.mode, NearMode::Surface | NearMode::Volume) {
                app.resource_reload_attempt = Some((
                    (job.key, job.mode, job.bounds, job.cut),
                    source_visibility::identity(
                        job.assets.as_ref(),
                        job.show_textures,
                        job.block_layers,
                    ),
                ));
            }
            tasks.push(dispatch_near(job));
        }
    }
    if !app.open_busy {
        if let Some((nonce, path)) = app.open_pending.take() {
            app.open_busy = true;
            tasks.push(dispatch_open(nonce, path));
        }
    }
    tasks.push(app.maintain_spectator_stream());
    tasks.push(app.maintain_scene_layers());
    let quad_active = app.show_advanced
        && app.advanced_section == AdvancedSection::Quad
        && app
            .quad
            .source_scope()
            .is_some_and(|(key, _)| Some(key) == app.current_key());
    let models_active = (!app.show_advanced && app.visible_scene_is_current())
        || quad_active
        || (app.show_advanced
            && app.advanced_section == AdvancedSection::Maps
            && app.scene_layers.has_plane_sources(app));
    let version = app
        .current_key()
        .and_then(|(wi, _)| app.worlds[wi].data_version);
    let (scene_assets, scene_textures) = if quad_active {
        app.quad.appearance().expect("已核对四视图的实际源范围")
    } else if app.show_advanced && app.advanced_section == AdvancedSection::Maps {
        (app.texture_bank.clone(), app.show_textures)
    } else {
        (app.scene_texture_bank().cloned(), app.scene_show_textures())
    };
    app.entity_models.language = app.language;
    app.entity_models.maps = app
        .current_key()
        .and_then(|(wi, _)| app.map_editor.source_view(&app.worlds[wi].root));
    app.entity_models.selected = app.scene_layers.selected.clone();
    app.entity_models.item_world = app.current_key().map(|(wi, di)| {
        item_world::Environment::new(&app.worlds[wi], &app.worlds[wi].dimensions[di].name)
    });
    tasks.push(app.entity_models.maintain_with_root(
        app.scene_layers.snapshot.clone(),
        scene_assets,
        version,
        scene_textures,
        models_active,
        app.current_key().map(|(wi, _)| app.worlds[wi].root.clone()),
    ));
    // One bounded read-only scan of the selected Minecraft instance.
    if !app.manual_textures
        && app.texture_bank.is_none()
        && app.structures.active.is_none()
        && !app.desktop.show_recent
    {
        if let Some(task) = resource_bindings::restore_world(app) {
            tasks.push(task);
        } else if let Some(index) = app.current {
            if app.surface.is_some() {
                if let Some(world) = app.worlds.get(index) {
                    if app
                        .auto_assets_attempted
                        .insert((index, world.root.clone()))
                    {
                        app.texture_loading = true;
                        app.texture_report = "正在扫描当前实例资源…".into();
                        tasks.push(auto_assets::load(
                            index,
                            world,
                            app.texture_load_nonce,
                            std::sync::Arc::clone(&app.texture_cancelled),
                            &app.texture_budget,
                            !app.texture_blocks_only,
                            app.plugins.resource_sources(),
                            app.block_definitions.applied.for_source(Some(&world.root)),
                            app.desktop.saved.client_installations.roots(),
                        ));
                    }
                }
            }
        }
    }
    selection_edit::remember(app);
    iced::Task::batch(tasks)
}
fn update_impl(app: &mut App, message: Message) -> iced::Task<Message> {
    if matches!(
        &message,
        Message::Open(..) | Message::Select(_) | Message::Dimension(_) | Message::WorkspaceIdle
    ) {
        app.map_editor.invalidate();
    }
    if matches!(
        &message,
        Message::Open(..) | Message::Select(_) | Message::WorkspaceIdle
    ) {
        app.player_browser.invalidate();
    }
    // A cached far-height patch reflects one original+overlay snapshot.
    // Any mutation must invalidate it before the next camera move.
    if matches!(
        &message,
        Message::Open(..)
            | Message::Select(_)
            | Message::Dimension(_)
            | Message::ConfirmWorldSave
            | Message::ConfirmBeSave
            | Message::FinalizeRegion
            | Message::RollbackRegion
            | Message::FinalizeRegionGroup
            | Message::RollbackRegionGroup
            | Message::WorkspaceIdle
    ) {
        app.far = None;
    }
    match message {
        Message::NativeProfile(event) => return native_profile::update(app, event),
        message @ (Message::DragDelivered(..)
        | Message::DragDrop(_)
        | Message::FloatingPanel(_)
        | Message::WindowInput(..)
        | Message::ToolWindow(..)
        | Message::AtlasInspector(_)
        | Message::About(_)
        | Message::WorldBrowser(_)) => return update_routed(app, message),
        Message::SingleInstance(event) => return single_instance::update(app, event),
        Message::BlockDefinitions(action) => return block_definitions_ui::update(app, action),
        Message::BlockPicker(action) => return block_picker::update(app, action),
        Message::SelectionDelete(action) => return deletion_ui::update(app, action),
        Message::ChunkOperations(action) => return chunk_operations_ui::update(app, action),
        Message::LogView(action) => return log_view::update(app, action),
        Message::BlockReplacement(action) => return block_replacement_ui::update(app, action),
        Message::History(action) => return history_ui::update(app, action),
        Message::Analysis(action) => return analysis_ui::update(app, action),
        Message::Library(action) => return library::update(app, action),
        Message::StructureWorkspace(id, action) => {
            return structure_workspace::update(app, id, action)
        }
        Message::FileInput(action) => return file_input::update(app, action),
        Message::CloseRequested(id) => {
            return workspace_tabs::update(
                app,
                workspace_tabs::Action::Close(workspace_tabs::CloseTarget::Window(id)),
            );
        }
        Message::Workspace(action) => return workspace_tabs::update(app, action),
        Message::TabBar(action) => return tab_bar::update(app, action),
        Message::MainMenu(action) => return workspace_ui::menu::update(app, action),
        Message::WorkspaceLayout(action) => return workspace_ui::layout::update(app, action),
        Message::WorkspacePresentation(action) => {
            return workspace_ui::presentation::update(app, action)
        }
        Message::KeyboardBindings(action) => return keyboard_bindings::update(app, action),
        Message::NavigationInput(action) => return navigation_bindings::update(app, action),
        Message::CameraDistance(action) => return camera_distance::update(app, action),
        Message::Minimap(action) => return minimap::update(app, action),
        Message::WorkPlane(action) => return work_plane::update(app, action),
        Message::CameraPerspective(perspective) => {
            desktop_state::camera_projection(app, perspective);
            return iced::Task::none();
        }
        Message::StickyInput(action) => return navigation_bindings::sticky::update(app, action),
        Message::WorkspaceKeyboard(event) => return workspace_tabs::keyboard(app, event),
        Message::CancelSavePlanning => {
            app.save_cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            app.save_planning = false;
            app.edit_report = "保存预检已取消，世界文件未修改".into();
        }
        Message::SavePrepared(key, revision, token, result) => {
            app.save_worker = false;
            app.save_planning = false;
            if app.current_key() != Some(key)
                || !std::sync::Arc::ptr_eq(&token, &app.save_cancelled)
                || token.load(std::sync::atomic::Ordering::Acquire)
                || app
                    .sessions
                    .get(&key)
                    .is_none_or(|s| !std::sync::Arc::ptr_eq(&revision, &s.revision()))
            {
                log::debug!("[save] 丢弃已取消或源修订过期的保存预检");
                return iced::Task::none();
            }
            app.save_planning = false;
            match result {
                Ok((intent, report)) => {
                    app.save_preview = Some(intent);
                    app.edit_report = report;
                }
                Err(error) => {
                    log::error!("[save] {error}");
                    app.edit_report = format!("保存预检被拒绝：{error}");
                }
            }
        }
        Message::SavePostprocess(action) => return save_postprocess::update(app, action),
        Message::WorldSave(action) => return world_save::update(app, action),
        Message::RegionRecovery(action) => return region_recovery::update(app, action),
        Message::ViewState(action) => return view_state::update(app, action),
        Message::NbtSearch(action) => return nbt_search::update(app, action),
        Message::Desktop(action) => return desktop_state::update(app, action),
        Message::MetadataReady(ready) => return metadata_refresh::finish(app, ready),
        Message::NbtPath(path) => {
            app.nbt_path = path;
            app.nbt_prefill = None;
        }
        Message::NbtEditable(editable) => app.nbt_editable = editable,
        Message::NbtOpen => return nbt_documents::open(app),
        Message::NbtOpenScoped(request) => return nbt_documents::open_scoped(app, request),
        Message::NbtOpened(id, key, result) => return nbt_documents::opened(app, id, key, result),
        Message::Nbt(id, action) => return nbt_documents::edit(app, id, action),
        Message::NbtWorkspace(action) => return nbt_documents::update(app, action),
        Message::SelectionClipboard(action) => return selection_clipboard::update(app, action),
        Message::SelectionEdit(action) => return selection_edit::update(app, action),
        Message::SelectionAll(action) => return selection_edit::update_extent(app, action),
        Message::Placement(action) => return placement::update(app, action),
        Message::Flood(action) => return flood_fill::update(app, action),
        Message::MapEditor(action) => return map_editor::update(app, action),
        Message::PlayerBrowser(action) => return player_browser::update(app, action),
        Message::Transfer(action) => return transfer_edit::update(app, action),
        Message::Exchange(action) => return structure_exchange::update(app, action),
        Message::BlockEdit(action) => return block_edit::update(app, action),
        Message::MapRefresh(ready) => map_refresh::update(app, ready),
        Message::Brush(action) => return brush_stroke::update(app, action),
        Message::StructureCreation(action) => {
            transfer_edit::cancel(app);
            structure_exchange::revoke(app, true);
            app.structure_creation.update(action);
        }
        Message::WorldInfo => {
            app.nbt_path = if app
                .current_key()
                .is_some_and(|(wi, _)| app.worlds[wi].root.join("level.dat").is_file())
            {
                "level.dat"
            } else {
                "level.dat_old"
            }
            .into();
            app.nbt_editable = !app.readonly;
            app.nbt_prefill = Some(nbt_documents::Prefill::new(
                None,
                Vec::new(),
                Some("/Data".into()),
            ));
            app.show_advanced = true;
            app.advanced_section = AdvancedSection::WorldInfo;
            let path = std::path::PathBuf::from(&app.nbt_path);
            return nbt_documents::queue_open(app, path);
        }
        action @ (Message::NbtGroupPrepare
        | Message::NbtGroupFinish
        | Message::NbtGroupRollback) => {
            if app.nbt_group_busy
                || app.mixed_busy
                || app.save_worker
                || app.save_planning
                || app.mixed_key.is_some()
            {
                return iced::Task::none();
            }
            let Some(key) = app.current_key() else {
                app.nbt_group_status = "请先选择世界".into();
                return iced::Task::none();
            };
            if app.nbt_group_key.is_some_and(|old| old.0 != key.0) {
                app.nbt_group_status = "保存组属于另一个世界".into();
                return iced::Task::none();
            }
            let kind = match action {
                Message::NbtGroupPrepare => 0,
                Message::NbtGroupFinish => 1,
                _ => 2,
            };
            let root = app.worlds[key.0].root.clone();
            let candidates = app.nbt_group.clone();
            if kind == 0 {
                if let Err(error) = metadata_refresh::verify(app, &candidates) {
                    app.nbt_group_status = error;
                    return iced::Task::none();
                }
            }
            let read_only = app.readonly;
            app.nbt_group_busy = true;
            app.nbt_group_key = Some(key);
            return iced::Task::perform(
                nbt_background(move || {
                    let saved = if kind == 1 {
                        mcedit2_world::nbt_file::group::identities(&root)
                            .map_err(|e| e.to_string())?
                    } else {
                        Vec::new()
                    };
                    let count = (match kind {
                        0 => mcedit2_world::nbt_file::group::prepare(&root, &candidates, read_only),
                        1 => mcedit2_world::nbt_file::group::finish(&root, read_only),
                        _ => mcedit2_world::nbt_file::group::rollback(&root, read_only),
                    })
                    .map_err(|e| e.to_string())?;
                    Ok((count, saved))
                }),
                move |result| Message::NbtGroupDone(key, kind, result),
            );
        }
        Message::NbtGroupDone(key, kind, result) => {
            app.nbt_group_busy = false;
            if app.nbt_group_key != Some(key) {
                return iced::Task::none();
            }
            match result {
                Ok((count, saved)) => {
                    app.nbt_group_status = match kind {
                        0 => format!("已准备 {count} 个源文件；尚未替换，需明确确认提交或回滚"),
                        1 => format!("已提交 {count} 个源文件"),
                        _ => format!("已回滚 {count} 个源文件"),
                    };
                    if kind != 0 {
                        app.nbt_group.clear();
                        app.nbt_group_key = None;
                        let root = app.worlds[key.0].root.clone();
                        return metadata_refresh::start_saved(app, root, saved);
                    }
                }
                Err(error) => {
                    log::error!("[nbt-group] {error}");
                    app.nbt_group_status = format!("{error}；若事务已准备，请选择恢复方向");
                }
            }
        }
        Message::MixedPhrase(value) => app.mixed_phrase = value,
        action @ (Message::MixedPrepare | Message::MixedFinish | Message::MixedRollback) => {
            let kind = match action {
                Message::MixedPrepare => 0,
                Message::MixedFinish => 1,
                _ => 2,
            };
            return mixed_save::run(app, kind);
        }
        Message::MixedDone(key, kind, result) => return mixed_save::finish(app, key, kind, result),
        Message::MixedReviewed(ready) => return mixed_save::reviewed(app, ready),
        Message::ObjectInspector(action) => return object_inspector::update(app, action),
        Message::BeTreeOperation(value) => {
            app.be_tree_operation = value;
            app.be_tree_preview = None;
        }
        Message::BeTreePrepare => {
            app.be_tree_preview = None;
            if app.readonly {
                app.be_tree_status = "只读模式禁止方块实体编辑".into();
                return iced::Task::none();
            }
            let Some(key) = app.current_key() else {
                return iced::Task::none();
            };
            let Some(selected) =
                app.scene_layers.selected.as_ref().filter(|v| {
                    v.marker().kind == mcedit2_world::scene_annotations::Kind::BlockEntity
                })
            else {
                app.be_tree_status = "请选择方块实体".into();
                return iced::Task::none();
            };
            if app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.key != key || !snapshot.markers.iter().any(|m| m.same(selected))
            }) {
                app.be_tree_status = "方块实体快照已过期".into();
                return iced::Task::none();
            }
            let p = selected.marker().position;
            let source_sha = selected.marker().sha256.clone();
            if p.iter().any(|v| {
                !v.is_finite() || *v < i32::MIN as f64 || *v > i32::MAX as f64 || v.fract() != 0.
            }) {
                app.be_tree_status = "方块实体位置无效".into();
                return iced::Task::none();
            }
            let op = serde_json::from_str::<mcedit2_world::nbt_document::Operation>(
                &app.be_tree_operation,
            )
            .map_err(|e| e.to_string());
            let result = op.and_then(|op| {
                app.current_session(key).and_then(|session| {
                    let pos = (p[0] as i32, p[1] as i32, p[2] as i32);
                    let original = session
                        .inspect_original_block_entity(pos)
                        .map_err(|e| e.to_string())?
                        .ok_or("方块实体原始源不存在")?;
                    if original.original_compound_sha256 != source_sha {
                        return Err("方块实体原始源已改变，请重新读取".into());
                    }
                    session
                        .prepare_be_nbt_edit(pos, op)
                        .map_err(|e| e.to_string())
                })
            });
            match result {
                Ok(preview) => {
                    app.be_tree_preview = Some(preview);
                    app.be_tree_status = "方块实体 NBT 已预检，确认后暂存；保存仍须混合事务".into();
                }
                Err(e) => {
                    log::error!("[be-nbt] {e}");
                    app.be_tree_status = e;
                }
            }
        }
        Message::BeTreeApply => {
            if app.readonly {
                app.be_tree_status = "只读模式禁止方块实体编辑".into();
                return iced::Task::none();
            }
            let Some(key) = app.current_key() else {
                return iced::Task::none();
            };
            let Some(preview) = app.be_tree_preview.take() else {
                return iced::Task::none();
            };
            match app.current_session(key).and_then(|session| {
                session
                    .apply_be_nbt_preview(preview)
                    .map_err(|e| e.to_string())
            }) {
                Ok(()) => {
                    app.be_tree_status = "方块实体修改已暂存，可撤销".into();
                    app.scene_layers.invalidate();
                    app.save_preview = None;
                }
                Err(e) => {
                    log::error!("[be-nbt] {e}");
                    app.be_tree_status = e;
                }
            }
        }
        Message::EntityDelta(value) => {
            app.entity_clone_cancel
                .store(true, std::sync::atomic::Ordering::Release);
            app.entity_edit_delta = value;
            app.entity_edit_preview = None;
        }
        action @ (Message::EntityPrepareDelete | Message::EntityPrepareTranslate) => {
            app.entity_clone_cancel
                .store(true, std::sync::atomic::Ordering::Release);
            app.entity_edit_preview = None;
            if app.readonly {
                app.entity_edit_status = "只读模式禁止实体修改".into();
                return iced::Task::none();
            }
            let Some(key) = app.current_key() else {
                return iced::Task::none();
            };
            let Some(selected) = app.scene_layers.selected.as_ref().filter(|v| {
                matches!(
                    v.marker().kind,
                    mcedit2_world::scene_annotations::Kind::Entity
                        | mcedit2_world::scene_annotations::Kind::Item
                )
            }) else {
                app.entity_edit_status = "请选择实体".into();
                return iced::Task::none();
            };
            if app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.key != key || !snapshot.markers.iter().any(|m| m.same(selected))
            }) {
                app.entity_edit_status = "实体快照已过期".into();
                return iced::Task::none();
            }
            let chunk = selected.marker().chunk;
            let path = selected.marker().path.clone();
            let position = selected.marker().position;
            let selected_sha = selected.marker().sha256.clone();
            let delta = parse_xyz(&app.entity_edit_delta).map_err(|e| e.to_string());
            let result = match app.current_session(key) {
                Ok(session) => {
                    let source = session
                        .inspect_entities(chunk.0, chunk.1)
                        .map_err(|error| error.to_string())
                        .and_then(|catalog| {
                            catalog
                                .entities
                                .into_iter()
                                .find(|entity| entity.path == path && entity.sha256 == selected_sha)
                                .ok_or_else(|| "实体源或内容已变化，请重新加载".to_string())
                        });
                    if let Err(error) = source {
                        Err(error)
                    } else if matches!(action, Message::EntityPrepareDelete) {
                        session
                            .prepare_entity_delete(chunk, &path)
                            .map_err(|e| e.to_string())
                    } else {
                        delta.and_then(|(x, y, z)| {
                            let tx = ((position[0] + f64::from(x)) / 16.0).floor();
                            let tz = ((position[2] + f64::from(z)) / 16.0).floor();
                            if tx < i32::MIN as f64
                                || tx > i32::MAX as f64
                                || tz < i32::MIN as f64
                                || tz > i32::MAX as f64
                            {
                                return Err("目标区块超出坐标范围".into());
                            }
                            let target = (tx as i32, tz as i32);
                            if target == chunk {
                                session
                                    .prepare_entity_translation(chunk, &path, [x, y, z])
                                    .map_err(|e| e.to_string())
                            } else {
                                let index = path
                                    .trim_start_matches('/')
                                    .strip_prefix("Entities/")
                                    .and_then(|v| v.parse::<usize>().ok())
                                    .ok_or("跨区块移动仅支持实体列表顶层项")?;
                                session
                                    .prepare_entity_move(chunk, target, index, [x, y, z])
                                    .map_err(|e| e.to_string())
                            }
                        })
                    }
                }
                Err(e) => Err(e),
            };
            match result {
                Ok(preview) => {
                    app.entity_edit_preview = Some(preview);
                    app.entity_edit_status =
                        format!("已预检实体 {path}，确认后只暂存，保存仍需混合事务");
                }
                Err(e) => {
                    log::error!("[entity-edit] {e}");
                    app.entity_edit_status = e;
                }
            }
        }
        Message::EntityPrepareClone => {
            app.entity_clone_cancel
                .store(true, std::sync::atomic::Ordering::Release);
            app.entity_clone_cancel =
                std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            app.entity_edit_preview = None;
            if app.readonly {
                app.entity_edit_status = "只读模式禁止实体克隆".into();
                return iced::Task::none();
            }
            let Some(key) = app.current_key() else {
                return iced::Task::none();
            };
            let Some(selected) = app.scene_layers.selected.clone().filter(|v| {
                matches!(
                    v.marker().kind,
                    mcedit2_world::scene_annotations::Kind::Entity
                        | mcedit2_world::scene_annotations::Kind::Item
                )
            }) else {
                app.entity_edit_status = "请选择实体".into();
                return iced::Task::none();
            };
            if app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.key != key || !snapshot.markers.iter().any(|m| m.same(&selected))
            }) {
                app.entity_edit_status = "实体快照已过期".into();
                return iced::Task::none();
            }
            let delta = match parse_xyz(&app.entity_edit_delta) {
                Ok((x, y, z)) => [x, y, z],
                Err(e) => {
                    app.entity_edit_status = e.to_string();
                    return iced::Task::none();
                }
            };
            let marker = selected.marker();
            let from = marker.chunk;
            let position = marker.position;
            let Some(index) = marker
                .path
                .trim_start_matches('/')
                .strip_prefix("Entities/")
                .and_then(|v| v.parse::<usize>().ok())
            else {
                app.entity_edit_status = "克隆仅支持实体列表顶层项".into();
                return iced::Task::none();
            };
            let x = ((position[0] + f64::from(delta[0])) / 16.).floor();
            let z = ((position[2] + f64::from(delta[2])) / 16.).floor();
            if x < i32::MIN as f64
                || x > i32::MAX as f64
                || z < i32::MIN as f64
                || z > i32::MAX as f64
            {
                app.entity_edit_status = "目标区块超出范围".into();
                return iced::Task::none();
            }
            let to = (x as i32, z as i32);
            let sha = marker.sha256.clone();
            let path = marker.path.clone();
            let budget = match app.block_edit.budget.parse() {
                Ok(budget) => budget.bytes(),
                Err(error) => {
                    log::error!("[entity-clone] {error}");
                    app.entity_edit_status = error;
                    return iced::Task::none();
                }
            };
            let session = match app.current_session(key) {
                Ok(s) => s.clone(),
                Err(e) => {
                    app.entity_edit_status = e;
                    return iced::Task::none();
                }
            };
            let cancel = std::sync::Arc::clone(&app.entity_clone_cancel);
            let check_token = std::sync::Arc::clone(&cancel);
            app.entity_edit_status = "正在后台扫描全世界实体 UUID 并预检克隆…".into();
            return iced::Task::perform(
                nbt_background(move || {
                    let cancelled = || cancel.load(std::sync::atomic::Ordering::Acquire);
                    let catalog = session
                        .inspect_entities(from.0, from.1)
                        .map_err(|e| e.to_string())?;
                    if !catalog
                        .entities
                        .iter()
                        .any(|v| v.path == path && v.sha256 == sha)
                    {
                        return Err("实体源已变化，请重新读取".into());
                    }
                    let identities = session
                        .scan_entity_identities(budget, cancelled)
                        .map_err(|e| e.to_string())?;
                    session
                        .prepare_entity_clone(
                            from,
                            to,
                            index,
                            delta,
                            &identities,
                            budget,
                            cancelled,
                        )
                        .map_err(|e| e.to_string())
                }),
                move |result| Message::EntityClonePrepared(key, selected, check_token, result),
            );
        }
        Message::EntityClonePrepared(key, selected, token, result) => {
            if !std::sync::Arc::ptr_eq(&token, &app.entity_clone_cancel)
                || app
                    .entity_clone_cancel
                    .load(std::sync::atomic::Ordering::Acquire)
                || app.current_key() != Some(key)
                || app
                    .scene_layers
                    .selected
                    .as_ref()
                    .is_none_or(|m| !m.same(&selected))
                || app
                    .scene_layers
                    .snapshot
                    .as_ref()
                    .is_none_or(|s| !s.markers.iter().any(|m| m.same(&selected)))
            {
                return iced::Task::none();
            }
            match result {
                Ok(preview) => {
                    app.entity_edit_status = format!(
                        "克隆已预检，可确认暂存。{}",
                        preview.diagnostics().join("；")
                    );
                    app.entity_edit_preview = Some(preview);
                }
                Err(e) => {
                    log::error!("[entity-clone] {e}");
                    app.entity_edit_status = e;
                }
            }
        }
        Message::EntityApplyPreview => {
            if app.readonly {
                app.entity_edit_status = "只读模式禁止实体修改".into();
                return iced::Task::none();
            }
            let Some(key) = app.current_key() else {
                return iced::Task::none();
            };
            let Some(preview) = app.entity_edit_preview.take() else {
                return iced::Task::none();
            };
            match app.current_session(key).and_then(|session| {
                session
                    .apply_entity_preview(preview)
                    .map_err(|e| e.to_string())
            }) {
                Ok(()) => {
                    app.entity_edit_status = "实体修改已暂存，可撤销；磁盘尚未修改".into();
                    app.scene_layers.invalidate();
                    app.save_preview = None;
                }
                Err(e) => {
                    log::error!("[entity-edit] {e}");
                    app.entity_edit_status = e;
                }
            }
        }
        Message::PlayerJump => {
            if let Some(selected) = app
                .scene_layers
                .selected
                .as_ref()
                .filter(|v| v.marker().kind == mcedit2_world::scene_annotations::Kind::Player)
            {
                if app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
                    Some(snapshot.key) != app.current_key()
                        || !snapshot.markers.iter().any(|m| m.same(selected))
                }) {
                    log::error!("[players] 拒绝过期的玩家位置");
                    return iced::Task::none();
                }
                let location = mcedit2_world::player_catalog::Location {
                    dimension: app
                        .current_key()
                        .map(|(wi, di)| app.worlds[wi].dimensions[di].name.clone())
                        .expect("已核对当前玩家来源"),
                    position: selected.marker().position,
                    rotation: selected.marker().visual.as_ref().and_then(|v| v.rotation),
                };
                return iced::Task::done(Message::ViewState(view_state::Action::PlayerSource(
                    app.current.expect("玩家世界来源已核实"),
                    location,
                )));
            }
        }
        kind @ (Message::PlayerInventory
        | Message::PlayerAttributes
        | Message::PlayerMoveToCamera
        | Message::PlayerRotateToCamera) => {
            let Some(selected) = app
                .scene_layers
                .selected
                .as_ref()
                .filter(|v| v.marker().kind == mcedit2_world::scene_annotations::Kind::Player)
            else {
                return iced::Task::none();
            };
            if app.scene_layers.snapshot.as_ref().is_none_or(|snapshot| {
                Some(snapshot.key) != app.current_key()
                    || !snapshot.markers.iter().any(|m| m.same(selected))
            }) {
                log::error!("[players] 拒绝过期的玩家源");
                return iced::Task::none();
            }
            let marker = selected.marker();
            let prefix = if marker.path == "/" {
                String::new()
            } else {
                format!("/{}", marker.path.trim_matches('/'))
            };
            let moving = matches!(
                kind,
                Message::PlayerMoveToCamera | Message::PlayerRotateToCamera
            );
            let rotating = matches!(kind, Message::PlayerRotateToCamera);
            if moving && app.readonly {
                app.nbt_status = "只读模式禁止暂存玩家位置".into();
                return iced::Task::none();
            }
            let focus = if rotating {
                format!("{prefix}/Rotation")
            } else if moving {
                format!("{prefix}/Pos")
            } else if matches!(kind, Message::PlayerAttributes) {
                prefix.clone()
            } else {
                format!("{prefix}/Inventory")
            };
            let prefill = if moving {
                let pose = match player_editor::capture_camera(app, !rotating) {
                    Ok(pose) => pose,
                    Err(error) => {
                        log::error!("[players] {error}");
                        app.nbt_status = error;
                        return iced::Task::none();
                    }
                };
                nbt_documents::Prefill::camera(marker.sha256.clone(), prefix, pose, focus)
            } else {
                nbt_documents::Prefill::new(Some(marker.sha256.clone()), Vec::new(), Some(focus))
            };
            app.nbt_path = marker.source.clone();
            app.nbt_editable = !app.readonly;
            app.nbt_prefill = Some(prefill);
            app.show_advanced = true;
            app.advanced_section = AdvancedSection::Players;
            let path = std::path::PathBuf::from(&app.nbt_path);
            return nbt_documents::queue_open(app, path);
        }
        Message::Quad(action) => return quad_view::update(app, action),
        Message::Plugin(action) => return plugin_ui::update(app, action),
        Message::StartupScript(action) => return startup_script_ui::update(app, action),
        Message::Path(x) => app.path = x,
        Message::TexturePath(x) => app.texture_path = x,
        Message::TextureBudget(x) => app.texture_budget = x,
        Message::TextureAllPng(x) => app.texture_blocks_only = !x,
        Message::SpectatorSampled(x) => {
            app.spectator_sampled = x;
            app.spectator_stream = None;
        }
        Message::TextureDiagnostics => app.texture_diagnostics = !app.texture_diagnostics,
        Message::AdvancedSection(tab) => {
            if matches!(tab, AdvancedSection::Players | AdvancedSection::WorldInfo) {
                return metadata_pages::show(app, tab);
            }
            app.advanced_section = tab;
            app.show_advanced = true;
        }
        Message::WorkspacePane(pane) => {
            keyboard_bindings::interrupt(app);
            app.workspace_pane = pane;
            let kind = workspace_ui::layout::active(app);
            app.workspace.layout.show(kind);
            let task = if pane == workspace_ui::Pane::Library {
                iced::Task::batch([
                    workspace_ui::scroll_start(workspace_ui::OPTIONS_SCROLL),
                    library::update(app, library::Action::Show),
                ])
            } else if pane == workspace_ui::Pane::History {
                iced::Task::batch([
                    workspace_ui::scroll_start(workspace_ui::OPTIONS_SCROLL),
                    history_ui::update(app, history_ui::Action::Show),
                ])
            } else if pane == workspace_ui::Pane::Analysis {
                analysis_ui::update(app, analysis_ui::Action::Show)
            } else {
                workspace_ui::scroll_start(workspace_ui::options_scroll(app))
            };
            return iced::Task::batch([task, tab_bar::reveal(app, tab_bar::Id::Panel(pane))]);
        }
        Message::EditorTool(tool) => {
            app.editor_tool = tool;
            let kind = workspace_ui::layout::active(app);
            app.workspace.layout.show(kind);
            return workspace_ui::scroll_start(workspace_ui::OPTIONS_SCROLL);
        }
        Message::WorkspaceView(view) => {
            use workspace_ui::View;
            match view {
                View::Slices | View::Quad => {
                    app.advanced_section = if view == View::Slices {
                        AdvancedSection::Maps
                    } else {
                        AdvancedSection::Quad
                    };
                    app.show_advanced = true;
                }
                View::Perspective | View::Isometric | View::Overhead => {
                    app.show_advanced = false;
                    app.spectator_keys.clear();
                    if view == View::Overhead {
                        if !app.show_top_down {
                            return update_impl(app, Message::ToggleTopDown);
                        }
                    } else {
                        let was_top = app.show_top_down;
                        app.show_top_down = false;
                        if app.spectator != (view == View::Perspective) {
                            return update_impl(app, Message::ToggleSpectator);
                        }
                        if was_top && app.selected_window().is_none() {
                            app.inspect_surface(true);
                        }
                    }
                }
            }
        }
        Message::ChooseAdvancedFile(kind, save) => {
            return dialogs::advanced_file(app, kind, save);
        }
        Message::NativeDialog(action) => return dialogs::update(app, action),
        Message::ToggleTextures => app.show_textures = !app.show_textures,
        Message::RefreshInstanceAssets => {
            let Some(index) = app.current else {
                app.texture_report =
                    "Select a Java world before scanning instance resources.".into();
                return iced::Task::none();
            };
            let Some(world) = app.worlds.get(index).cloned() else {
                return iced::Task::none();
            };
            // Preserve the old completed atlas until the new one is ready.
            resource_bindings::automatic(app, &world.root);
            app.manual_textures = false;
            app.texture_loading = true;
            app.restart_asset_request();
            app.auto_assets_attempted
                .insert((index, world.root.clone()));
            app.texture_report =
                "Scanning this world's instance for vanilla, mod and enabled pack assets…".into();
            return auto_assets::load(
                index,
                &world,
                app.texture_load_nonce,
                std::sync::Arc::clone(&app.texture_cancelled),
                &app.texture_budget,
                !app.texture_blocks_only,
                app.plugins.resource_sources(),
                app.block_definitions.applied.for_source(Some(&world.root)),
                app.desktop.saved.client_installations.roots(),
            );
        }
        Message::ToggleGpu => {
            // Never silently substitute a top-down/orthographic fallback
            // for the perspective viewer when the GPU is disabled.
            if !app.spectator && !app.uses_world_mesh() {
                app.use_gpu = !app.use_gpu;
            }
        }
        Message::ToggleTopDown => {
            app.show_top_down = !app.show_top_down;
            app.spectator_keys.clear();
            // A 2D map needs a full-height surface scan. Restore exact
            // voxel streaming when returning to first-person spectator.
            if app.selected_window().is_none() {
                app.inspect_surface(!app.show_top_down);
            }
        }
        Message::ToggleSpectator => {
            app.spectator = !app.spectator;
            app.use_gpu = true;
            if app.spectator {
                app.show_top_down = false;
            }
            if let Some(window) = app.selected_window() {
                if app.spectator {
                    app.ensure_window_camera(window);
                }
            } else if !app.show_top_down && !app.surface_exposed {
                app.inspect_surface(true);
            }
            app.spectator_keys.clear();
        }
        Message::SpectatorLook(dx, dy) => {
            if navigation_bindings::perspective_active(app) {
                app.spectator_camera.look(dx, dy);
            }
        }
        Message::SpectatorKey(event) => {
            return keyboard_bindings::movement(app, event);
        }
        Message::SpectatorFocusLost => {
            app.shortcuts.navigation.sticky.discard();
            app.shortcuts.navigation.modifiers = iced::keyboard::Modifiers::default();
            drag_drop::focus_lost(app);
        }
        Message::SpectatorRadius(value) => {
            return camera_distance::update(app, camera_distance::Action::Radius(value));
        }
        Message::SpectatorStreamBatch(nonce, batch_id, results) => {
            if let Some(stream) = app.spectator_stream.as_mut() {
                if let Some(error) = stream.receive_delivery(nonce, batch_id, results) {
                    app.surface_report = error;
                }
            }
        }
        Message::SpectatorStreamPlan(nonce, plan_id, result) => {
            if let Some(stream) = app.spectator_stream.as_mut() {
                return stream.receive_plan(nonce, plan_id, result);
            }
        }
        Message::SpectatorTick(at) => {
            if navigation_bindings::perspective_active(app) && has_flight_input(&app.spectator_keys)
            {
                if let Some(previous) = app.spectator_last_frame {
                    if let Some(elapsed) = at.checked_duration_since(previous) {
                        app.spectator_last_frame = Some(at);
                        app.spectator_camera
                            .fly(&app.spectator_keys, elapsed.as_secs_f32());
                        app.recenter_spectator();
                    }
                }
            }
        }
        // GPU 世代通知只唤醒一次主更新，继续合并的区块批次并刷新实际显示数量。
        Message::WorldMeshPresented => {}
        Message::RuntimeRatesRefresh => {}
        Message::SetSceneLayer(kind, enabled) => {
            scene_layers::preferences::set_layer(app, kind, enabled);
        }
        Message::SetCommandLayer(layer, enabled) => {
            scene_layers::preferences::set_command_layer(app, layer, enabled);
        }
        Message::SceneHeightmap(heightmap) => {
            scene_layers::preferences::set_heightmap(app, heightmap);
        }
        Message::InspectSceneLayers(enabled) => app.scene_layers.inspect = enabled,
        Message::SceneLayerDiagnostics => {
            app.scene_layers.show_diagnostics = !app.scene_layers.show_diagnostics
        }
        Message::SceneLayerControls => {
            app.scene_layers.show_controls = !app.scene_layers.show_controls
        }
        Message::ReloadSceneLayers => {
            app.entity_models.reload_appearances();
            app.scene_layers.invalidate();
        }
        Message::SceneLayersLoaded(epoch, key, result) => {
            app.scene_layers.accept(epoch, key, result)
        }
        Message::SceneMarkerSelected(snapshot, index) => {
            if app.reviewing_block_preview() {
                return iced::Task::none();
            }
            let key = app.current_key();
            if !app.scene_layers.select(&snapshot, index, key) {
                log::error!("[layers] 拒绝过期或已隐藏的位置标记选择");
            } else if let (Some((wi, di)), Some(selected)) =
                (key, app.scene_layers.selected.as_ref())
            {
                let marker = selected.marker();
                if app.object_inspector.picking_chunks {
                    let chunk = marker.chunk;
                    let _ = object_inspector::update(
                        app,
                        object_inspector::Action::ChunkCoordinates(format!(
                            "{},{}",
                            chunk.0, chunk.1
                        )),
                    );
                    return object_inspector::update(app, object_inspector::Action::OpenChunk);
                }
                if let Some(mcedit2_world::nbt_document::Value::String(command)) = marker
                    .visual
                    .as_ref()
                    .and_then(|v| v.appearance.get("Command"))
                {
                    let point = marker.position.map(|v| v as i32);
                    match mcedit2_world::command_visuals::analyze(
                        command,
                        point,
                        &app.worlds[wi].dimensions[di].name,
                        app.worlds[wi].data_version,
                    ) {
                        Ok(visuals) => {
                            app.scene_layers.command_visuals =
                                Some((marker.sha256.clone(), std::sync::Arc::new(visuals)))
                        }
                        Err(error) => {
                            log::error!("[commands] {error}");
                            app.scene_layers.status = format!("命令范围不可显示：{error}");
                        }
                    }
                }
            }
        }
        Message::ShowEntityModels(enabled) => {
            entity_visibility::set_enabled(app, enabled);
        }
        Message::EntityModelVisibility(layer, enabled) => {
            return entity_visibility::update(app, layer, enabled)
        }
        Message::ShowOnlinePlayerSkins(enabled) => {
            app.entity_models.online_skins = enabled;
            if app.pending_import.clipboard.is_some() {
                return crate::pending_import::reload(app);
            }
        }
        Message::PlayerSkinLoaded(epoch, uuid, result) => {
            app.entity_models.accept_skin(epoch, uuid, result)
        }
        Message::HeadSkinLoaded(epoch, key, result) => {
            app.entity_models.accept_head_skin(epoch, key, result)
        }
        Message::ProjectedEntitySelected(rendered, marker) => {
            if app.reviewing_block_preview() {
                return iced::Task::none();
            }
            let main_presented = app
                .projected_presented
                .lock()
                .expect("编辑器场景状态锁被毒化")
                .as_ref()
                .is_some_and(|shown| std::sync::Arc::ptr_eq(shown, &rendered));
            let valid =
                (main_presented
                    || app.grid_main.is_presented(&rendered)
                    || app.grid_horizontal.is_presented(&rendered)
                    || app.grid_vertical.is_presented(&rendered)
                    || app.quad.has_presented(&rendered))
                    && entity_visibility::can_display(app, marker.marker())
                    && rendered.projected.as_ref().is_some_and(|projection| {
                        projection
                            .frame
                            .displays(app.entity_models.enabled, app.entity_models.visibility)
                            && Some(projection.frame.snapshot.key) == app.current_key()
                            && app.scene_layers.snapshot.as_ref().is_some_and(|s| {
                                std::sync::Arc::ptr_eq(s, &projection.frame.snapshot)
                            })
                            && projection
                                .frame
                                .snapshot
                                .markers
                                .iter()
                                .any(|m| m.same(&marker))
                    });
            if valid {
                app.scene_layers.selected = Some(marker);
            } else {
                log::error!("[entities] 拒绝非当前已显示投影帧的实体选择");
            }
        }
        Message::EntityModelsLoaded(epoch, result) => {
            let previous = app.entity_models.frame.clone();
            app.entity_models.accept(epoch, result);
            let replaced = app.entity_models.frame.as_ref().is_some_and(|frame| {
                previous
                    .as_ref()
                    .is_none_or(|old| !std::sync::Arc::ptr_eq(old, frame))
            });
            if replaced
                && app.pending_import.clipboard.is_some()
                && app.pending_import.frame.as_ref().is_some_and(|preview| {
                    app.entity_models.frame.as_ref().is_some_and(|frame| {
                        Some(frame.snapshot.key) == app.current_key()
                            && app
                                .scene_texture_bank()
                                .and_then(|bank| bank.atlas())
                                .is_some_and(|base| textures::atlas_compatible(&frame.atlas, base))
                            && (preview.show_textures != frame.show_textures
                                || !preview.compatible_with(Some(&frame.atlas)))
                    })
                })
            {
                return crate::pending_import::reload(app);
            }
        }
        Message::EntityModelSelected(mesh, marker) => {
            if app.reviewing_block_preview() {
                return iced::Task::none();
            }
            let valid = entity_visibility::can_display(app, marker.marker())
                && app
                    .spectator_presented
                    .lock()
                    .expect("编辑器场景状态锁被毒化")
                    .as_ref()
                    .is_some_and(|shown| std::sync::Arc::ptr_eq(shown, &mesh))
                && mesh.entities.as_ref().is_some_and(|frame| {
                    frame.displays(app.entity_models.enabled, app.entity_models.visibility)
                        && app.current_key() == Some(frame.snapshot.key)
                        && app
                            .scene_layers
                            .snapshot
                            .as_ref()
                            .is_some_and(|s| std::sync::Arc::ptr_eq(s, &frame.snapshot))
                        && frame.snapshot.markers.iter().any(|m| m.same(&marker))
                });
            if valid {
                app.scene_layers.selected = Some(marker);
            } else {
                log::error!("[entities] 拒绝过期或已隐藏的实体模型选择");
            }
        }
        Message::CycleGpuQuality => app.gpu_quality = app.gpu_quality.next(),
        Message::UnloadTextures => {
            return resource_bindings::unload(app);
        }
        Message::LoadTextures => {
            return resource_bindings::load_manual(app);
        }
        Message::CancelTextureLoad => {
            let warning = std::mem::take(&mut app.texture_warning);
            app.restart_asset_request();
            app.texture_warning = warning;
            app.texture_loading = false;
            app.texture_report = "已取消后续材质加载。".into();
        }
        Message::TextureProgress(nonce, progress) => {
            if nonce == app.texture_load_nonce && app.texture_loading {
                app.texture_report = progress;
            }
        }
        Message::TextureWarning(nonce, warning) => {
            if nonce == app.texture_load_nonce {
                app.texture_warning = warning;
            }
        }
        Message::TextureBaseLoaded(nonce, world, bank) => {
            let same_world = world.as_ref().is_none_or(|(index, root)| {
                app.current == Some(*index)
                    && app.worlds.get(*index).is_some_and(|w| &w.root == root)
            });
            if nonce == app.texture_load_nonce && same_world {
                app.texture_bank = Some(std::sync::Arc::new(entity_visibility::bank_for_display(
                    app, bank,
                )));
                app.show_textures = true;
                app.texture_report =
                    "基础材质已就绪，正在更新显示源；继续加载实例的模组与资源包…".into();
            }
        }
        Message::TextureLoadFinished(nonce, path, result) => {
            if nonce != app.texture_load_nonce {
                return iced::Task::none();
            }
            app.texture_loading = false;
            match result {
                Ok((bank, detected)) => {
                    let sources =
                        detected
                            .as_ref()
                            .map(|local| {
                                format!(
                        " (instance: {} framework JARs / {} mod JARs / {} enabled packs / {} generated, {})",
                        local.framework_count, local.mod_count, local.selected_count, local.generated_count,
                        local.instance.as_ref().map(|p| p.display().to_string())
                            .unwrap_or_else(|| "unknown game directory".into())
                    )
                            })
                            .unwrap_or_default();
                    app.texture_report = format!(
                        "Loaded {} PNGs / {} GPU pages ({}px) / {} animated sprites / {} models, manual {}{}. {}",
                        bank.statistics.decoded_png, bank.atlas().map_or(0, |a| a.page_count()),
                        bank.atlas().map_or(0, |a| a.tile_resolution()),
                        bank.animation_count(), bank.model_count(), path.display(), sources,
                        bank.warnings.first().map(String::as_str).or_else(||
                            detected.as_ref().and_then(|s| s.warnings.first().map(String::as_str)))
                            .unwrap_or(""));
                    app.texture_bank = Some(std::sync::Arc::new(
                        entity_visibility::bank_for_display(app, bank),
                    ));
                    app.show_textures = true;
                }
                Err(e) => {
                    app.texture_bank = None;
                    app.surface_assets = None;
                    log::info!("[assets] 资源加载失败：{e}");
                    app.texture_report = format!("资源加载失败：{e}");
                }
            }
        }
        Message::AutoAssetsLoaded(index, root, nonce, result) => {
            if app.current == Some(index)
                && app
                    .worlds
                    .get(index)
                    .is_some_and(|world| world.root == root)
                && nonce == app.texture_load_nonce
                && !app.manual_textures
            {
                app.texture_loading = false;
                match result {
                    Some(Ok((sources, bank))) => {
                        app.texture_report = format!(
                            "Instance {}: vanilla {}, {} framework JARs, {} mod JARs, {} generated packs, {} enabled packs; {} PNG / {} atlas pages / {} models. {}",
                            sources.instance.as_ref().map(|p| p.display().to_string())
                                .unwrap_or_else(|| "unknown game directory".into()),
                            if sources.vanilla { "found" } else { "missing" },
                            sources.framework_count, sources.mod_count, sources.generated_count, sources.selected_count,
                            bank.statistics.decoded_png, bank.atlas().map_or(0, |a| a.page_count()),
                            bank.model_count(),
                            sources.warnings.first().map(String::as_str)
                                .or_else(|| bank.warnings.first().map(String::as_str))
                                .unwrap_or("")
                        );
                        app.texture_bank = Some(std::sync::Arc::new(
                            entity_visibility::bank_for_display(app, bank),
                        ));
                        app.show_textures = true;
                    }
                    Some(Err(error)) => {
                        app.texture_bank = None;
                        app.surface_assets = None;
                        log::info!("[assets] 实例资源加载失败：{error}");
                        app.texture_report = format!("实例资源加载失败：{error}");
                    }
                    None => {
                        app.texture_bank = None;
                        app.surface_assets = None;
                        app.texture_report = "No readable instance assets found; choose a client JAR or resource pack manually.".into();
                    }
                }
            }
        }
        Message::SurfaceArea(value) => {
            app.surface_area = value;
            // Editing a source-window coordinate is not a published scene
            // change. Keep the last valid picture until Reload/Inspect.
            app.far = None;
        }
        Message::InspectSurface => app.read_scene_input(false, true),
        Message::InspectVolume => app.read_scene_input(true, true),
        Message::ReloadScene => app.reload_scene(),
        Message::FollowWorld => app.follow_world(),
        Message::Shift3D(x, y, z) => app.shift_3d(x, y, z),
        Message::Center3DOnPicked => app.center_3d(),
        Message::ScenePreset(side) => app.scene_preset(side),
        Message::StartFarView(radius) => return app.start_far_view(radius, None),
        Message::ShiftFarView(dx, dz) => {
            let Some(far) = app.far.as_ref() else {
                return iced::Task::none();
            };
            let (radius, (x, z)) = (far.radius, far.center);
            let Some(nx) = x.checked_add(dx.saturating_mul(16)) else {
                return iced::Task::none();
            };
            let Some(nz) = z.checked_add(dz.saturating_mul(16)) else {
                return iced::Task::none();
            };
            return app.start_far_view(radius, Some((nx, nz)));
        }
        Message::FarBatchLoaded(nonce, patches) => return app.finish_far_batch(nonce, patches),
        Message::NearLoaded(nonce, key, result) => app.near_loaded(nonce, key, result),
        Message::Toggle3DFrame => app.surface_frame = !app.surface_frame,
        Message::Toggle3DAxes => app.surface_axes = !app.surface_axes,
        Message::Toggle3DChunkGrid => app.surface_chunk_grid = !app.surface_chunk_grid,
        Message::Toggle3DSections => app.surface_section_guides = !app.surface_section_guides,
        Message::Set3DCornerA => app.set_3d_corner(true),
        Message::Set3DCornerB => app.set_3d_corner(false),
        Message::Use3DBox => return selection_edit::update(app, selection_edit::Action::Corners),
        Message::Clear3DBox => return selection_edit::update(app, selection_edit::Action::Clear),
        Message::Cut3DTop(delta) => app.move_cutaway(delta),
        Message::Reset3DCut => {
            app.cut_scene(None);
        }
        Message::SpectatorPick(mut request) => {
            if app.object_inspector.picking_chunks {
                request.intent = picking::Intent::Point;
            }
            let point = matches!(&request.intent, picking::Intent::Point);
            if app.pick_spectator(request) && point {
                if let Some(task) = object_inspector::picked(app) {
                    return task;
                }
                return plugin_ui::tool_click(app, "perspective");
            }
        }
        Message::PickSurfaceFace(x, y, z, face) => {
            if app.pick_surface(x, y, z) {
                app.selected_surface_face = Some(face);
                let created = match app.current_key() {
                    Some(key) => match app.current_session(key) {
                        Ok(session) => {
                            session.creation_origin(x.div_euclid(16), z.div_euclid(16), y)
                        }
                        Err(error) => {
                            log::error!("[pick] {error}");
                            false
                        }
                    },
                    None => false,
                };
                app.surface_report=format!(
                    "Selected {} voxel ({x},{y},{z}) {:?} face; normal {:?}. Edits require preview/Apply.",
                    if created {"new staged"} else {"source"},face,face.normal());
                if let Some(task) = object_inspector::picked(app) {
                    return task;
                }
                return plugin_ui::tool_click(
                    app,
                    if app.show_top_down {
                        "top_down"
                    } else {
                        "isometric"
                    },
                );
            }
        }
        Message::RotateSurface => app.surface_camera = (app.surface_camera + 1) % 4,
        Message::OrbitSurface(deg) => app.surface_yaw = (app.surface_yaw + deg).rem_euclid(360),
        Message::OrbitPitchSurface(yaw, pitch) => {
            app.surface_yaw = (app.surface_yaw + yaw).rem_euclid(360);
            app.surface_pitch = (app.surface_pitch + pitch).clamp(15, 80);
        }
        Message::HoverSurface(view, pos) => world_hover::hover(app, view, pos),
        Message::WorldHover(action) => return world_hover::update(app, action),
        Message::PitchSurface(deg) => app.surface_pitch = (app.surface_pitch + deg).clamp(15, 80),
        Message::Navigate(target, action) => app.navigate(target, action),
        Message::ZoomSurface(n) => app.navigate(
            app.active_view(),
            view_navigation::Action::centered_zoom(n as f32),
        ),
        Message::PanSurface(dx, dy) => {
            app.navigate(
                app.active_view(),
                view_navigation::Action::Pan(iced::Vector::new(dx as f32, dy as f32)),
            );
        }
        Message::ResetSurfaceCamera => {
            let target = app.active_view();
            app.navigate(target, view_navigation::Action::Reset);
            if target == view_navigation::Target::Perspective {
                if let Some(window) = app.selected_window() {
                    app.window_camera = None;
                    app.ensure_window_camera(window);
                } else {
                    app.spectator_camera.yaw = 0.0;
                    app.spectator_camera.pitch = 0.0;
                }
            }
            if target == view_navigation::Target::Isometric {
                app.surface_camera = 0;
                app.surface_yaw = 0;
                app.surface_pitch = 35;
            }
        }
        Message::ClipboardDestination(value) => {
            structure_exchange::revoke(app, true);
            app.clipboard_destination = value;
        }
        request @ (Message::PendingClipboardLoad | Message::PendingClipboardReload(_)) => {
            if let Message::PendingClipboardReload(request) = request {
                if !request.matches(app) {
                    log::debug!("[structure-preview] 已取消或来源改变的排队预览请求退役");
                    return iced::Task::none();
                }
            }
            transfer_edit::cancel(app);
            let budget = match app.block_edit.budget.parse() {
                Ok(value) => value,
                Err(error) => {
                    log::error!("[structure-preview] {error}");
                    structure_exchange::revoke(app, true);
                    app.edit_report = error;
                    return iced::Task::none();
                }
            };
            let Some(key) = app.current_key() else {
                app.edit_report = "请选择世界和维度".into();
                return iced::Task::none();
            };
            let destination = match structure_exchange::destination(app) {
                Ok(point) => point,
                Err(e) => {
                    log::error!("[placement-position] {e}");
                    app.edit_report = e.to_string();
                    return iced::Task::none();
                }
            };
            let show_textures = app.scene_show_textures();
            let atlas = app
                .scene_texture_bank()
                .and_then(|bank| bank.atlas())
                .cloned();
            let session = match app.current_session(key) {
                Ok(s) => s.clone(),
                Err(e) => {
                    app.edit_report = e;
                    return iced::Task::none();
                }
            };
            let revision = session.revision();
            let world = session.original_world().clone();
            let dimension = session.dimension().to_owned();
            let path = PathBuf::from(app.exchange.path.trim());
            let format = app.exchange.format;
            let creation = app.structure_creation.clone();
            let online_skins = app.entity_models.online_skins;
            let language = app.language;
            let existing = app.pending_import.clipboard.clone();
            let transform = match app.exchange.transform() {
                Ok(value) => value,
                Err(e) => {
                    structure_exchange::revoke(app, true);
                    app.edit_report = e;
                    return iced::Task::none();
                }
            };
            let include_entities = app.exchange.source_options().entities;
            let include_biomes = app.exchange.source_options().biomes;
            app.pending_import.begin();
            plugin_ui::clear_structure(app);
            let epoch = app.pending_import.epoch;
            let cancel = std::sync::Arc::clone(&app.pending_import.cancelled);
            app.edit_report = "正在后台预检并构建待放置结构…".into();
            return iced::Task::perform(
                nbt_background(move || {
                    let clip = if let Some(clip) = existing {
                        clip
                    } else {
                        std::sync::Arc::new(
                            format
                                .read_with_budget(&world, &dimension, &path, budget)
                                .map_err(|e| e.to_string())?,
                        )
                    };
                    let mut source = (*clip).clone();
                    if !include_entities {
                        source.entities.clear();
                    }
                    if !include_biomes {
                        source.biomes = None;
                        source.numeric_biomes = None;
                    }
                    let profile = creation.profile(&world, &dimension)?;
                    let (prepared, transformed) = session
                        .prepare_resampled_structure_paste_with_budget_cancellable(
                            &source,
                            transform,
                            destination.into(),
                            mcedit2_world::clipboard::PasteOptions { include_biomes },
                            profile,
                            budget,
                            || cancel.load(std::sync::atomic::Ordering::Acquire),
                        )
                        .map_err(|e| e.to_string())?;
                    let frame = pending_import::Placement::new(
                        &transformed,
                        destination,
                        show_textures,
                        online_skins,
                        language,
                        atlas,
                        prepared.diagnostics.clone(),
                    )?;
                    let mut candidate = session.clone();
                    candidate
                        .commit_structure_cancellable(prepared.clone(), || {
                            cancel.load(std::sync::atomic::Ordering::Acquire)
                        })
                        .map_err(|error| error.to_string())?;
                    if let Some(source) = &clip.source_file {
                        source.verify().map_err(|e| e.to_string())?;
                    }
                    Ok((
                        prepared,
                        std::sync::Arc::new(frame),
                        clip,
                        std::sync::Arc::new(candidate),
                    ))
                }),
                move |result| {
                    Message::PendingLoaded(epoch, key, std::sync::Arc::clone(&revision), result)
                },
            );
        }
        Message::PendingLoaded(epoch, key, revision, result) => {
            if !app.pending_import.finish(epoch)
                || app.current_key() != Some(key)
                || app
                    .sessions
                    .get(&key)
                    .is_none_or(|s| !std::sync::Arc::ptr_eq(&s.revision(), &revision))
            {
                return iced::Task::none();
            }
            match result {
                Ok((preview, frame, clip, candidate)) => {
                    let current_atlas = app.scene_texture_bank().and_then(|bank| bank.atlas());
                    let stale_atlas = current_atlas
                        .is_some_and(|base| !frame.compatible_with(Some(base)))
                        || app
                            .entity_models
                            .frame
                            .as_ref()
                            .filter(|entities| {
                                Some(entities.snapshot.key) == app.current_key()
                                    && entities.show_textures == app.scene_show_textures()
                                    && entities.displays(
                                        app.entity_models.enabled,
                                        app.entity_models.visibility,
                                    )
                                    && current_atlas.is_some_and(|base| {
                                        textures::atlas_compatible(&entities.atlas, base)
                                    })
                            })
                            .is_some_and(|entities| !frame.compatible_with(Some(&entities.atlas)));
                    if stale_atlas
                        || frame.show_textures != app.scene_show_textures()
                        || frame.online_skins != app.entity_models.online_skins
                        || frame.language != app.language
                    {
                        app.pending_import.clipboard = Some(clip);
                        return crate::pending_import::reload(app);
                    }
                    for diagnostic in &frame.diagnostics {
                        log::error!("[pending-import] {diagnostic}");
                    }
                    let details = frame.diagnostics.join("；");
                    app.edit_report = format!(
                        "待放置预览已就绪。覆盖 {} 个方块实体、{} 个容器物品槽。{}",
                        preview.overwritten_block_entities,
                        preview.overwritten_inventory_stacks,
                        details
                    );
                    app.pending_import.frame = Some(frame);
                    app.pending_import.clipboard = Some(clip);
                    plugin_ui::offer_structure(app, key, preview, candidate, revision);
                    placement::show_preview(app);
                }
                Err(e) => {
                    log::error!("[pending-import] {e}");
                    structure_exchange::revoke(app, true);
                    app.edit_report = format!("待放置预览失败：{e}");
                }
            }
        }
        Message::PendingMove(dx, dy, dz) => {
            structure_exchange::revoke(app, true);
            let [x, y, z] = match structure_exchange::destination(app) {
                Ok(v) => v,
                Err(e) => {
                    log::error!("[placement-position] {e}");
                    app.edit_report = e.to_string();
                    return iced::Task::none();
                }
            };
            match (x.checked_add(dx), y.checked_add(dy), z.checked_add(dz)) {
                (Some(x), Some(y), Some(z)) => {
                    structure_exchange::set_destination(app, [x, y, z]);
                    return crate::pending_import::reload(app);
                }
                _ => app.edit_report = "放置位置溢出".into(),
            }
        }
        Message::PendingRotate => return structure_exchange::rotate_y(app),
        Message::PendingClipboard(clip) => {
            let destination = clip.source_min;
            structure_exchange::install_clipboard(app, clip, destination);
            return transfer_edit::begin_import_placement(app, "导入结构");
        }
        Message::PendingCancel => {
            transfer_edit::cancel(app);
            app.pending_import.cancel();
            plugin_ui::clear_structure(app);
            app.edit_report = "已取消待放置预览".into();
        }
        Message::PickSlice(x, y, z) => {
            app.selected_block = None;
            app.pick_slice(x, y, z);
            if app.selected_block.is_some() {
                if let Some(task) = object_inspector::picked(app) {
                    return task;
                }
                return plugin_ui::tool_click(app, "horizontal");
            }
        }
        Message::MoveSlice(dx, dy, dz) => app.pan_slice(dx, dy, dz),
        Message::SavePhrase(value) => app.save_phrase = value,
        Message::BeSavePhrase(value) => app.be_save_phrase = value,
        Message::PrepareBeSave => return app.prepare_be_save(),
        Message::ConfirmBeSave => return app.confirm_be_save(),
        Message::RecoverRegion(value) => app.recover_region = value,
        Message::RecoveryPhrase(value) => app.recovery_phrase = value,
        Message::PrepareWorldSave => return app.prepare_save(),
        Message::ConfirmWorldSave => return app.confirm_save(),
        Message::FinalizeRegion => return app.recover_region(false),
        Message::RollbackRegion => return app.recover_region(true),
        Message::FinalizeRegionGroup => return app.recover_group(false),
        Message::RollbackRegionGroup => return app.recover_group(true),
        Message::PickWorld => return dialogs::world_folder(app),
        Message::WorldChosen(path) => {
            if let Some(path) = path {
                app.path = path.to_string_lossy().to_string();
                return update_inner(app, Message::Open(path, None));
            }
        }
        Message::ChooseTextureFile => return dialogs::resource_file(app),
        Message::ChooseTextureDirectory => return dialogs::resource_directory(app),
        Message::ToggleLanguage => {
            app.language = app.language.toggle();
            if app.pending_import.clipboard.is_some() {
                return crate::pending_import::reload(app);
            }
        }
        Message::ToggleAdvanced => app.show_advanced = !app.show_advanced,
        Message::Open(path, _) => {
            app.path = path.to_string_lossy().into_owned();
            app.invalidate_near();
            app.far = None;
            app.be_prepared = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.surface = None;
            app.save_preview = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.slice_grid = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.vertical_grid = None;
            app.open_nonce = app.open_nonce.wrapping_add(1);
            app.open_pending = None;
            if path.as_os_str().is_empty() {
                app.messages
                    .push("Enter a Java world folder or level.dat path".into());
            } else {
                app.open_pending = Some((app.open_nonce, path));
                app.messages
                    .push("Indexing original Java world in a read-only worker.".into());
            }
        }
        Message::OpenFinished(nonce, result) => {
            // Release the single indexer slot EVEN if another world selection
            // superseded this read. A queued latest open can then start.
            app.open_busy = false;
            let readonly = file_input::take_world_access(app, nonce);
            if nonce == app.open_nonce {
                match result {
                    Ok((world, terrain_y, locations)) => {
                        let root = world.root.clone();
                        let task = workspace_tabs::opened_with_access(
                            app, world, terrain_y, locations, readonly,
                        );
                        return iced::Task::batch([
                            task,
                            file_input::world_opened(app, nonce, Ok(root)),
                        ]);
                    }
                    Err(e) => {
                        log::error!("打开世界失败：{e}");
                        app.messages.push(format!("Cannot inspect world: {e}"));
                        return file_input::world_opened(app, nonce, Err(e));
                    }
                }
            } else {
                return file_input::world_opened(
                    app,
                    nonce,
                    Err("世界读取期间已选择其他文档，未继续打开后续文件".into()),
                );
            }
        }
        Message::Select(index) => {
            app.structures.active = None;
            app.open_nonce = app.open_nonce.wrapping_add(1);
            app.open_pending = None;
            app.be_prepared = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.surface = None;
            app.save_preview = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.slice_grid = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.vertical_grid = None;
            if index < app.worlds.len() {
                app.workspace.open(index);
                let next_world = app.worlds[index].clone();
                app.reset_assets_for_world(&next_world);
                app.dimension_index = app.initial_dimension(&app.worlds[index]);
                app.current = Some(index);
                app.block_result.clear();
                app.slice_result.clear();
                app.clear_block_preview();
                app.edit_report.clear();
                let y = app
                    .worlds
                    .get(index)
                    .map(|world| initial_scene_height(world, app.dimension_index))
                    .unwrap_or(64);
                app.open_default_scene(y);
                workspace_tabs::restore(app, index);
                desktop_state::activated(app, index);
            }
        }
        Message::Coordinates(value) => {
            app.query_coordinates = value;
            app.be_prepared = None;
        }
        Message::Dimension(index) => {
            app.structures.active = None;
            app.open_nonce = app.open_nonce.wrapping_add(1);
            app.open_pending = None;
            app.be_prepared = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.surface = None;
            app.save_preview = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.slice_grid = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.vertical_grid = None;
            if app
                .current
                .and_then(|i| app.worlds.get(i))
                .is_some_and(|w| index < w.dimensions.len())
            {
                app.dimension_index = index;
                if let Some(world) = app.current {
                    app.workspace
                        .session_dimensions
                        .insert(world, app.worlds[world].dimensions[index].name.clone());
                }
                app.block_result.clear();
                app.slice_result.clear();
                app.clear_block_preview();
                app.edit_report.clear();
                let y = app
                    .current
                    .and_then(|i| app.worlds.get(i))
                    .map(|world| initial_scene_height(world, index))
                    .unwrap_or(64);
                app.open_default_scene(y);
            }
        }
        Message::InspectBlock => {
            app.selected_block = None;
            app.block_result = match app.current.and_then(|i| app.worlds.get(i)) {
                None => "Select an indexed original Java world first.".into(),
                Some(world) => match world.dimensions.get(app.dimension_index) {
                    None => "Selected world has no indexed dimensions.".into(),
                    Some(dimension) => match parse_xyz(&app.query_coordinates) {
                        Err(e) => format!("Invalid query coordinates: {e}"),
                        Ok((x, y, z)) => {
                            let key = (app.current.unwrap_or_default(), app.dimension_index);
                            let result = if let Some(session) = app.sessions.get(&key) {
                                session.visible_block((x, y, z))
                            } else {
                                read_world_block(world, &dimension.name, x, y, z)
                            };
                            match result {
                                Ok(record) => {
                                    if let mcedit2_world::block_lookup::Lookup::Present(block) =
                                        &record
                                    {
                                        app.selected_block = Some(block_names::Selection {
                                            key,
                                            position: (x, y, z),
                                            state: block.state.clone(),
                                        });
                                    }
                                    record.description()
                                }
                                Err(e) => format!("Original block read refused: {e}"),
                            }
                        }
                    },
                },
            };
        }
        Message::BeScalarField(value) => {
            app.be_scalar_field = value;
            app.be_prepared = None;
        }
        Message::EntitySnapshotPath(value) => app.entity_snapshot_path = value,
        Message::InspectEntities => {
            let outcome = (|| -> Result<String, String> {
                let p = parse_xyz(&app.query_coordinates).map_err(|e| e.to_string())?;
                let key = app.current_key().ok_or("Select original world")?;
                let report = app
                    .current_session(key)?
                    .inspect_entities(p.0.div_euclid(16), p.2.div_euclid(16))
                    .map_err(|e| e.to_string())?;
                let mut out = format!(
                    "Original {} chunk {:?}, {} entities, SHA-256 {}. READ ONLY.\n",
                    report.source,
                    report.chunk,
                    report.entities.len(),
                    report.source_sha256
                );
                for e in report.entities.iter().take(64) {
                    out.push_str(&format!(
                        "id={} Pos={:?} fields={} original NBT={} bytes SHA={}\n",
                        e.id.as_deref().unwrap_or("unknown"),
                        e.pos,
                        e.fields,
                        e.bytes,
                        e.sha256
                    ));
                }
                Ok(out)
            })();
            app.block_result =
                outcome.unwrap_or_else(|e| format!("Entity inspection refused: {e}"));
        }
        Message::ExportEntities => {
            let file = PathBuf::from(app.entity_snapshot_path.trim());
            let outcome = (|| -> Result<usize, String> {
                let p = parse_xyz(&app.query_coordinates).map_err(|e| e.to_string())?;
                let key = app.current_key().ok_or("Select original world")?;
                app.current_session(key)?
                    .export_original_entities(p.0.div_euclid(16), p.2.div_euclid(16), &file)
                    .map_err(|e| e.to_string())
            })();
            app.block_result = match outcome {
                Ok(n) => format!(
                    "Exported {n} bytes of exact original entity snapshot. World unchanged."
                ),
                Err(e) => format!("Entity snapshot export refused: {e}"),
            };
        }
        Message::VerifyEntities => {
            let file = PathBuf::from(app.entity_snapshot_path.trim());
            let outcome = app
                .current_key()
                .ok_or_else(|| "Select original world".to_string())
                .and_then(|key| app.current_session(key))
                .and_then(|editor| {
                    editor
                        .verify_original_entities(&file)
                        .map_err(|e| e.to_string())
                });
            app.block_result = match outcome {
                Ok(n) => format!("Verified {n} original entity NBT bytes against current source."),
                Err(e) => format!("Entity snapshot verification refused: {e}"),
            };
        }
        Message::BeSnapshotPath(value) => app.be_snapshot_path = value,
        Message::BeJournalPath(value) => app.be_journal_path = value,
        Message::BeBatchPath(value) => app.be_batch_path = value,
        Message::RegionBatchPath(value) => app.region_batch_path = value,
        Message::PreviewRegionBatch => {
            let result = (|| -> Result<_, String> {
                let key = app.current_key().ok_or("Select original world")?;
                let time = u32::try_from(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| "System clock before UNIX epoch")?
                        .as_secs(),
                )
                .map_err(|_| "Region timestamp cannot represent system clock")?;
                app.current_session(key)?
                    .prepare_region_batch_report(time)
                    .map_err(|e| e.to_string())
            })();
            app.edit_report=match result{
                Ok(report)=>format!("Verified {} original Region candidates across this dimension, {} BE scalar proof steps. READ-ONLY: multi-Region save is not atomic or available.",report.regions.len(),report.regions.iter().map(|r|r.be_scalar_steps).sum::<usize>()),
                Err(e)=>format!("Region batch preflight refused: {e}"),
            };
        }
        Message::ExportRegionBatch => {
            let path = PathBuf::from(app.region_batch_path.trim());
            let result = (|| -> Result<usize, String> {
                let key = app.current_key().ok_or("Select original world")?;
                let time = u32::try_from(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| "Invalid system time")?
                        .as_secs(),
                )
                .map_err(|_| "Invalid Region timestamp")?;
                app.current_session(key)?
                    .export_region_batch_report(time, &path)
                    .map_err(|e| e.to_string())
            })();
            app.edit_report=match result{
                Ok(n)=>format!("Wrote create-new original-byte dry-run manifest for {n} Region files; no Minecraft files changed."),
                Err(e)=>format!("Region batch export refused: {e}"),
            };
        }
        Message::VerifyRegionBatch => {
            let path = PathBuf::from(app.region_batch_path.trim());
            let result = app
                .current_key()
                .ok_or_else(|| "Select original world".to_string())
                .and_then(|key| app.current_session(key))
                .and_then(|session| {
                    session
                        .verify_region_batch_report(&path)
                        .map_err(|e| e.to_string())
                });
            app.edit_report=match result{
                Ok(n)=>format!("Verified {n} exact Region candidates against original world bytes and current overlay; no disk writes."),
                Err(e)=>format!("Region manifest verification refused: {e}"),
            };
        }
        Message::PreviewBeBatch => {
            let file = PathBuf::from(app.be_batch_path.trim());
            let result = app
                .current_key()
                .ok_or_else(|| "Select a world".to_string())
                .and_then(|key| app.current_session(key))
                .and_then(|session| {
                    session
                        .preview_be_operation_file(&file)
                        .map_err(|e| e.to_string())
                });
            app.edit_report=match result{
                Ok((count,chunks))=>format!("Validated all {count} BE scalar edits against source NBT across {chunks} chunks. Nothing staged or written."),
                Err(e)=>format!("BE batch preview refused: {e}"),
            };
        }
        Message::StageBeBatch => {
            if app.readonly {
                app.edit_report = "-view forbids BE changes".into();
            } else {
                let file = PathBuf::from(app.be_batch_path.trim());
                let result = app
                    .current_key()
                    .ok_or_else(|| "Select a world".to_string())
                    .and_then(|key| app.current_session(key))
                    .and_then(|session| {
                        session
                            .stage_be_operation_file(&file)
                            .map_err(|e| e.to_string())
                    });
                app.edit_report=match result{
                    Ok(n)=>format!("Atomically staged all {n} BE scalar operations IN MEMORY. Existing BE Undo available; world disk unchanged."),
                    Err(e)=>format!("Entire BE batch refused: {e}"),
                };
            }
        }
        Message::ExportBeJournal => {
            let file = PathBuf::from(app.be_journal_path.trim());
            let result = app
                .current_key()
                .ok_or_else(|| "Select an original world".to_string())
                .and_then(|key| app.current_session(key))
                .and_then(|editor| editor.export_be_journal(&file).map_err(|e| e.to_string()));
            app.edit_report=match result{
                Ok(n)=>format!("Exported {n} source-hashed BE scalar operations to detached create-new journal; original world untouched."),
                Err(e)=>format!("BE checkpoint export refused: {e}"),
            };
        }
        Message::ImportBeJournal => {
            if app.readonly {
                app.edit_report = "-view forbids BE journal import.".into();
            } else {
                let file = PathBuf::from(app.be_journal_path.trim());
                let result = app
                    .current_key()
                    .ok_or_else(|| "Select an original world".to_string())
                    .and_then(|key| app.current_session(key))
                    .and_then(|editor| editor.import_be_journal(&file).map_err(|e| e.to_string()));
                app.edit_report=match result{
                    Ok(n)=>format!("Restored {n} verified BE scalar changes IN MEMORY. Each step separately undoable; Region world save remains blocked."),
                    Err(e)=>format!("BE journal restore refused without modifying session: {e}"),
                };
            }
        }
        Message::PluginManifestPath(value) => app.plugin_manifest_path = value,
        Message::PreviewPlugin => {
            app.clear_block_preview();
            app.save_preview = None;
            let filename = PathBuf::from(app.plugin_manifest_path.trim());
            let result = (|| -> Result<Preview, String> {
                let key = app
                    .current_key()
                    .ok_or("Select an original world/dimension")?;
                app.current_session(key)?
                    .prepare_plugin_file(&filename)
                    .map_err(|e| e.to_string())
            })();
            match result {
                Ok(preview) => {
                    app.edit_report=format!("Declarative plugin PREVIEW: {}. Stage using existing Undo-able Apply control; no arbitrary executable is run.",preview.summary());
                    if let Some((wi, di)) = app.current_key() {
                        app.offer_block_preview((wi, di, preview));
                    }
                }
                Err(e) => app.edit_report = format!("Plugin manifest preflight refused: {e}"),
            }
        }
        Message::ExportOriginalBe => {
            let path = PathBuf::from(app.be_snapshot_path.trim());
            let result = (|| -> Result<usize, String> {
                let point = parse_xyz(&app.query_coordinates).map_err(|e| e.to_string())?;
                let key = app.current_key().ok_or("Select original world/dimension")?;
                app.current_session(key)?
                    .export_original_be_snapshot(point, &path)
                    .map_err(|e| e.to_string())
            })();
            app.block_result=match result{
                Ok(size)=>format!("Exported immutable original Block Entity NBT ({} raw bytes) to external snapshot. World untouched.",size),
                Err(e)=>format!("Opaque BE snapshot export refused: {e}"),
            };
        }
        Message::VerifyOriginalBe => {
            let path = PathBuf::from(app.be_snapshot_path.trim());
            let result = (|| -> Result<((i32, i32, i32), usize), String> {
                let key = app.current_key().ok_or("Select original world/dimension")?;
                app.current_session(key)?
                    .verify_original_be_snapshot(&path)
                    .map_err(|e| e.to_string())
            })();
            app.block_result=match result{
                Ok((xyz,size))=>format!("Verified {} exact original NBT bytes at XYZ {:?} against the current source chunk; no data imported or modified.",size,xyz),
                Err(e)=>format!("Opaque BE snapshot verification refused: {e}"),
            };
        }
        Message::BeScalarValue(value) => {
            app.be_scalar_value = value;
            app.be_prepared = None;
        }
        Message::PreviewBeScalar => {
            app.be_prepared = None;
            let outcome = (|| -> Result<_, String> {
                let point = parse_xyz(&app.query_coordinates).map_err(|e| e.to_string())?;
                let key = app
                    .current_key()
                    .ok_or("Select original world and dimension")?;
                let field = app.be_scalar_field.clone();
                let value = app.be_scalar_value.clone();
                let proposal = app
                    .current_session(key)?
                    .prepare_be_edit(point, &field, &value)
                    .map_err(|e| e.to_string())?;
                Ok((key.0, key.1, proposal))
            })();
            match outcome {
                Ok((wi, di, preview)) => {
                    app.block_result=format!(
                        "Verified BE scalar proposal: {} NBT bytes, SHA-256 {}. STAGE is memory-only; Region saving blocked while BE edits remain.",
                        preview.patched_nbt.len(),
                        preview.sha256()
                    );
                    app.be_prepared = Some((wi, di, preview));
                }
                Err(e) => app.block_result = format!("BE preview refused: {e}"),
            }
        }
        Message::StageBeScalar => {
            if app.readonly {
                app.block_result = "-view forbids BE staging.".into();
            } else {
                let outcome = app
                    .be_prepared
                    .take()
                    .ok_or("Preview original BE scalar change first".to_owned())
                    .and_then(|(wi, di, prepared)| {
                        if app.current_key() != Some((wi, di)) {
                            return Err("Changed dimension/world since BE preview".into());
                        }
                        app.current_session((wi, di))?
                            .commit_be_edit(prepared)
                            .map_err(|e| e.to_string())
                    });
                app.block_result=match outcome{
                    Ok(())=>"BE NBT staged with separate Undo; Minecraft world unchanged and disk save BLOCKED.".into(),
                    Err(e)=>format!("BE stage refused: {e}"),
                };
            }
        }
        Message::UndoBeScalar => {
            if app.readonly {
                app.block_result = "-view forbids BE Undo.".into();
            } else {
                let r = app
                    .current_key()
                    .ok_or("Select original world".to_owned())
                    .and_then(|k| app.current_session(k))
                    .and_then(|s| s.undo_be_edit().map_err(|e| e.to_string()));
                app.block_result = match r {
                    Ok(true) => "BE scalar Undo completed in memory.".into(),
                    Ok(false) => "No BE scalar Undo history.".into(),
                    Err(e) => format!("BE Undo refused: {e}"),
                };
            }
        }
        Message::RedoBeScalar => {
            if app.readonly {
                app.block_result = "-view forbids BE Redo.".into();
            } else {
                let r = app
                    .current_key()
                    .ok_or("Select original world".to_owned())
                    .and_then(|k| app.current_session(k))
                    .and_then(|s| s.redo_be_edit().map_err(|e| e.to_string()));
                app.block_result = match r {
                    Ok(true) => "BE scalar Redo completed in memory.".into(),
                    Ok(false) => "No BE scalar Redo history.".into(),
                    Err(e) => format!("BE Redo refused: {e}"),
                };
            }
        }
        Message::PreviewBeRegion => {
            app.save_preview = None;
            let result = app
                .current_key()
                .ok_or_else(|| "Select original world".to_string())
                .and_then(|key| app.current_session(key))
                .and_then(|editor| {
                    editor
                        .preview_be_region_candidates(0)
                        .map_err(|e| e.to_string())
                });
            app.edit_report=match result{
                Ok(regions)=>format!("BE-AUTHENTICATED IN-MEMORY Region preview: {} affected Region files; {} total bytes in candidates. Disk saving requires the separate fresh SAVE_BE_NBT transaction and backup confirmation.",regions.len(),
                    regions.iter().map(|r|r.planned_bytes).sum::<usize>()),
                Err(e)=>format!("BE-aware Region preview refused: {e}"),
            };
        }
        Message::InspectStagedBe => {
            let r = (|| -> Result<String, String> {
                let p = parse_xyz(&app.query_coordinates).map_err(|e| e.to_string())?;
                let k = app.current_key().ok_or("Select original world")?;
                Ok(app
                    .current_session(k)?
                    .inspect_staged_be(p)
                    .map_err(|e| e.to_string())?
                    .map_or_else(|| "No original/staged BE found.".into(), |be| be.report()))
            })();
            app.block_result = r.unwrap_or_else(|e| format!("BE inspection refused: {e}"));
        }
        Message::InspectBeTree => {
            let result = (|| -> Result<String, String> {
                let pos = parse_xyz(&app.query_coordinates).map_err(|e| e.to_string())?;
                let key = app.current_key().ok_or("Select an original world")?;
                let tree = app
                    .current_session(key)?
                    .inspect_original_be_tree(pos)
                    .map_err(|e| e.to_string())?;
                if tree.is_empty() {
                    return Ok("No original BE NBT at this XYZ.".into());
                }
                let mut message=format!("Original modded BE tree: {} visible nested paths (read only). Safely editable scalar paths marked EDIT; no arrays/list elements are edited.\n",tree.len());
                for item in tree {
                    message.push_str(&format!(
                        "{} kind={} bytes={} children={} [{}]\n",
                        item.path,
                        item.kind,
                        item.payload_bytes,
                        item.immediate_children,
                        if item.safely_editable_scalar {
                            "EDIT"
                        } else {
                            "READ ONLY"
                        }
                    ));
                }
                Ok(message)
            })();
            app.block_result = result.unwrap_or_else(|e| format!("BE tree refused: {e}"));
        }
        Message::InspectOriginalBlockEntity => {
            let result = (|| -> Result<String, String> {
                let point = parse_xyz(&app.query_coordinates).map_err(|e| e.to_string())?;
                let key = app.current_key().ok_or("Select an original Java world")?;
                let be = app
                    .current_session(key)?
                    .inspect_original_block_entity(point)
                    .map_err(|e| e.to_string())?;
                Ok(match be {
                    Some(data) => data.report(),
                    None => format!(
                        "No ORIGINAL Block Entity is recorded at ({},{},{}).",
                        point.0, point.1, point.2
                    ),
                })
            })();
            app.block_result =
                result.unwrap_or_else(|e| format!("Block Entity inspection refused: {e}"));
        }
        Message::VerticalText(value) => {
            app.vertical_area = value;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.vertical_grid = None;
        }
        Message::InspectVertical => app.inspect_vertical_view(),
        Message::PickVertical(row, col) => {
            app.selected_block = None;
            app.pick_vertical(row, col);
            if app.selected_block.is_some() {
                if let Some(task) = object_inspector::picked(app) {
                    return task;
                }
                return plugin_ui::tool_click(app, "vertical");
            }
        }
        Message::SliceText(value) => {
            app.slice_area = value;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.slice_grid = None;
        }
        Message::InspectSlice => app.inspect_slice_view(),
        Message::SelectionText(value) => selection_edit::raw(app, value),
        Message::TargetText(value) => {
            app.edit_target = value;
            app.clear_block_preview();
            app.save_preview = None;
            app.save_phrase.clear();
        }
        Message::FilterText(value) => {
            app.edit_filter = value;
            app.clear_block_preview();
            app.save_preview = None;
            app.save_phrase.clear();
        }
        Message::AnalyzeRegionPlan => {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| u32::try_from(d.as_secs()).ok());
            app.edit_report=match (app.current_key(),timestamp){
                (None,_)=>"Select an original Java world first.".into(),
                (_,None)=>"System clock outside supported Region timestamp range.".into(),
                (Some(key),Some(timestamp))=>app.current_session(key)
                    .and_then(|editor|editor.prepare_native_region_candidates(timestamp)
                        .map_err(|e|e.to_string()))
                    .map(|plans|format!(
                        "Original Region dry-run: {} Region images, {} changed chunks, {} original bytes -> {} candidate bytes. NO FILES WRITTEN.",
                        plans.len(),
                        plans.iter().map(|p|p.modified_chunks).sum::<usize>(),
                        plans.iter().map(|p|p.original_bytes).sum::<usize>(),
                        plans.iter().map(|p|p.planned_bytes).sum::<usize>(),
                    ))
                    .unwrap_or_else(|e|format!("Region dry-run refused: {e}")),
            };
        }
        Message::AnalyzeNativePatch => {
            app.edit_report=match app.current_key(){
                None=>"Open a world before analyzing native NBT changes.".into(),
                Some(key)=>app.current_session(key)
                    .and_then(|editor|editor.prepare_native_palette_candidates().map_err(|e|e.to_string()))
                    .map(|chunks|format!("Original-byte in-memory NBT preview: {} chunks and {} blocks eligible; NO Minecraft files saved.",
                        chunks.len(),chunks.iter().map(|v|v.edited_blocks).sum::<usize>()))
                    .unwrap_or_else(|e|format!("Lossless native save-preview REFUSED: {e}")),
            };
        }
        Message::PatchPath(value) => app.patch_path = value,
        Message::ExportPatch => {
            let key = app.current_key();
            let path = PathBuf::from(app.patch_path.trim());
            app.edit_report=match key {
                None=>"Select a Java world before exporting a journal.".into(),
                Some(key)=>app.current_session(key)
                    .and_then(|editor|editor.export_patch(&path).map_err(|e|e.to_string()))
                    .map(|n|format!("Exported {n} original-NBT-verified block edits to external journal. Minecraft world NOT saved."))
                    .unwrap_or_else(|e|format!("Journal export refused: {e}")),
            };
        }
        Message::ImportPatch => {
            if app.readonly {
                app.edit_report = "-view prohibits importing staged edits.".into();
            } else {
                let key = app.current_key();
                let path = PathBuf::from(app.patch_path.trim());
                app.edit_report=match key {
                    None=>"Select a Java world before importing a journal.".into(),
                    Some(key)=>app.current_session(key)
                        .and_then(|editor|editor.import_patch(&path).map_err(|e|e.to_string()))
                        .map(|n|format!("Restored {n} verified block edits as one Undo group. World files still untouched."))
                        .unwrap_or_else(|e|format!("Journal import refused with no staged mutations: {e}")),
                };
            }
        }
        Message::FillSeed(value) => {
            app.fill_seed = value;
            app.clear_block_preview();
            app.save_preview = None;
            app.save_phrase.clear();
        }
        Message::PreviewFill => return flood_fill::update(app, flood_fill::Action::Preview),
        Message::PreviewBrush(shape) => block_edit::start(app, block_edit::Tool::Brush(shape)),
        Message::CloneDestination(value) => {
            transfer_edit::cancel(app);
            app.clone_destination = value;
            app.clear_block_preview();
            app.save_preview = None;
            app.save_phrase.clear();
        }
        Message::PreviewClone => {
            return transfer_edit::update(
                app,
                transfer_edit::Action::Preview(mcedit2_world::editing::TransferKind::Clone),
            )
        }
        Message::PreviewMove => {
            return transfer_edit::update(
                app,
                transfer_edit::Action::Preview(mcedit2_world::editing::TransferKind::Move),
            )
        }
        Message::PreviewEdit => block_edit::start(app, block_edit::Tool::Selection),
        Message::ApplyEdit => {
            if app.prepared.is_some() || app.block_edit.has_prepared() {
                return block_edit::apply(app);
            } else if app.plugins.candidate_active() {
                return plugin_ui::update(app, plugin_ui::Action::ApplyStructure);
            } else {
                return transfer_edit::update(app, transfer_edit::Action::Apply);
            }
        }
        Message::UndoEdit => {
            return transfer_edit::update(app, transfer_edit::Action::History(false))
        }
        Message::RedoEdit => {
            return transfer_edit::update(app, transfer_edit::Action::History(true))
        }
        Message::DiscardEdits => {
            app.be_save_phrase.clear();
            if let Some(key) = app.current_key() {
                if let Some(editor) = app.sessions.get_mut(&key) {
                    editor.discard();
                }
            }
            app.edit_report =
                "Discarded all staged edits for this world/dimension; original disk untouched."
                    .into();
        }
        Message::PluginApplicationContinue => {
            // 外层 update 的 application::maintain 消费该唤醒，不改变任何文档。
        }
        Message::WorkspaceIdle => {
            app.selection_edit.clear_workspace();
            app.selection_clipboard.cancel();
            app.nbt_documents.clear();
            app.open_nonce = app.open_nonce.wrapping_add(1);
            app.open_pending = None;
            app.be_save_phrase.clear();
            app.be_prepared = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.surface = None;
            app.selected_scene = None;
            app.window_camera = None;
            app.world_orthographic_framing = Default::default();
            app.far = None;
            app.save_preview = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.slice_grid = None;
            app.grid_main.cancel();
            app.grid_horizontal.cancel();
            app.grid_vertical.cancel();
            app.grid_revision = app.grid_revision.wrapping_add(1);
            app.vertical_grid = None;
            app.recovery_phrase.clear();
            app.save_phrase.clear();
            app.path.clear();
            app.restart_asset_request();
            app.texture_loading = false;
            app.texture_bank = None;
            app.manual_textures = false;
            app.auto_assets_attempted.clear();
            app.texture_report.clear();
            app.source_locations.clear();
            app.workspace.tabs.clear();
            app.workspace.closing = None;
            app.sessions.clear();
            app.clear_block_preview();
            app.edit_report.clear();
            app.current = None;
            app.dimension_index = 0;
            app.query_coordinates = "0, 64, 0".into();
            app.block_result.clear();
            app.slice_result.clear();
            app.messages.clear();
        }
    }
    iced::Task::none()
}
fn render_surface<'a>(
    app: &'a App,
    view: &'a std::sync::Arc<SurfaceView>,
    height: Length,
) -> Element<'a, Message> {
    let lighting = &crate::lighting::Table::unlit();
    let content = navigation_bindings::wrap(
        app,
        app.active_view(),
        render_surface_content(app, view, height, lighting),
    );
    let content = if app.show_top_down {
        world_ruler::surface(
            content,
            view,
            app.top_down_navigation,
            height,
            app.language,
            app.minimap
                .capture(app.current_key(), view_navigation::Target::TopDown),
            1.,
        )
    } else {
        sky_background::wrap(content, height, 1.)
    };
    content
}
fn render_surface_content<'a>(
    app: &'a App,
    view: &'a std::sync::Arc<SurfaceView>,
    height: Length,
    lighting: &std::sync::Arc<lighting::Table>,
) -> Element<'a, Message> {
    // 世界网格直接使用实体网格，不需要受限投影或其图集前缀检查。
    // 避免在自由视角的每次响应式视图重建中争用投影缓存锁。
    let projected_entity_frame =
        if app.show_top_down || !((app.spectator && app.use_gpu) || app.uses_world_mesh()) {
            app.entity_models
                .frame
                .as_ref()
                .filter(|frame| {
                    frame.displays(app.entity_models.enabled, app.entity_models.visibility)
                        && frame.show_textures == app.scene_show_textures()
                        && Some(frame.snapshot.key) == app.current_key()
                        && app
                            .scene_layers
                            .snapshot
                            .as_ref()
                            .is_some_and(|s| std::sync::Arc::ptr_eq(s, &frame.snapshot))
                        && app
                            .scene_texture_bank()
                            .and_then(|b| b.atlas())
                            .is_some_and(|atlas| textures::atlas_compatible(&frame.atlas, atlas))
                })
                .cloned()
        } else {
            None
        };
    let surface_canvas = IsometricSurface {
        selection_handle: crate::selection_handle::config(app),
        brush: brush_stroke::config(app),
        brush_key: app.current_key(),
        surface: std::sync::Arc::clone(view),
        camera_quarters: app.surface_camera,
        texture_bank: app.scene_texture_bank().cloned(),
        show_textures: app.scene_show_textures(),
        block_layers: app.scene_block_layers(),
        yaw_degrees: app.surface_yaw,
        pitch_degrees: app.surface_pitch,
        zoom: app.surface_navigation.zoom,
        pan_x: app.surface_navigation.pan.x,
        pan_y: app.surface_navigation.pan.y,
        selected: app.selected_surface,
        hovered: app.surface_hover,
        selection_box: app.selection_box(),
        show_frame: app.surface_frame,
        show_axes: app.surface_axes,
        show_chunk_grid: app.surface_chunk_grid,
        show_section_guides: app.surface_section_guides,
        navigation_target: crate::view_navigation::Target::Isometric,
        gpu_faces: app.use_gpu,
        gpu_quality: app.gpu_quality,
        frame_cache: std::sync::Arc::clone(&app.render_face_cache),
        framing_cache: std::sync::Arc::clone(&app.render_framing_cache),
        canvas_cache: std::sync::Arc::clone(&app.render_canvas_cache),
        gpu_frame_cache: std::sync::Arc::clone(&app.render_gpu_cache),
        gpu_pending: std::sync::Arc::clone(&app.render_gpu_pending),
        entity_frame: projected_entity_frame.clone(),
        entity_clip: {
            let bounds = app
                .selected_window()
                .map(|window| {
                    let mut b = window.bounds;
                    if let Some(top) = window.cut {
                        b.maximum[1] = b.maximum[1].min(top);
                    }
                    b
                })
                .unwrap_or(mcedit2_world::editing::Bounds {
                    minimum: [view.x, view.y_min, view.z],
                    maximum: [
                        view.x
                            .saturating_add(i32::from(view.width).saturating_sub(1)),
                        view.y_max,
                        view.z
                            .saturating_add(i32::from(view.depth).saturating_sub(1)),
                    ],
                });
            Some((
                bounds.minimum.map(|v| f64::from(v) - 0.5),
                bounds.maximum.map(|v| f64::from(v) + 0.5),
            ))
        },
        rendered_cache: std::sync::Arc::clone(&app.projected_ready),
        presented: std::sync::Arc::clone(&app.projected_presented),
        lighting: std::sync::Arc::clone(lighting),
    };
    let drop_surface = surface_canvas.clone();
    let annotation_projection = if app.spectator && app.use_gpu {
        scene_layers::ProjectionSource::Presented(std::sync::Arc::clone(
            &app.spectator_presented_view,
        ))
    } else if app.spectator {
        scene_layers::ProjectionSource::Free(app.spectator_camera)
    } else if app.uses_world_mesh() {
        scene_layers::ProjectionSource::World {
            presented: std::sync::Arc::clone(&app.spectator_presented),
            frame: std::sync::Arc::clone(&app.spectator_presented_view),
            view: app.world_orthographic_view(),
        }
    } else {
        scene_layers::ProjectionSource::Bounded(surface_canvas.clone())
    };
    let stage: Element<'_, Message> = if app.show_top_down {
        TopDown {
            selection_handle: crate::selection_handle::config(app),
            chunk_column: object_inspector::selected_column(app),
            brush: brush_stroke::config(app),
            brush_key: app.current_key(),
            layers: app
                .scene_layers
                .view_for(app.current_key(), scene_layers::SourceRef::Surface(view)),
            source: std::sync::Arc::clone(view),
            zoom: app.top_down_navigation.zoom,
            pan_x: app.top_down_navigation.pan.x,
            pan_y: app.top_down_navigation.pan.y,
            selected: app.selected_surface,
            texture_bank: app.scene_texture_bank().cloned(),
            show_textures: app.scene_show_textures(),
            block_layers: app.scene_block_layers(),
            entity_frame: projected_entity_frame,
            grid_cache: std::sync::Arc::clone(&app.grid_main),
            lighting: std::sync::Arc::clone(lighting),
            quality: app.gpu_quality,
            source_revision: app.grid_revision,
        }
        .element(app, height)
    } else if (app.spectator && app.use_gpu) || app.uses_world_mesh() {
        let spectator = SpectatorScene {
            entity_frame: app.entity_models.frame.clone(),
            entity_enabled: app.entity_models.enabled,
            entity_cache: app.entity_models.composite.clone(),
            surface: app
                .spectator_stream_view
                .as_ref()
                .cloned()
                .unwrap_or_else(|| std::sync::Arc::clone(view)),
            texture_bank: app.scene_texture_bank().cloned(),
            show_textures: app.scene_show_textures(),
            block_layers: app.scene_block_layers(),
            camera: app.spectator_camera,
            quality: app.gpu_quality,
            cache: std::sync::Arc::clone(&app.spectator_cache),
            presented: std::sync::Arc::clone(&app.spectator_presented),
            presented_view: std::sync::Arc::clone(&app.spectator_presented_view),
            pending: std::sync::Arc::clone(&app.spectator_pending),
            fallback: app.spectator_fallback.clone(),
            snapshot: app.spectator_stream_snapshot.clone(),
            chunk_cache: std::sync::Arc::clone(&app.spectator_chunk_cache),
            lighting: std::sync::Arc::clone(lighting),
        };
        if !app.spectator {
            let orthographic = app.world_orthographic_view();
            iced::widget::stack![
                iced::widget::shader(GpuScene(SceneSource::WorldOrthographic(
                    spectator,
                    orthographic.clone()
                )))
                .width(Length::Fill)
                .height(height),
                canvas(orthographic_world::Controls {
                    selection_handle: crate::selection_handle::config(app),
                    presented: std::sync::Arc::clone(&app.spectator_presented),
                    presented_view: std::sync::Arc::clone(&app.spectator_presented_view),
                    key: app.current_key(),
                    selected: app.selected_surface,
                    selection: app.pending_import.bounds().or_else(|| app.selection_box()),
                    show_frame: app.surface_frame,
                    show_axes: app.surface_axes,
                    show_chunk_grid: app.surface_chunk_grid,
                    show_section_guides: app.surface_section_guides,
                    brush: brush_stroke::config(app),
                })
                .width(Length::Fill)
                .height(height),
            ]
            .into()
        } else {
            iced::widget::stack![
                iced::widget::shader(GpuScene(SceneSource::Spectator(spectator)))
                    .width(Length::Fill)
                    .height(height),
                canvas(spectator::Controls {
                    workplane: work_plane::config(app),
                    selection_handle: crate::selection_handle::config(app),
                    presented: std::sync::Arc::clone(&app.spectator_presented),
                    presented_view: std::sync::Arc::clone(&app.spectator_presented_view),
                    key: app.current_key(),
                    selected: app.selected_surface,
                    selection: app.pending_import.bounds().or_else(|| app.selection_box()),
                    flying: has_flight_input(&app.spectator_keys),
                    brush: brush_stroke::config(app),
                })
                .width(Length::Fill)
                .height(height),
            ]
            .into()
        }
    } else if app.use_gpu {
        iced::widget::stack![
            iced::widget::shader(GpuScene(SceneSource::Isometric(surface_canvas.clone())))
                .width(Length::Fill)
                .height(height),
            canvas(surface_canvas).width(Length::Fill).height(height),
        ]
        .into()
    } else {
        iced::widget::stack![
            canvas(canvas_raster::ImageLayer(surface_canvas.clone()))
                .width(Length::Fill)
                .height(height),
            canvas(surface_canvas).width(Length::Fill).height(height),
        ]
        .into()
    };
    let stage = if !app.show_top_down && ((app.spectator && app.use_gpu) || app.uses_world_mesh()) {
        let presented = std::sync::Arc::clone(&app.spectator_presented);
        let frames = std::sync::Arc::clone(&app.spectator_presented_view);
        let projection_frames = std::sync::Arc::clone(&frames);
        let plane = work_plane::config(app);
        library::scene_target(
            app,
            app.active_view(),
            stage,
            move |point, size| {
                let mesh = presented.lock().expect("拖放显示网格锁被毒化").clone()?;
                let frame = frames.lock().expect("拖放实际投影锁被毒化").clone()?;
                if frame.viewport.logical != size || !std::sync::Arc::ptr_eq(&frame.mesh, &mesh) {
                    return None;
                }
                Some(brush_stroke::InputFrame::world_with_plane(
                    frame, point, plane,
                ))
            },
            move |size| {
                let frame = projection_frames
                    .lock()
                    .expect("放置实际投影锁被毒化")
                    .clone()?;
                (frame.viewport.logical == size)
                    .then_some(selection_handle::View::World(frame.camera, size))
            },
        )
    } else if !app.show_top_down {
        let projection = drop_surface.clone();
        library::scene_target(
            app,
            app.active_view(),
            stage,
            move |point, size| {
                let hit = drop_surface.pick_face(point, size);
                Some(brush_stroke::InputFrame::surface(
                    std::sync::Arc::clone(&drop_surface.surface),
                    drop_surface.navigation_target,
                    hit,
                ))
            },
            move |size| projection.selection_view(size),
        )
    } else {
        stage
    };
    let stage = brush_cursor::overlay(stage, brush_stroke::config(app), app.active_view(), height);
    let stage: Element<'_, Message> = if app.spectator && app.use_gpu {
        iced::widget::stack![
            stage,
            canvas(spectator::Crosshair {
                anchor: app.shortcuts.navigation.sticky.center()
            })
            .width(Length::Fill)
            .height(height)
        ]
        .into()
    } else {
        stage
    };
    let stage: Element<'_, Message> = if !app.show_top_down
        && (app.spectator || app.uses_world_mesh())
    {
        if let Some((_, visuals)) = app
            .scene_layers
            .command_visuals
            .as_ref()
            .filter(|(hash, _)| {
                app.scene_layers
                    .selected
                    .as_ref()
                    .is_some_and(|m| &m.marker().sha256 == hash)
            })
        {
            let projection =
                command_visuals::Projection(std::sync::Arc::clone(&app.spectator_presented_view));
            iced::widget::stack![
                stage,
                canvas(command_visuals::Overlay {
                    visuals: std::sync::Arc::clone(visuals),
                    projection
                })
                .width(Length::Fill)
                .height(height)
            ]
            .into()
        } else {
            stage
        }
    } else {
        stage
    };
    let stage = if !app.show_top_down {
        minimap::observe(
            app,
            app.active_view(),
            stage,
            annotation_projection.clone(),
            height,
        )
    } else {
        stage
    };
    let stage = if !app.show_top_down {
        chunk_outline::overlay(app, stage, annotation_projection.clone(), height)
    } else {
        stage
    };
    if !app.show_top_down && app.scene_layers.mask != 0 {
        if let Some(snapshot) = app
            .scene_layers
            .snapshot
            .as_ref()
            .filter(|s| Some(s.key) == app.current_key())
        {
            return view_compass::overlay(
                iced::widget::stack![
                    stage,
                    canvas(scene_layers::Overlay {
                        snapshot: std::sync::Arc::clone(snapshot),
                        projection: annotation_projection.clone(),
                        mask: app.scene_layers.mask,
                        inspect: app.scene_layers.inspect,
                        selected: app.scene_layers.selected.clone(),
                    })
                    .width(Length::Fill)
                    .height(height)
                ]
                .into(),
                annotation_projection,
                height,
            );
        }
    }
    if app.show_top_down {
        stage
    } else {
        view_compass::overlay(stage, annotation_projection, height)
    }
}
fn welcome_view(app: &App) -> Element<'_, Message> {
    let t = app.language;
    let can_open = !app.open_busy && !app.path.trim().is_empty();
    let intro = container(column![
        row![
            text("MCEdit2").size(32).width(Length::Fill),
            button(t.pick("English", "简体中文"))
                .style(button::text)
                .on_press(Message::ToggleLanguage)
        ].spacing(12).align_y(iced::Alignment::Center),
        text(t.pick("打开世界，开始编辑", "Open a world and start editing")).size(22),
        text(t.pick(
            "选择包含 level.dat 的 Java 版世界文件夹，也可以打开结构或 NBT 文件。",
            "Choose a Java Edition world folder containing level.dat, or open a structure or NBT file."
        )).size(14),
        row![
            button(t.pick("打开世界文件夹…", "Open world folder…"))
                .padding([10, 16])
                .on_press_maybe((!app.open_busy).then_some(Message::PickWorld)),
            button(t.pick("打开结构 / NBT…", "Open structure / NBT…"))
                .style(button::secondary)
                .padding([10, 16])
                .on_press(Message::FileInput(file_input::Action::Pick)),
            button(t.pick("浏览存档…", "Browse worlds…"))
                .style(button::secondary)
                .padding([10, 16])
                .on_press(Message::WorldBrowser(world_browser::Action::Show)),
        ].spacing(8).wrap(),
        row![
            text_input(
                t.pick("或粘贴世界文件夹路径", "Or paste a world folder path"),
                &app.path
            )
            .on_input(Message::Path)
            .on_submit_maybe(can_open.then(|| Message::Open(PathBuf::from(app.path.trim()), None)))
            .width(Length::Fill),
            button(t.pick("打开", "Open"))
                .style(button::secondary)
                .on_press_maybe(can_open.then(|| Message::Open(PathBuf::from(app.path.trim()), None))),
        ].spacing(8),
        text(if app.open_busy {
            t.pick("正在读取存档，请稍候…", "Reading the world, please wait…")
        } else {
            t.pick("修改先保留在当前文档，确认保存后才写入磁盘。", "Edits stay in the current document until you confirm Save.")
        }).size(13),
        row![
            button(t.pick("操作指南", "Quick start"))
                .style(button::text)
                .on_press(Message::About(about::Action::Guide)),
            button(t.pick("资料库", "Library"))
                .style(button::text)
                .on_press(Message::Library(library::Action::Show)),
            button(t.pick("日志与诊断", "Logs & diagnostics"))
                .style(button::text)
                .on_press(Message::LogView(log_view::Action::Toggle)),
        ].spacing(8).wrap(),
    ].spacing(16).width(Length::Fill))
        .padding(24)
        .style(container::rounded_box)
        .width(Length::Fill);
    let mut page = column![intro].spacing(10).width(Length::Fill);
    if !app.workspace.order.is_empty() {
        page = page.push(workspace_ui::header(app));
    }
    if app.structures.active.is_some() || app.current_key().is_some() {
        page = page.push(
            button(t.pick("返回当前文档", "Return to current document"))
                .on_press(Message::Desktop(desktop_state::Action::ShowRecent(false))),
        );
    }
    if app.world_browser.visible {
        page = page.push(world_browser::view(app));
    }
    page = page.push(desktop_state::recent_list(app));
    page = page.push(
        button(t.pick(
            if app.workspace.presentation.welcome_settings {
                "收起安装与偏好设置"
            } else {
                "安装与偏好设置…"
            },
            if app.workspace.presentation.welcome_settings {
                "Hide setup & preferences"
            } else {
                "Setup & preferences…"
            },
        ))
        .style(button::text)
        .on_press(Message::WorkspacePresentation(
            workspace_ui::presentation::Action::WelcomeSettings,
        )),
    );
    if app.workspace.presentation.welcome_settings {
        page = page.push(desktop_state::welcome_settings(app));
    }

    if app.library.welcome {
        page = page.push(
            if app
                .floating_panels
                .contains(floating_panels::Panel::Library)
            {
                floating_panels::placeholder(app, floating_panels::Panel::Library)
            } else {
                library::view(app)
            },
        );
    }
    if let Some(problem) = app.messages.last() {
        page = page.push(text(problem));
    }
    container(scrollable(
        container(container(page).max_width(1080).width(Length::Fill)).center_x(Length::Fill),
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(24)
    .into()
}
fn workspace_sidebar(app: &App) -> Element<'_, Message> {
    if let Some(panel) = floating_panels::Panel::from_pane(app.workspace_pane) {
        if app.floating_panels.contains(panel) {
            return floating_panels::placeholder(app, panel);
        }
    }
    workspace_sidebar_for(app, app.workspace_pane, true)
}
fn workspace_sidebar_for(app: &App, pane: workspace_ui::Pane, tabs: bool) -> Element<'_, Message> {
    let l = app.language;
    let scene_workspace = workspace_ui::scene_workspace(app);
    let mut panel = column![].spacing(10).width(Length::Fill);
    if scene_workspace {
        if let Some(session) = app.current_key().and_then(|key| app.sessions.get(&key)) {
            panel = panel.push(
                text(format!(
                    "{} {} · {} {} · {} {}",
                    l.pick("待保存方块", "Unsaved blocks"),
                    session.pending_cells(),
                    l.pick("可撤销", "Undo"),
                    session.undo_depth(),
                    l.pick("可重做", "Redo"),
                    session.redo_depth()
                ))
                .size(12),
            );
        }
        if !app.edit_report.is_empty() {
            panel = panel.push(text(&app.edit_report));
        }
        if app.block_edit.busy {
            panel = panel.push(text(block_edit::progress_text(app)));
        }
        if let Some(world) = app.current.and_then(|world| app.worlds.get(world)) {
            for warning in &world.warnings {
                panel = panel.push(text(warning).size(12));
            }
        }
        for message in &app.messages {
            panel = panel.push(text(message).size(12));
        }
    }
    if pane == workspace_ui::Pane::Library {
        panel = panel.push(library::view(app));
    }
    if pane == workspace_ui::Pane::History {
        panel = panel.push(history_ui::view(app));
    }
    if pane == workspace_ui::Pane::Analysis {
        panel = panel.push(analysis_ui::view(app));
    }
    if pane == workspace_ui::Pane::BlockDefinitions {
        panel = panel.push(block_definitions_ui::view(app));
    }
    if pane == workspace_ui::Pane::ChunkOperations {
        panel = panel.push(chunk_operations_ui::view(app));
    }
    if pane == workspace_ui::Pane::Edit {
        panel = panel.push(editor_options::content(app));
    }
    if pane == workspace_ui::Pane::Display && scene_workspace {
        panel = panel.push(text(app.advanced_section.label(l)).size(18));
        if app.advanced_section == AdvancedSection::Viewer {
            panel = panel
                .push(spectator::projection_controls(app))
                .push(work_plane::controls(app))
                .push(advanced_pages::viewer_controls(app, column![].spacing(8)));
        } else if app.advanced_section == AdvancedSection::Maps {
            panel = panel.push(advanced_pages::slice_controls(app));
        } else if app.advanced_section == AdvancedSection::Quad {
            panel = panel.push(quad_view::controls(app));
        }
        panel = panel
            .push(scene_layers::controls(app))
            .push(view_state::controls(app));
    }
    if pane == workspace_ui::Pane::Display && !scene_workspace {
        panel = panel.push(display_options::view(app));
    }
    if pane == workspace_ui::Pane::Resources {
        panel = panel.push(atlas_inspector::view(app));
        panel = panel.push(
            text(l.pick("资源包（按顺序叠加）", "Resource packs (ordered overlays)")).size(16),
        );
        panel = panel
            .push(
                text(l.pick(
                    "资源工作预算 MiB（留空自动，重新加载生效）",
                    "Resource budget MiB (blank = auto; reload to apply)",
                ))
                .size(12),
            )
            .push(text_input("自动 / Auto", &app.texture_budget).on_input(Message::TextureBudget))
            .push(
                iced::widget::checkbox(!app.texture_blocks_only)
                    .label(l.pick(
                        "加载全部 PNG（含物品与界面）",
                        "Load all PNGs (including items / GUI)",
                    ))
                    .on_toggle(Message::TextureAllPng),
            );
        panel = panel.push(
            button(l.pick("重新发现当前实例资源", "Rescan this instance's assets"))
                .on_press(Message::RefreshInstanceAssets),
        );
        if app.texture_loading {
            panel = panel.push(
                button(l.pick("取消后续资源加载", "Cancel pending asset loading"))
                    .on_press(Message::CancelTextureLoad),
            );
        }
        panel = panel.push(
            row![
                button(l.pick("打开 ZIP / JAR", "Open ZIP / JAR"))
                    .on_press(Message::ChooseTextureFile),
                button(l.pick("目录…", "Folder…")).on_press(Message::ChooseTextureDirectory)
            ]
            .spacing(5),
        );
        // The toggle reports user preference; the atlas might still be missing.
        // A Minecraft model cannot gain authentic textures from palette colors.
        if let Some(bank) = &app.texture_bank {
            panel = panel.push(
                text(format!(
                    "{}: {} PNG / {} {}",
                    l.pick("已加载材质", "Textures loaded"),
                    bank.statistics.decoded_png,
                    bank.model_count(),
                    l.pick("模型", "models"),
                ))
                .size(12),
            );
            let s = &bank.statistics;
            panel = panel.push(
                button(l.pick("展开/收起资源诊断", "Toggle asset diagnostics"))
                    .on_press(Message::TextureDiagnostics),
            );
            if app.texture_diagnostics {
                panel = panel.push(
                    scrollable(text(bank.warnings.join("\n")).size(12))
                        .height(Length::Fixed(200.0)),
                );
            }
            panel = panel.push(
                text(format!(
                    "{}: {} PNG / {} {}",
                    l.pick("3D 图集纹理", "3D atlas textures"),
                    s.atlas_png,
                    bank.atlas().map_or(0, |a| a.page_count()),
                    l.pick("页（自动扩展）", "pages (grow as needed)")
                ))
                .size(12),
            );
            panel = panel
                .push(
                    text(format!(
                        "{}: {}\n{}: {}\n{}: {}；{}: {}",
                        l.pick("PNG 索引", "PNG indexed"),
                        s.indexed_png,
                        l.pick("本次请求", "PNG requested"),
                        s.requested_png,
                        l.pick("成功解码", "PNG decoded"),
                        s.decoded_png,
                        l.pick("PNG 失败", "PNG errors"),
                        s.failed_png
                    ))
                    .size(12),
                )
                .push(
                    text(format!(
                        "{}: {}；{}: {} / {}",
                        l.pick("缺失引用", "Missing references"),
                        s.missing_references.len(),
                        l.pick("模型索引/失败", "Models indexed/errors"),
                        s.indexed_models,
                        s.failed_models
                    ))
                    .size(12),
                )
                .push(
                    text(format!(
                        "RGBA {} MiB / {} {} MiB / {} {} MiB",
                        s.decoded_bytes / (1024 * 1024),
                        l.pick("图集", "Atlas"),
                        s.atlas_bytes / (1024 * 1024),
                        l.pick("工作预算", "Budget"),
                        s.budget_bytes / (1024 * 1024)
                    ))
                    .size(12),
                );
        } else {
            panel = panel.push(text(if app.texture_loading {
            l.pick("正在加载资源包材质…", "Loading resource-pack textures…")
        } else if app.show_textures {
            l.pick("未加载材质！当前是纯色预览。请选择原版客户端 JAR 或资源包 ZIP。",
                "No textures loaded: palette-only preview. Select a client JAR or resource-pack ZIP.")
        } else {
            l.pick("纹理已禁用，显示纯色预览。", "Textures disabled: palette preview.")
        }).size(12));
        }
        if !app.texture_report.is_empty() {
            panel = panel.push(
                text(&app.texture_report)
                    .size(12)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            );
        }
        if !app.texture_warning.is_empty() {
            panel = panel.push(
                text(&app.texture_warning)
                    .size(12)
                    .width(Length::Fill)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph),
            );
        }
    }
    if pane == workspace_ui::Pane::Inspect {
        panel = panel.push(object_inspector::view(app).map(Message::ObjectInspector));
        for (label, section) in [
            (
                l.pick("实体与方块实体工具", "Entity tools"),
                AdvancedSection::BlockEntities,
            ),
            (l.pick("玩家", "Players"), AdvancedSection::Players),
            (
                l.pick("世界属性", "World properties"),
                AdvancedSection::WorldInfo,
            ),
            (l.pick("地图物品", "Map items"), AdvancedSection::MapsEditor),
        ] {
            panel = panel.push(button(label).on_press(Message::AdvancedSection(section)));
        }
    }
    if scene_workspace
        && matches!(
            app.advanced_section,
            AdvancedSection::Maps | AdvancedSection::Quad
        )
    {
        use view_navigation::{QuadPane, Target};
        let views: &[(Target, &str, &str)] = if app.advanced_section == AdvancedSection::Quad {
            &[
                (Target::Quad(QuadPane::Horizontal), "水平", "Horizontal"),
                (Target::Quad(QuadPane::Front), "正面", "Front"),
                (Target::Quad(QuadPane::Side), "侧面", "Side"),
                (Target::Quad(QuadPane::Isometric), "等轴", "Isometric"),
            ]
        } else {
            &[
                (Target::Horizontal, "水平切片", "Horizontal slice"),
                (Target::Vertical, "纵向切片", "Vertical slice"),
            ]
        };
        for &(target, zh, en) in views {
            panel = panel.push(
                text(format!(
                    "{} · {}",
                    l.pick(zh, en),
                    navigation_bindings::hint(app, target)
                ))
                .size(12),
            );
        }
    } else if pane != workspace_ui::Pane::Display {
        panel = panel.push(text(if app.spectator && !app.show_top_down {
            format!(
                "{} · {} · {}",
                keyboard_bindings::flight_hint(app),
                navigation_bindings::hint(app, app.active_view()),
                l.pick(
                    "左键点选/拖动框选 · Alt 点选角点 A · Ctrl 点选角点 B",
                    "Left click/drag select · Alt-click A · Ctrl-click B"
                )
            )
        } else {
            format!(
                "{} · {}",
                l.pick("左键选块", "Left click: pick"),
                navigation_bindings::hint(app, app.active_view())
            )
        }));
    }
    let scroll = if !tabs {
        if pane == workspace_ui::Pane::Inspect {
            "mcedit2-floating-inspector"
        } else {
            "mcedit2-floating-options"
        }
    } else {
        workspace_ui::options_scroll(app)
    };
    let navigation: Element<'_, Message> = if scene_workspace {
        button(l.pick("更多工具与文件…", "More tools & files…"))
            .on_press(Message::AdvancedSection(AdvancedSection::Overview))
            .into()
    } else {
        column![].into()
    };
    let sidebar = column![
        if tabs { navigation } else { column![].into() },
        if tabs {
            workspace_ui::pane_tabs(app)
        } else {
            column![].into()
        },
        editor_options::active(app),
        if pane == workspace_ui::Pane::Edit {
            editor_options::tabs(app)
        } else {
            column![].into()
        },
        if tabs {
            minimap::view(app)
        } else {
            column![].into()
        },
        iced::widget::keyed_column([(
            (pane, app.editor_tool),
            scrollable(panel)
                .id(iced::advanced::widget::Id::new(scroll))
                .direction(iced::widget::scrollable::Direction::Vertical(
                    iced::widget::scrollable::Scrollbar::new().spacing(8)
                ))
                .height(Length::Fill)
                .into()
        )])
        .height(Length::Fill)
    ]
    .spacing(8)
    .width(Length::Fill)
    .height(Length::Fill);
    sidebar.into()
}
fn workspace_view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let heading = workspace_ui::header(app);
    let sidebar = workspace_sidebar(app);
    let viewport: Element<'_, Message> = match &app.surface {
        Some((wi, di, scene)) if app.current_key() == Some((*wi, *di)) => {
            render_surface(app, scene, Length::Fill)
        }
        _ => container(text(if app.surface_report.is_empty() {
            l.pick(
                "正在读取存档并创建 3D 地形…",
                "Reading original chunks and preparing 3D terrain…",
            )
        } else {
            &app.surface_report
        }))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into(),
    };
    let status = workspace_ui::presentation::scene_status(
        app,
        &[&app.surface_report],
        "mcedit2-world-status",
    );
    let content = column![
        container(viewport).width(Length::Fill).height(Length::Fill),
        status
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .spacing(10);
    container(
        column![
            heading,
            workspace_ui::layout::split(
                app,
                workspace_ui::layout::Kind::Editor,
                sidebar.into(),
                content.into()
            ),
            editor_options::pending(app)
        ]
        .spacing(12),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(12)
    .into()
}
fn view(app: &App) -> Element<'_, Message> {
    drag_drop::maintain(app);
    let content = about::modal(
        app,
        keyboard_bindings::modal(
            app,
            workspace_tabs::modal(
                app,
                block_picker::modal(
                    app,
                    log_view::shell(
                        app,
                        startup_script_ui::shell(
                            app,
                            file_input::shell(
                                app,
                                drag_drop::wrap(&app.drag_drop, app.main_window, view_content(app)),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    );
    world_save::modal(app, content)
}
fn window_view(app: &App, id: iced::window::Id) -> Element<'_, Message> {
    if app.main_window == Some(id) {
        view(app)
    } else {
        floating_panels::view(app, id)
    }
}
fn view_content(app: &App) -> Element<'_, Message> {
    let _native_span = iced_wgpu::window::frame_statistics::span("app_view");
    let mut page = column![workspace_ui::menu::bar(app), view_page(app)];
    if let Some(rates) = runtime_rates::view(app) {
        page = page.push(container(rates).padding([2, 12]).width(Length::Fill));
    }
    page.width(Length::Fill).height(Length::Fill).into()
}
fn view_page(app: &App) -> Element<'_, Message> {
    if let Some(id) = structure_workspace::active(app) {
        return structure_workspace::view(app, id);
    }
    if app.current_key().is_none() || app.desktop.show_recent {
        return welcome_view(app);
    }
    if !app.show_advanced {
        return workspace_view(app);
    }
    advanced_view(app)
}
fn advanced_view(app: &App) -> Element<'_, Message> {
    let l = app.language;
    if app.advanced_section == AdvancedSection::Quad {
        return workspace_ui::advanced_shell(app, quad_view::view(app));
    }
    if app.advanced_section == AdvancedSection::Viewer {
        return workspace_ui::advanced_shell(app, advanced_pages::viewer(app));
    }
    if app.advanced_section == AdvancedSection::Maps {
        return workspace_ui::advanced_shell(app, advanced_pages::slices(app));
    }
    let mut body = column![text(app.advanced_section.label(l)).size(24),]
        .spacing(14)
        .width(Length::Fill);
    if app.advanced_section == AdvancedSection::Search {
        return workspace_ui::advanced_shell(app, scrollable(nbt_search::view(app)).into());
    }
    if app.advanced_section == AdvancedSection::BlockDefinitions {
        return workspace_ui::advanced_shell(
            app,
            scrollable(block_definitions_ui::view(app))
                .id(workspace_ui::DOCUMENT_SCROLL)
                .into(),
        );
    }
    if app.advanced_section == AdvancedSection::Analysis {
        return workspace_ui::advanced_shell(
            app,
            scrollable(analysis_ui::view(app))
                .id(workspace_ui::DOCUMENT_SCROLL)
                .into(),
        );
    }
    if app.advanced_section == AdvancedSection::Replacement {
        return workspace_ui::advanced_shell(
            app,
            scrollable(block_replacement_ui::view(app))
                .id(workspace_ui::DOCUMENT_SCROLL)
                .into(),
        );
    }
    if app.advanced_section == AdvancedSection::ChunkOperations {
        return workspace_ui::advanced_shell(
            app,
            scrollable(chunk_operations_ui::view(app))
                .id(workspace_ui::DOCUMENT_SCROLL)
                .into(),
        );
    }
    if let Some(world) = app.current.and_then(|i| app.worlds.get(i)) {
        if matches!(
            app.advanced_section,
            AdvancedSection::Files
                | AdvancedSection::Players
                | AdvancedSection::WorldInfo
                | AdvancedSection::BlockEntities
        ) {
            return workspace_ui::advanced_shell(app, file_ui::view(app, world));
        }
        if app.advanced_section == AdvancedSection::Overview {
            body = advanced_pages::overview(app, world, body);
        }
        if app.advanced_section == AdvancedSection::Editing {
            body = advanced_pages::editing(app, world, body);
        }
        if matches!(
            app.advanced_section,
            AdvancedSection::Editing | AdvancedSection::Structures | AdvancedSection::Plugins
        ) {
            body = body.push(block_edit::budget_view(app));
        }
        if matches!(
            app.advanced_section,
            AdvancedSection::MapsEditor
                | AdvancedSection::Structures
                | AdvancedSection::Plugins
                | AdvancedSection::Save
                | AdvancedSection::Chunks
        ) {
            body = body.push(file_ui::view(app, world));
        }
        let stats = app.current_key().and_then(|key| app.sessions.get(&key));
        if let Some(session) = stats {
            body = body.push(text(format!(
                "{} {} · {} {} · {} {}",
                l.pick("待保存方块", "Unsaved blocks"),
                session.pending_cells(),
                l.pick("可撤销", "Undo"),
                session.undo_depth(),
                l.pick("可重做", "Redo"),
                session.redo_depth()
            )));
        }
        if !app.edit_report.is_empty() {
            body = body.push(text(&app.edit_report));
            if app.block_edit.busy {
                body = body.push(text(block_edit::progress_text(app)));
            }
        }
        for warning in &world.warnings {
            body = body.push(text(format!("Warning: {warning}")));
        }
    }
    for message in &app.messages {
        body = body.push(text(message));
    }
    let page = iced::widget::keyed_column([(
        app.advanced_section,
        scrollable(body)
            .id(workspace_ui::TOOLS_SCROLL)
            .direction(iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::new().spacing(8),
            ))
            .height(Length::Fill)
            .width(Length::Fill)
            .into(),
    )])
    .height(Length::Fill)
    .width(Length::Fill);
    workspace_ui::advanced_shell(app, page.into())
}

fn has_flight_input(keys: &std::collections::BTreeSet<char>) -> bool {
    ['w', 'a', 's', 'd', 'q', ' ']
        .iter()
        .any(|key| keys.contains(key))
}
fn subscription(app: &App) -> iced::Subscription<Message> {
    let mut events = vec![
        iced::window::close_requests().map(Message::CloseRequested),
        iced::event::listen_with(workspace_tabs::input),
        iced::event::listen_with(desktop_state::input),
        iced::event::listen_with(file_input::input),
        render_wake::subscription().map(|_| Message::WorldMeshPresented),
    ];
    if let Some(profile) = &app.native_profile {
        events.push(profile.subscription());
    }
    if let Some(server) = &app.single_instance {
        events.push(server.subscription());
    }
    if let Some(rates) = runtime_rates::subscription(app) {
        events.push(rates);
    }
    if app.about.open {
        events.push(iced::event::listen_with(about::keyboard));
    }
    if let Some(logs) = log_view::subscription(app) {
        events.push(logs);
    }
    if let Some(reload) = plugin_ui::reload::subscription(app) {
        events.push(reload);
    }
    // 动画仅由可见 GPU / Canvas 控件按资源时序安排下一次重绘，不轮询世界网格。
    // GPU / Canvas 控件在等待后台缓存时安排重绘，无需独立的 8Hz 网格轮询。
    if keyboard_bindings::flight_active(app) {
        if has_flight_input(&app.spectator_keys) {
            events.push(iced::window::frames().map(Message::SpectatorTick));
        }
    }
    iced::Subscription::batch(events)
}
fn main() -> std::process::ExitCode {
    entry::run()
}
fn run_gui(args: launch::Arguments) -> Result<(), String> {
    log::debug!(
        "原生应用状态 {} 字节，消息 {} 字节",
        std::mem::size_of::<App>(),
        std::mem::size_of::<Message>()
    );
    mcedit2_world::diagnostics::set_error_waker(render_wake::notify)?;
    crash_log::install();
    let profile = native_profile::Config::load(&args)?;
    let instance = if args.view {
        None
    } else {
        let paths = if args.eval.is_some() || args.reset_prefs {
            &[][..]
        } else {
            args.worlds.as_slice()
        };
        match single_instance::claim(paths)
            .map_err(|error| format!("初始化主窗口端点失败：{error}"))?
        {
            single_instance::Claim::Primary(guard) => Some(guard),
            single_instance::Claim::Existing => {
                log::info!("主窗口已在运行，已发送 {} 个文件并请求唤起", paths.len());
                if args.eval.is_some() || args.reset_prefs {
                    return Err("主窗口已在运行；此次 -eval 或 -resetPrefs 参数及所附文件未执行。请先关闭主窗口后再使用这些启动参数；-view 可打开独立只读窗口。".into());
                }
                return Ok(());
            }
        }
    };
    mesh_async::initialize()?;
    if args.reset_prefs {
        launch::reset_rust_prefs().map_err(|error| format!("重置 Rust 配置失败：{error}"))?;
    }
    let profile_scale = profile.as_ref().map_or(1., |p| p.scale);
    let outcome = profile
        .as_ref()
        .map(|config| std::sync::Arc::clone(&config.outcome));
    let (mut app, task) = App::boot(args, profile.is_none())?;
    if let Some(radius) = profile.as_ref().and_then(|profile| profile.radius) {
        app.spectator_radius = radius;
    }
    log::debug!("原生初始状态已完成读取");
    app.single_instance = instance.as_ref().map(single_instance::Guard::server);
    let script_outcome = app.startup_script.outcome.clone();
    let mut window = if let Some(profile) = &profile {
        iced::window::Settings {
            size: profile.window_size,
            exit_on_close_request: false,
            ..Default::default()
        }
    } else {
        app.desktop.saved.window()
    };
    window.icon = Some(about::window_icon());
    app.native_profile = profile.map(native_profile::State::new);
    app.drag_drop.set_application_scale(profile_scale);
    // iced 的启动接口为 Fn；运行器只创建一次主状态，后台线程也只启动一次。
    let (primary, opened) = iced::window::open(window);
    app.main_window = Some(primary);
    let task = iced::Task::batch([
        task,
        opened.map(|id| Message::FloatingPanel(floating_panels::Action::PrimaryOpened(id))),
    ]);
    // 运行器的启动与事件循环按值交接状态；大型编辑器状态留在堆上，避免多层栈副本。
    let initial = std::cell::RefCell::new(Some((Box::new(app), task)));
    fn boxed_view(app: &Box<App>, id: iced::window::Id) -> iced::Element<'_, Message> {
        window_view(app.as_ref(), id)
    }
    let result = iced::daemon(
        move || initial.borrow_mut().take().expect("主窗口只能初始化一次"),
        |app: &mut Box<App>, message| update(app.as_mut(), message),
        boxed_view,
    )
    .scale_factor(move |_, _| profile_scale)
    .subscription(|app: &Box<App>| subscription(app.as_ref()))
    .title(|app: &Box<App>, id| floating_panels::title(app.as_ref(), id))
    .theme(Theme::Dark)
    .run();
    if let Err(error) = &result {
        crash_log::failure(error);
    }
    let closed = instance
        .map(single_instance::Guard::close)
        .transpose()
        .map_err(|error| {
            let error = format!("释放主窗口端点失败：{error}");
            log::error!("[single-instance] {error}");
            error
        });
    result.map_err(|e| e.to_string())?;
    closed?;
    if outcome.is_some_and(|state| state.load(std::sync::atomic::Ordering::Acquire) != 1) {
        return Err("原生回放没有完成".into());
    }
    if script_outcome.is_some_and(|state| state.load(std::sync::atomic::Ordering::Acquire) != 1) {
        return Err("启动脚本未成功完成；详情见启动脚本状态和运行日志".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    fn asset_progress_base_publication_and_cancellation_obey_the_request_nonce() {
        let mut app = super::App {
            texture_load_nonce: 4,
            texture_loading: true,
            ..Default::default()
        };
        let _ = super::update_impl(
            &mut app,
            super::Message::TextureProgress(3, "旧进度".into()),
        );
        assert!(app.texture_report.is_empty());
        let _ = super::update_impl(
            &mut app,
            super::Message::TextureBaseLoaded(4, None, crate::textures::TextureBank::default()),
        );
        assert!(app.texture_bank.is_some());
        assert!(app.texture_loading);
        let _ = super::update_impl(
            &mut app,
            super::Message::TextureProgress(4, "解码材质 10/20".into()),
        );
        assert_eq!(app.texture_report, "解码材质 10/20");
        let _ = super::update_impl(
            &mut app,
            super::Message::TextureWarning(4, "版本不匹配".into()),
        );
        let token = std::sync::Arc::clone(&app.texture_cancelled);
        let _ = super::update_impl(&mut app, super::Message::CancelTextureLoad);
        assert!(token.load(std::sync::atomic::Ordering::Acquire));
        assert!(!app.texture_loading);
        assert!(app.texture_bank.is_some());
        assert_eq!(app.texture_warning, "版本不匹配");
        let _ = super::update_impl(
            &mut app,
            super::Message::TextureProgress(4, "不应恢复旧任务".into()),
        );
        assert_eq!(app.texture_report, "已取消后续材质加载。");
        // 同一世界重新打开时，也必须按真实数据包来源撤销旧材质与在途请求。
        app.worlds.push(mcedit2_world::WorldSummary {
            root: std::path::PathBuf::from("world"),
            world_data: Default::default(),
            seed: None,
            level_name: None,
            data_version: Some(3955),
            storage_format: Some(19133),
            version_name: Some("1.21.1".into()),
            dimensions: vec![],
            warnings: vec![],
        });
        app.current = Some(0);
        let mut changed = app.worlds[0].clone();
        std::sync::Arc::make_mut(&mut changed.world_data).origin = "level:changed".into();
        let nonce = app.texture_load_nonce;
        app.reset_assets_for_world(&changed);
        assert!(app.texture_bank.is_none());
        assert_ne!(app.texture_load_nonce, nonce);
    }

    fn near_refresh_retains_completed_view_and_rejects_other_worlds() {
        use std::sync::Arc;
        let mut app = super::App {
            current: Some(0),
            ..super::App::default()
        };
        let source = Arc::new(mcedit2_world::surface::SurfaceView {
            biomes: Default::default(),
            x: 16,
            z: -16,
            y_min: 0,
            y_max: 47,
            width: 16,
            depth: 16,
            voxels: vec![],
            source_tiles: vec![],
            source_chunks: 0,
            missing_columns: 0,
        });
        app.grid_main.cancel();
        app.grid_horizontal.cancel();
        app.grid_vertical.cancel();
        app.grid_revision = app.grid_revision.wrapping_add(1);
        app.surface = Some((0, 0, Arc::clone(&source)));
        assert!(app.visible_scene_is_current());
        // An attempted reload can fail without discarding the last valid view.
        app.inspect_surface(true);
        assert!(app
            .surface
            .as_ref()
            .is_some_and(|(_, _, view)| Arc::ptr_eq(view, &source)));
        app.current = Some(1);
        assert!(!app.visible_scene_is_current());
        app.inspect_surface(true);
        assert!(app.surface.is_none());
    }

    fn read_only_refresh_keeps_spectator_mesh_until_new_source_is_ready() {
        let mut before = super::App::default().render_signature();
        before.source = Some((0, 0, 3, 10, 5, 2));
        let mut after = super::App::default().render_signature();
        after.source = Some((0, 0, 4, 12, 7, 3));
        assert!(!super::must_reset_spectator_mesh(&before, &after, true));
        assert!(super::must_reset_spectator_mesh(&before, &after, false));
        after.geometry = Some(super::textures::TextureBank::default().visibility_id());
        assert!(super::must_reset_spectator_mesh(&before, &after, true));
        after.geometry = before.geometry.clone();
        after.show_textures = !before.show_textures;
        assert!(super::must_reset_spectator_mesh(&before, &after, true));
    }
    use super::{
        parse_region_name, parse_scene_window, shifted_scene, shifted_slice, slice_cell_label,
    };
    use mcedit2_world::editing::{Bounds, SliceCell};
    use mcedit2_world::sections::State;

    fn superseded_indexer_and_near_read_cannot_publish_stale_results() {
        let mut app = super::App {
            open_nonce: 8,
            open_busy: true,
            ..super::App::default()
        };
        let before = app.messages.len();
        let _ = super::update_impl(
            &mut app,
            super::Message::OpenFinished(7, Err("a stale bad world".into())),
        );
        assert!(!app.open_busy);
        assert_eq!(app.messages.len(), before);
        app.near_nonce = 12;
        app.near_busy = true;
        app.near_loaded(11, (0, 0), super::NearResult::WorkerStopped);
        assert!(!app.near_busy);
        assert!(app.surface_report.is_empty());
        assert!(app.slice_grid.is_none());
    }

    fn opened_world_immediately_schedules_source_3d_and_hides_advanced_panel() {
        let mut app = super::App {
            show_advanced: true,
            ..super::App::default()
        };
        app.worlds.push(mcedit2_world::WorldSummary {
            world_data: Default::default(),
            seed: Default::default(),
            root: std::path::PathBuf::from("."),
            level_name: Some("Original world".into()),
            data_version: Some(3955),
            storage_format: Some(19133),
            version_name: Some("1.21.1".into()),
            dimensions: vec![mcedit2_world::DimensionSummary {
                name: "minecraft:overworld".into(),
                region_dir: std::path::PathBuf::from("./region"),
                regions: 0,
                chunks: 0,
                extension: "mca".into(),
            }],
            warnings: vec![],
        });
        app.current = Some(0);
        app.open_default_scene(85);
        assert!(!app.show_advanced);
        assert!(!app.show_top_down);
        assert!(app.spectator);
        assert!(app.surface_exposed);
        assert!(app.surface.is_none());
        assert!(app.near_job.is_some());
        assert_eq!(app.surface_area, "-16,64,-16;15,111,15");
        assert!(app.surface_report.contains("Loading"));
        let before = app.render_signature();
        let _ = super::update_impl(&mut app, super::Message::ToggleLanguage);
        assert!(app.render_signature() == before);
    }

    fn rapid_open_requests_keep_only_latest_source_path() {
        let mut app = super::App {
            path: "original-A".into(),
            ..super::App::default()
        };
        let _ = super::update_impl(&mut app, super::Message::Open("original-A".into(), None));
        let old = app.open_nonce;
        app.path = "original-B".into();
        let _ = super::update_impl(&mut app, super::Message::Open("original-B".into(), None));
        assert_eq!(
            app.open_pending.as_ref().unwrap().1,
            std::path::PathBuf::from("original-B")
        );
        assert_ne!(app.open_nonce, old);
        let _ = super::update_impl(&mut app, super::Message::WorkspaceIdle);
        assert!(app.open_pending.is_none());
    }

    fn far_tile_cache_is_bounded_and_lod_sensitive() {
        use super::{far_cache_key, FAR_CACHE_PATCHES, FAR_CACHE_VOXELS};
        use mcedit2_world::far_view::{FarChunkPatch, FarChunkRequest};
        use mcedit2_world::surface::SourceChunkTile;
        let sample = FarChunkRequest {
            cx: 12,
            cz: -8,
            span: 8,
        };
        let tile = SourceChunkTile {
            chunk_x: 12,
            chunk_z: -8,
            source_present: false,
            known_columns: 0,
            unknown_columns: 4,
            exposed_voxels: 0,
        };
        let patch = std::sync::Arc::new(FarChunkPatch {
            biome_neighbors: Default::default(),
            biomes: Default::default(),
            sections: None,
            tile,
            voxels: vec![],
        });
        assert_ne!(
            far_cache_key(sample),
            far_cache_key(FarChunkRequest { span: 16, ..sample })
        );
        const {
            assert!(FAR_CACHE_PATCHES <= 4096);
        }
        const {
            assert!(FAR_CACHE_VOXELS <= 90_000);
        }
        assert_eq!(patch.voxels.len(), 0);
    }

    fn render_signature_ignores_animation_and_hover_but_tracks_camera_changes() {
        let mut app = super::App::default();
        let original = app.render_signature();
        app.surface_hover = Some((4, 64, 6));
        assert!(original == app.render_signature());
        app.surface_yaw = 45;
        assert!(original != app.render_signature());
    }

    fn flight_frame_clock_starts_and_stops_with_movement() {
        use iced::keyboard::{Event, Key, Modifiers};
        let (mut app, _, _, _) = crate::picking_integration_tests::fixture();
        app.desktop.show_recent = false;
        let key = Key::Character("w".into());
        let _ = super::update(
            &mut app,
            super::Message::SpectatorKey(Event::KeyPressed {
                key: key.clone(),
                modified_key: key.clone(),
                physical_key: iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::KeyW),
                location: iced::keyboard::Location::Standard,
                modifiers: Modifiers::default(),
                text: None,
                repeat: false,
            }),
        );
        let start = app.spectator_last_frame.unwrap();
        let _ = super::update(
            &mut app,
            super::Message::SpectatorTick(start + std::time::Duration::from_millis(16)),
        );
        let _ = super::update(&mut app, super::Message::SpectatorFocusLost);
        assert!(app.spectator_last_frame.is_none());
    }

    fn viewport_3d_navigation_is_independent_of_block_edit_budget() {
        let a = parse_scene_window("0,-64,0;15,-33,15").unwrap();
        assert_eq!(a.volume().unwrap(), 8192);
        assert_eq!(
            shifted_scene("0,-64,0;15,-33,15", -16, 0, -16).unwrap(),
            "-16,-64,-16;-1,-33,-1"
        );
        assert_eq!(
            parse_scene_window("0,-64,0;63,-17,63")
                .unwrap()
                .volume()
                .unwrap(),
            196_608
        );
        assert!(parse_scene_window("0,-64,0;64,-33,15").is_err());
        assert!(parse_scene_window("0,-64,0;15,-16,15").is_err());
        assert!(shifted_scene("29999980,0,0;29999995,15,15", 16, 0, 0).is_err());
        assert_eq!(
            shifted_scene("-32,-64,-32;-1,-17,-1", 16, 0, 16).unwrap(),
            "-16,-64,-16;15,-17,15"
        );
    }

    fn centering_in_spectator_moves_camera_and_keeps_current_orientation() {
        let mut app = super::App {
            spectator: true,
            query_coordinates: "43,125,-66".into(),
            spectator_camera: super::Camera {
                position: [0.0, 211.0, 0.0],
                yaw: 339.0,
                pitch: 29.0,
                zoom: 1.0,
                perspective: true,
            },
            ..Default::default()
        };
        app.center_3d();
        assert_eq!(app.spectator_camera.position, [43.5, 125.5, -65.5]);
        assert_eq!(
            (app.spectator_camera.yaw, app.spectator_camera.pitch),
            (339.0, 29.0)
        );
        app.query_coordinates = "30000000,64,0".into();
        app.center_3d();
        assert_eq!(app.spectator_camera.position, [43.5, 125.5, -65.5]);
    }

    fn interactive_slice_shift_and_cell_symbols_reject_overflow() {
        let bounds = Bounds::new((-16, -64, 0), (-1, -64, 15)).unwrap();
        assert_eq!(
            shifted_slice(bounds, 16, 1, -16).unwrap(),
            "0,-63,-16;15,-63,-1"
        );
        let edge = Bounds::new((29_999_999, 320, 0), (29_999_999, 320, 0)).unwrap();
        assert!(shifted_slice(edge, 16, 0, 0).is_err());
        assert_eq!(slice_cell_label(&SliceCell::MissingChunk), "·");
        assert_eq!(slice_cell_label(&SliceCell::MissingSection), "?");
        assert_eq!(
            slice_cell_label(&SliceCell::Present {
                state: State {
                    name: "minecraft:oak_log".into(),
                    properties: vec![]
                },
                original_block_entity: false,
                staged_creation: false,
                light: Default::default(),
            }),
            "O"
        );
        assert_eq!(
            slice_cell_label(&SliceCell::Present {
                state: State {
                    name: "mod:machine".into(),
                    properties: vec![]
                },
                original_block_entity: true,
                staged_creation: false,
                light: Default::default(),
            }),
            "@"
        );
    }

    fn original_region_name_parser_accepts_signed_coords_and_refuses_extra_paths() {
        assert_eq!(parse_region_name("r.-1.2.mca").unwrap(), (-1, 2, "mca"));
        assert_eq!(parse_region_name("r.0.-9.mcr").unwrap(), (0, -9, "mcr"));
        for value in [
            "../r.0.0.mca",
            "r.0.0.mca.tmp",
            "r.a.0.mca",
            "r.0.0.mcc",
            "r.0.0.mca/evil",
            "r.0.0.mca\\other",
            "r.0.0",
            "r.2147483648.0.mca",
        ] {
            assert!(parse_region_name(value).is_err(), "{value}");
        }
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Background {
            asset_progress_base_publication_and_cancellation_obey_the_request_nonce();
            near_refresh_retains_completed_view_and_rejects_other_worlds();
            read_only_refresh_keeps_spectator_mesh_until_new_source_is_ready();
            superseded_indexer_and_near_read_cannot_publish_stale_results();
            rapid_open_requests_keep_only_latest_source_path();
        }

        if category == crate::test_categories::Category::Navigation {
            opened_world_immediately_schedules_source_3d_and_hides_advanced_panel();
            far_tile_cache_is_bounded_and_lod_sensitive();
            render_signature_ignores_animation_and_hover_but_tracks_camera_changes();
            flight_frame_clock_starts_and_stops_with_movement();
            viewport_3d_navigation_is_independent_of_block_edit_budget();
            centering_in_spectator_moves_camera_and_keeps_current_orientation();
            interactive_slice_shift_and_cell_symbols_reject_overflow();
            original_region_name_parser_accepts_signed_coords_and_refuses_extra_paths();
        }
    }
}

#[cfg(test)]
mod test_categories;
