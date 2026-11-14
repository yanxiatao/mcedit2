//! 存档 / 维度的只读观察状态与书签；与笔刷参数共用单个配置写入队列。
use crate::{spectator::Camera, App, Bounds, Message, NearMode};
use iced::{
    widget::{button, column, row, text, text_input},
    Element, Task,
};
use mcedit2_world::{
    launch,
    player_catalog::{Location, Locations},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{mpsc, Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Viewport {
    zoom: f32,
    pan: [f32; 2],
}
impl From<crate::view_navigation::Viewport> for Viewport {
    fn from(v: crate::view_navigation::Viewport) -> Self {
        Self {
            zoom: v.zoom,
            pan: [v.pan.x, v.pan.y],
        }
    }
}
impl From<Viewport> for crate::view_navigation::Viewport {
    fn from(v: Viewport) -> Self {
        Self {
            zoom: v.zoom,
            pan: iced::Vector::new(v.pan[0], v.pan[1]),
        }
    }
}
impl Viewport {
    fn validate(self) -> Result<(), String> {
        crate::view_navigation::zoom_after(self.zoom, 0.)?;
        if !self.pan.into_iter().all(f32::is_finite) {
            return Err("保存的平移坐标无效".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Quad {
    pub(crate) center: Option<[i32; 3]>,
    pub(crate) yaw: i32,
    pub(crate) pitch: i32,
    pub(crate) navigation: [Viewport; 4],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    bounds: [[i32; 3]; 2],
    volume: bool,
    cut: Option<i32>,
    fixed: bool,
}
impl Source {
    fn bounds(&self) -> Result<Bounds, String> {
        let [a, b] = self.bounds;
        let bounds = crate::parse_scene_window(&format!(
            "{},{},{};{},{},{}",
            a[0], a[1], a[2], b[0], b[1], b[2]
        ))?;
        if bounds.minimum != a
            || bounds.maximum != b
            || self
                .cut
                .is_some_and(|y| !self.volume || y < a[1] || y > b[1])
        {
            return Err("保存的观察窗口边界或剖切高度无效".into());
        }
        Ok(bounds)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct View {
    camera: Camera,
    spectator: bool,
    top_down: bool,
    angles: [i32; 3],
    navigation: [Viewport; 4],
    framing: crate::orthographic_world::SavedFraming,
    source: Source,
    horizontal: Option<[[i32; 3]; 2]>,
    vertical: Option<[[i32; 3]; 2]>,
    quad: Option<Quad>,
    radius: u16,
    guides: [bool; 4],
    #[serde(default)]
    workplane: crate::work_plane::Saved,
    #[serde(default)]
    lighting: crate::lighting::Settings,
}
fn position(position: [f64; 3]) -> Result<(), String> {
    if !position.into_iter().all(f64::is_finite)
        || !(-30_000_000. ..30_000_000.).contains(&position[0])
        || !(-30_000_000. ..30_000_000.).contains(&position[2])
        || !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&position[1])
    {
        return Err("观察位置超出受支持的源坐标范围".into());
    }
    Ok(())
}
fn point_bounds(bounds: [[i32; 3]; 2]) -> Result<(), String> {
    for p in bounds {
        Bounds::new((p[0], p[1], p[2]), (p[0], p[1], p[2])).map_err(|e| e.to_string())?;
    }
    if !(0..3).all(|i| bounds[0][i] <= bounds[1][i]) {
        return Err("保存的切面边界倒置".into());
    }
    Ok(())
}
impl View {
    fn validate(&self) -> Result<(), String> {
        position(self.camera.position)?;
        self.workplane.validate()?;
        self.lighting.validate()?;
        if !self.camera.yaw.is_finite()
            || !self.camera.pitch.is_finite()
            || !(-89.5..=89.5).contains(&self.camera.pitch)
        {
            return Err("保存的相机朝向无效".into());
        }
        crate::view_navigation::zoom_after(self.camera.zoom, 0.)?;
        if !(0..4).contains(&self.angles[0])
            || !(0..360).contains(&self.angles[1])
            || !(15..=80).contains(&self.angles[2])
        {
            return Err("保存的等轴朝向无效".into());
        }
        for v in self.navigation {
            v.validate()?;
        }
        self.framing.validate()?;
        self.source.bounds()?;
        for b in [self.horizontal, self.vertical].into_iter().flatten() {
            point_bounds(b)?;
        }
        if let Some(quad) = &self.quad {
            if let Some(p) = quad.center {
                point_bounds([p, p])?;
            }
            if !(0..360).contains(&quad.yaw) || !(15..=80).contains(&quad.pitch) {
                return Err("保存的四视图朝向无效".into());
            }
            for v in quad.navigation {
                v.validate()?;
            }
        }
        if !(crate::camera_distance::MINIMUM..=crate::camera_distance::MAXIMUM)
            .contains(&self.radius)
        {
            return Err("保存的渲染距离无效".into());
        }
        Ok(())
    }
    fn capture(app: &App) -> Option<Self> {
        let key = app.current_key()?;
        let (bounds, volume, cut) = if let Some(w) = app.selected_window() {
            (w.bounds, w.volume, w.cut)
        } else if let Some((_, mode, bounds, cut)) =
            app.surface_request.filter(|(k, _, _, _)| *k == key)
        {
            (bounds, matches!(mode, NearMode::Volume), cut)
        } else {
            return None;
        };
        let previous = app.view_key().and_then(|key| app.view_state.entry(&key));
        Some(Self {
            camera: app.spectator_camera,
            workplane: app.work_plane.saved,
            lighting: crate::lighting::Settings::default(),
            spectator: app.spectator,
            top_down: app.show_top_down,
            angles: [
                i32::from(app.surface_camera),
                app.surface_yaw,
                app.surface_pitch,
            ],
            navigation: [
                app.surface_navigation,
                app.top_down_navigation,
                app.slice_navigation,
                app.vertical_navigation,
            ]
            .map(Into::into),
            framing: crate::orthographic_world::SavedFraming::capture(
                &app.world_orthographic_framing,
            ),
            source: Source {
                bounds: [bounds.minimum, bounds.maximum],
                volume,
                cut,
                fixed: app.selected_window().is_some(),
            },
            horizontal: app
                .slice_grid
                .as_ref()
                .filter(|(w, d, _)| (*w, *d) == key)
                .map(|(_, _, s)| [s.bounds.minimum, s.bounds.maximum])
                .or_else(|| previous.and_then(|e| e.view.horizontal)),
            vertical: app
                .vertical_grid
                .as_ref()
                .filter(|(w, d, _)| (*w, *d) == key)
                .map(|(_, _, s)| [s.bounds.minimum, s.bounds.maximum])
                .or_else(|| previous.and_then(|e| e.view.vertical)),
            quad: app.quad.saved_navigation(key),
            radius: app.spectator_radius,
            guides: [
                app.surface_frame,
                app.surface_axes,
                app.surface_chunk_grid,
                app.surface_section_guides,
            ],
        })
    }
    fn apply(&self, app: &mut App) -> Result<(), String> {
        self.validate()?;
        let bounds = self.source.bounds()?;
        let key = app.current_key().ok_or("尚未选择存档与维度")?;
        self.workplane.validate_source(
            &app.worlds[key.0],
            &app.worlds[key.0].dimensions[key.1].name,
        )?;
        crate::keyboard_bindings::interrupt_view_gesture(app);
        app.work_plane = crate::work_plane::State::restore(self.workplane);
        app.spectator_camera = self.camera;
        app.spectator = self.spectator;
        app.show_top_down = self.top_down;
        app.show_advanced = false;
        app.surface_camera = self.angles[0] as u8;
        app.surface_yaw = self.angles[1];
        app.surface_pitch = self.angles[2];
        [
            app.surface_navigation,
            app.top_down_navigation,
            app.slice_navigation,
            app.vertical_navigation,
        ] = self.navigation.map(Into::into);
        app.surface_area = crate::scene_selection::coordinates(bounds);
        app.surface_cut_top = self.source.cut;
        app.selected_scene = self.source.fixed.then_some(crate::scene_selection::Window {
            key,
            bounds,
            volume: self.source.volume,
            cut: self.source.cut,
        });
        app.window_camera = app.selected_scene;
        app.spectator_radius = self.radius.min(crate::desktop_state::distance(app).maximum);
        [
            app.surface_frame,
            app.surface_axes,
            app.surface_chunk_grid,
            app.surface_section_guides,
        ] = self.guides;
        if let Some([minimum, maximum]) = self.horizontal {
            app.slice_area = crate::scene_selection::coordinates(Bounds { minimum, maximum });
        }
        if let Some([minimum, maximum]) = self.vertical {
            app.vertical_area = crate::scene_selection::coordinates(Bounds { minimum, maximum });
        }
        app.quad.restore_navigation(key, self.quad.as_ref());
        app.spectator_keys.clear();
        app.invalidate_near();
        app.clear_stream_scene();
        app.world_orthographic_framing = self.framing.restore();
        app.inspect_surface(self.source.volume);
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bookmark {
    name: String,
    view: View,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    root: PathBuf,
    dimension: String,
    view: View,
    bookmarks: Vec<Bookmark>,
}
type Key = (PathBuf, String);
impl Entry {
    fn key(&self) -> Key {
        (self.root.clone(), self.dimension.clone())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    version: u32,
    entries: Vec<Entry>,
    recent: Vec<(PathBuf, String)>,
}
impl Default for Library {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
            recent: Vec::new(),
        }
    }
}
impl Library {
    fn read(values: &serde_json::Map<String, serde_json::Value>) -> Result<Self, String> {
        let Some(value) = values.get("views") else {
            return Ok(Self::default());
        };
        let result: Self =
            serde_json::from_value(value.clone()).map_err(|e| format!("视图配置无效：{e}"))?;
        if result.version != 1 {
            return Err(format!("不支持视图配置版本 {}", result.version));
        }
        let mut keys = std::collections::BTreeSet::new();
        for e in &result.entries {
            if !e.root.is_absolute() || e.dimension.is_empty() || !keys.insert(e.key()) {
                return Err("视图配置的存档、维度或重复记录无效".into());
            }
            e.view.validate()?;
            let mut names = std::collections::BTreeSet::new();
            for b in &e.bookmarks {
                name(&b.name)?;
                if !names.insert(&b.name) {
                    return Err("书签名称重复".into());
                }
                b.view.validate()?;
            }
        }
        let mut roots = std::collections::BTreeSet::new();
        for (root, dim) in &result.recent {
            if !roots.insert(root) || !keys.contains(&(root.clone(), dim.clone())) {
                return Err("最近使用维度记录无效".into());
            }
        }
        Ok(result)
    }
    fn merge(&mut self, entry: Entry) {
        if let Some(old) = self
            .entries
            .iter_mut()
            .find(|v| v.root == entry.root && v.dimension == entry.dimension)
        {
            *old = entry.clone();
        } else {
            self.entries.push(entry.clone());
        }
        if let Some(old) = self.recent.iter_mut().find(|(root, _)| *root == entry.root) {
            old.1 = entry.dimension;
        } else {
            self.recent.push((entry.root, entry.dimension));
        }
    }
}
fn name(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > 128 || value.chars().any(char::is_control)
    {
        return Err("书签名须为 1–128 个非控制字符".into());
    }
    Ok(())
}
#[derive(Default)]
struct Pending {
    entries: BTreeMap<Key, (u64, Entry)>,
    brush: Option<serde_json::Value>,
    selection: Option<serde_json::Value>,
    desktop: Option<serde_json::Value>,
    search: Option<serde_json::Value>,
    block_definitions: Option<serde_json::Value>,
    sequence: u64,
    closing: bool,
}
pub(crate) struct EditorPreferences {
    shared: Arc<(Mutex<Pending>, Condvar)>,
    brush: Option<serde_json::Value>,
    selection: Option<serde_json::Value>,
    search: Option<serde_json::Value>,
}
impl EditorPreferences {
    /// 预建提交持有实际队列，配置写盘成功后才交出本组编辑器偏好。
    pub(crate) fn submit(self) {
        let (mutex, wake) = &*self.shared;
        let mut pending = mutex.lock().expect("编辑器偏好配置队列锁被毒化");
        if let Some(brush) = self.brush {
            pending.brush = Some(brush);
        }
        if let Some(selection) = self.selection {
            pending.selection = Some(selection);
        }
        if let Some(search) = self.search {
            pending.search = Some(search);
        }
        drop(pending);
        wake.notify_one();
    }
}
struct Writer {
    shared: Arc<(Mutex<Pending>, Condvar)>,
    thread: Option<JoinHandle<()>>,
    errors: mpsc::Receiver<String>,
}
impl Writer {
    fn new() -> Result<Self, String> {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let worker = Arc::clone(&shared);
        let (sender, errors) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("editor-preferences".into())
            .spawn(move || {
                let (mutex, wake) = &*worker;
                loop {
                    let mut pending = mutex.lock().expect("编辑器配置锁被毒化");
                    while pending.entries.is_empty()
                        && pending.brush.is_none()
                        && pending.selection.is_none()
                        && pending.desktop.is_none()
                        && pending.search.is_none()
                        && pending.block_definitions.is_none()
                        && !pending.closing
                    {
                        pending = wake.wait(pending).expect("编辑器配置锁被毒化");
                    }
                    if pending.entries.is_empty()
                        && pending.brush.is_none()
                        && pending.selection.is_none()
                        && pending.desktop.is_none()
                        && pending.search.is_none()
                        && pending.block_definitions.is_none()
                        && pending.closing
                    {
                        break;
                    }
                    // 最长两秒提交一次；停止移动后 350ms 合并提交，关闭时直接排空。
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !pending.closing && Instant::now() < deadline {
                        let wait = Duration::from_millis(350)
                            .min(deadline.saturating_duration_since(Instant::now()));
                        let (next, timeout) = wake
                            .wait_timeout(pending, wait)
                            .expect("编辑器配置锁被毒化");
                        pending = next;
                        if timeout.timed_out() {
                            break;
                        }
                    }
                    let mut entries = std::mem::take(&mut pending.entries)
                        .into_values()
                        .collect::<Vec<_>>();
                    entries.sort_by_key(|(sequence, _)| *sequence);
                    let brush = pending.brush.take();
                    let selection = pending.selection.take();
                    let desktop = pending.desktop.take();
                    let search = pending.search.take();
                    let block_definitions = pending.block_definitions.take();
                    drop(pending);
                    let result = launch::update_rust_prefs(|values| {
                        if !entries.is_empty() {
                            let mut library = Library::read(values)?;
                            for (_, entry) in entries {
                                library.merge(entry);
                            }
                            values.insert(
                                "views".into(),
                                serde_json::to_value(library).map_err(|e| e.to_string())?,
                            );
                        }
                        if let Some(brush) = brush {
                            values.insert("brush".into(), brush);
                        }
                        if let Some(search) = search {
                            values.insert("nbt_search".into(), search);
                        }
                        if let Some(block_definitions) = block_definitions {
                            values.insert("block_definitions".into(), block_definitions);
                        }
                        if let Some(desktop) = desktop {
                            values.insert("desktop".into(), desktop);
                        }
                        if let Some(selection) = selection {
                            values.insert("selection".into(), selection);
                        }
                        Ok(())
                    });
                    if let Err(error) = result {
                        let message = format!("保存编辑器设置失败：{error}");
                        log::error!("[view-state] {message}");
                        let _ = sender.send(message);
                    }
                }
            })
            .map_err(|e| format!("无法启动视图配置线程：{e}"))?;
        Ok(Self {
            shared,
            thread: Some(thread),
            errors,
        })
    }
    fn submit(&self, entry: Entry) {
        let (mutex, wake) = &*self.shared;
        let mut pending = mutex.lock().expect("编辑器配置锁被毒化");
        pending.sequence = pending.sequence.wrapping_add(1);
        let sequence = pending.sequence;
        pending.entries.insert(entry.key(), (sequence, entry));
        drop(pending);
        wake.notify_one();
    }
    fn submit_brush(&self, value: serde_json::Value) {
        self.submit_tool_preferences(Some(value), None);
    }
    fn submit_search(&self, value: serde_json::Value) {
        self.submit_editor_preferences(None, None, Some(value));
    }
    fn submit_block_definitions(&self, value: serde_json::Value) {
        let (mutex, wake) = &*self.shared;
        mutex.lock().expect("配置队列锁被毒化").block_definitions = Some(value);
        wake.notify_one();
    }
    fn submit_desktop(&self, value: serde_json::Value) {
        let (mutex, wake) = &*self.shared;
        mutex.lock().expect("配置队列锁被毒化").desktop = Some(value);
        wake.notify_one();
    }
    fn submit_selection(&self, value: serde_json::Value) {
        self.submit_tool_preferences(None, Some(value));
    }
    fn submit_tool_preferences(
        &self,
        brush: Option<serde_json::Value>,
        selection: Option<serde_json::Value>,
    ) {
        self.submit_editor_preferences(brush, selection, None);
    }
    fn submit_editor_preferences(
        &self,
        brush: Option<serde_json::Value>,
        selection: Option<serde_json::Value>,
        search: Option<serde_json::Value>,
    ) {
        EditorPreferences {
            shared: Arc::clone(&self.shared),
            brush,
            selection,
            search,
        }
        .submit();
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        let (mutex, wake) = &*self.shared;
        mutex.lock().expect("编辑器配置锁被毒化").closing = true;
        wake.notify_one();
        if self.thread.take().is_some_and(|t| t.join().is_err()) {
            log::error!("[view-state] 视图配置线程异常退出");
        }
    }
}
#[derive(Default)]
pub(crate) struct State {
    library: Library,
    writer: Option<Writer>,
    pub(crate) bookmark_name: String,
    pub(crate) status: String,
    persistence_error: Option<String>,
}
impl State {
    pub(crate) fn queue_block_definitions(&self, value: serde_json::Value) -> Result<(), String> {
        self.writer
            .as_ref()
            .ok_or("编辑器配置写入线程尚未初始化")?
            .submit_block_definitions(value);
        Ok(())
    }
    pub(crate) fn queue_brush(&self, value: serde_json::Value) -> Result<(), String> {
        let writer = self.writer.as_ref().ok_or_else(|| {
            self.persistence_error
                .clone()
                .unwrap_or_else(|| "编辑器配置写入线程尚未初始化".into())
        })?;
        writer.submit_brush(value);
        Ok(())
    }
    pub(crate) fn queue_selection(&self, value: serde_json::Value) -> Result<(), String> {
        let writer = self.writer.as_ref().ok_or_else(|| {
            self.persistence_error
                .clone()
                .unwrap_or_else(|| "编辑器配置写入线程尚未初始化".into())
        })?;
        writer.submit_selection(value);
        Ok(())
    }
    pub(crate) fn queue_editor_preferences(
        &self,
        brush: Option<serde_json::Value>,
        selection: Option<serde_json::Value>,
        search: Option<serde_json::Value>,
    ) -> Result<(), String> {
        self.prepare_editor_preferences(brush, selection, search)?
            .submit();
        Ok(())
    }
    pub(crate) fn prepare_editor_preferences(
        &self,
        brush: Option<serde_json::Value>,
        selection: Option<serde_json::Value>,
        search: Option<serde_json::Value>,
    ) -> Result<EditorPreferences, String> {
        let writer = self.writer.as_ref().ok_or_else(|| {
            self.persistence_error
                .clone()
                .unwrap_or_else(|| "编辑器配置写入线程尚未初始化".into())
        })?;
        Ok(EditorPreferences {
            shared: Arc::clone(&writer.shared),
            brush,
            selection,
            search,
        })
    }
    pub(crate) fn queue_search(&self, value: serde_json::Value) -> Result<(), String> {
        self.writer
            .as_ref()
            .ok_or("编辑器配置写入线程尚未初始化")?
            .submit_search(value);
        Ok(())
    }
    pub(crate) fn queue_desktop(&self, value: serde_json::Value) -> Result<(), String> {
        self.writer
            .as_ref()
            .ok_or("编辑器配置写入线程尚未初始化")?
            .submit_desktop(value);
        Ok(())
    }
    pub(crate) fn load() -> Result<(Self, crate::desktop_state::Saved), String> {
        let values = launch::read_rust_prefs().map_err(|e| e.to_string())?;
        let library = Library::read(&values)?;
        let desktop = crate::desktop_state::Saved::read(&values)?;
        let writer = Writer::new()?;
        Ok((
            Self {
                library,
                writer: Some(writer),
                ..Default::default()
            },
            desktop,
        ))
    }
    pub(crate) fn dimension(&self, root: &std::path::Path) -> Option<&str> {
        self.library
            .recent
            .iter()
            .find(|(r, _)| r == root)
            .map(|(_, d)| d.as_str())
    }
    pub(crate) fn has_view(&self, root: &std::path::Path, dimension: &str) -> bool {
        self.library
            .entries
            .iter()
            .any(|e| e.root == root && e.dimension == dimension)
    }
    fn entry(&self, key: &Key) -> Option<&Entry> {
        self.library
            .entries
            .iter()
            .find(|e| e.root == key.0 && e.dimension == key.1)
    }
    fn put(&mut self, entry: Entry) {
        self.library.merge(entry.clone());
        if let Some(writer) = &self.writer {
            writer.submit(entry);
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Action {
    Name(String),
    Save,
    Jump(String),
    Delete(String),
    Player,
    Spawn,
    PlayerSource(usize, Location),
}
impl App {
    fn view_key(&self) -> Option<Key> {
        let (wi, di) = self.current_key()?;
        let world = self.worlds.get(wi)?;
        Some((world.root.clone(), world.dimensions.get(di)?.name.clone()))
    }
    pub(crate) fn remember_view(&mut self) {
        if let Some(writer) = &self.view_state.writer {
            for error in writer.errors.try_iter() {
                self.view_state.persistence_error = Some(error);
            }
        }
        if self.structures.active.is_some() {
            return;
        }
        let Some(key) = self.view_key() else {
            return;
        };
        let Some(view) = View::capture(self) else {
            return;
        };
        let old = self.view_state.entry(&key);
        if old.is_some_and(|e| e.view == view)
            && self.view_state.dimension(&key.0) == Some(key.1.as_str())
        {
            return;
        }
        if let Err(error) = view.validate() {
            self.view_state.status = format!("未保存无效视图：{error}");
            log::error!("[view-state] {}", self.view_state.status);
            return;
        }
        let bookmarks = old.map(|e| e.bookmarks.clone()).unwrap_or_default();
        self.view_state.put(Entry {
            root: key.0,
            dimension: key.1,
            view,
            bookmarks,
        });
    }
    pub(crate) fn initial_dimension(&self, world: &mcedit2_world::WorldSummary) -> usize {
        if let Some(dimension) = self
            .worlds
            .iter()
            .position(|entry| entry.root == world.root)
            .and_then(|index| self.workspace.session_dimensions.get(&index))
            .and_then(|name| {
                world
                    .dimensions
                    .iter()
                    .position(|dimension| &dimension.name == name)
            })
        {
            return dimension;
        }
        let preferred = self
            .view_state
            .dimension(&world.root)
            .map(str::to_owned)
            .or_else(|| {
                self.source_locations
                    .get(&world.root)
                    .and_then(|l| l.player.as_ref())
                    .map(|l| l.dimension.clone())
            });
        preferred
            .and_then(|d| world.dimensions.iter().position(|v| v.name == d))
            .unwrap_or(0)
    }
    pub(crate) fn restore_initial_view(&mut self, terrain_y: i32) {
        let Some(key) = self.view_key() else {
            return;
        };
        self.work_plane = crate::work_plane::State::default();
        if let Some(view) = self.view_state.entry(&key).map(|e| e.view.clone()) {
            if let Err(error) = view.apply(self) {
                self.view_state.status = error.clone();
                log::error!("[view-state] 恢复视图失败：{error}");
            } else {
                self.view_state.status = "已恢复此存档和维度的观察位置".into();
                return;
            }
        }
        crate::desktop_state::default_view(self);
        let location = self
            .source_locations
            .get(&key.0)
            .and_then(|l| {
                l.player
                    .as_ref()
                    .filter(|p| p.dimension == key.1)
                    .or_else(|| l.spawn.as_ref().filter(|p| p.dimension == key.1))
            })
            .cloned();
        if let Some(location) = location {
            match self.locate(&location) {
                Ok(()) => {
                    self.view_state.status = "初始观察位置来自存档玩家 / 出生点".into();
                    return;
                }
                Err(error) => {
                    log::error!("[view-state] 源位置无效：{error}");
                    self.view_state.status = error;
                }
            }
        }
        self.spectator_camera = Camera {
            position: [0., f64::from(terrain_y) + 3., 0.],
            yaw: 0.,
            pitch: 0.,
            zoom: 1.,
            perspective: self.spectator_camera.perspective,
        };
        self.surface_area =
            crate::scene_selection::coordinates(source_around(self.spectator_camera.position));
        self.inspect_surface(true);
    }
    pub(crate) fn locate(&mut self, location: &Location) -> Result<(), String> {
        self.locate_projection(location, None, 1.62)
    }
    pub(crate) fn player_view(&mut self, location: &Location) -> Result<(), String> {
        self.locate_projection(location, Some(true), 0.)
    }
    fn locate_projection(
        &mut self,
        location: &Location,
        perspective: Option<bool>,
        offset: f64,
    ) -> Result<(), String> {
        let (wi, di) = self.current_key().ok_or("请先打开存档")?;
        validate_location(&self.worlds[wi], location, offset)?;
        if self.worlds[wi].dimensions[di].name != location.dimension {
            return Err("定位目标属于另一个维度".into());
        }
        let mut camera = Camera {
            position: location.position,
            yaw: 0.,
            pitch: 0.,
            zoom: 1.,
            perspective: perspective.unwrap_or(self.spectator_camera.perspective),
        };
        camera.position[1] += offset;
        if let Some([yaw, pitch]) = location.rotation {
            if !yaw.is_finite() || !pitch.is_finite() {
                return Err("源朝向不是有限数".into());
            }
            camera.yaw = yaw.rem_euclid(360.);
            camera.pitch = pitch.clamp(-89.5, 89.5);
        }
        camera.zoom = 1.;
        self.locate_camera(camera, location.position)
    }
    /// 以当前朝向观察精确目标，留出可见距离；与玩家定位共用源窗口交接。
    pub(crate) fn locate_point(&mut self, target: [f64; 3]) -> Result<(), String> {
        self.locate_point_distance(target, 12.)
    }
    pub(crate) fn locate_point_distance(
        &mut self,
        target: [f64; 3],
        distance: f64,
    ) -> Result<(), String> {
        let camera = self.point_camera(target, distance)?;
        let show_advanced = self.show_advanced;
        self.locate_camera(camera, target)?;
        self.show_advanced = show_advanced;
        Ok(())
    }
    pub(crate) fn point_camera(&self, target: [f64; 3], distance: f64) -> Result<Camera, String> {
        if !distance.is_finite() || distance <= 0. {
            return Err("定位距离必须有限且大于零".into());
        }
        position(target)?;
        let mut camera = self.spectator_camera;
        let (_, _, forward) = camera.basis();
        camera.position = std::array::from_fn(|i| target[i] - f64::from(forward[i]) * distance);
        position(camera.position)?;
        Ok(camera)
    }
    pub(crate) fn locate_camera(&mut self, camera: Camera, target: [f64; 3]) -> Result<(), String> {
        position(camera.position)?;
        self.invalidate_near();
        self.clear_stream_scene();
        self.selected_scene = None;
        self.window_camera = None;
        self.surface_cut_top = None;
        self.spectator_camera = camera;
        self.spectator = true;
        self.show_top_down = false;
        self.show_advanced = false;
        self.use_gpu = true;
        self.spectator_keys.clear();
        self.surface_area = crate::scene_selection::coordinates(source_around(camera.position));
        let [x, y, z] = target.map(|p| p.floor() as i32);
        self.query_coordinates = format!("{x},{y},{z}");
        self.inspect_surface(true);
        Ok(())
    }
}
pub(crate) fn source_around(position: [f64; 3]) -> Bounds {
    let x = (position[0].floor() as i32)
        .saturating_sub(16)
        .clamp(-29_999_999, 29_999_968);
    let y = (position[1].floor() as i32)
        .saturating_sub(24)
        .clamp(-2048, 4048);
    let z = (position[2].floor() as i32)
        .saturating_sub(16)
        .clamp(-29_999_999, 29_999_968);
    Bounds {
        minimum: [x, y, z],
        maximum: [x + 31, y + 47, z + 31],
    }
}
pub(crate) fn update(app: &mut App, action: Action) -> Task<Message> {
    let result = (|| -> Result<(), String> {
        if let Action::Name(value) = action {
            app.view_state.bookmark_name = value;
            return Ok(());
        }
        let key = app.view_key().ok_or("请先选择存档与维度")?;
        match action {
            Action::Name(_) => unreachable!(),
            Action::Save => {
                let label = app.view_state.bookmark_name.trim().to_owned();
                name(&label)?;
                let view = View::capture(app).ok_or("尚无已应用的观察窗口")?;
                view.validate()?;
                let mut entry = app
                    .view_state
                    .entry(&key)
                    .cloned()
                    .unwrap_or_else(|| Entry {
                        root: key.0,
                        dimension: key.1,
                        view: view.clone(),
                        bookmarks: Vec::new(),
                    });
                if let Some(b) = entry.bookmarks.iter_mut().find(|b| b.name == label) {
                    b.view = view.clone();
                } else {
                    entry.bookmarks.push(Bookmark {
                        name: label.clone(),
                        view: view.clone(),
                    });
                }
                entry.view = view;
                app.view_state.put(entry);
                app.view_state.status = format!("已保存书签：{label}");
            }
            Action::Jump(label) => {
                let view = app
                    .view_state
                    .entry(&key)
                    .and_then(|e| e.bookmarks.iter().find(|b| b.name == label))
                    .ok_or("书签不存在")?
                    .view
                    .clone();
                view.apply(app)?;
                app.view_state.status = format!("已定位：{label}");
            }
            Action::Delete(label) => {
                let mut entry = app.view_state.entry(&key).ok_or("书签不存在")?.clone();
                let index = entry
                    .bookmarks
                    .iter()
                    .position(|b| b.name == label)
                    .ok_or("书签不存在")?;
                entry.bookmarks.remove(index);
                app.view_state.put(entry);
                app.view_state.status = format!("已移除书签：{label}");
            }
            kind @ (Action::Player | Action::Spawn) => {
                let locations = app
                    .source_locations
                    .get(&key.0)
                    .ok_or("存档源位置未成功读取")?;
                let location = if matches!(kind, Action::Player) {
                    locations.player.as_ref()
                } else {
                    locations.spawn.as_ref()
                }
                .ok_or("存档未提供此位置")?
                .clone();
                validate_location(
                    &app.worlds[app.current.expect("当前观察世界已核实")],
                    &location,
                    1.62,
                )?;
                if key.1 != location.dimension {
                    return Err("目标维度没有已索引的源区块".into());
                }
                app.locate(&location)?;
                app.view_state.status = "已定位到存档源位置".into();
            }
            Action::PlayerSource(world, location) => {
                if app.current != Some(world) {
                    return Err("玩家观察目标不属于当前世界".into());
                }
                validate_location(&app.worlds[world], &location, 0.)?;
                app.player_view(&location)?;
                app.view_state.status = "已定位到玩家来源的位置与朝向".into();
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        log::error!("[view-state] {error}");
        app.view_state.status = error;
    }
    Task::none()
}
pub(crate) fn controls(app: &App) -> Element<'_, Message> {
    let l = app.language;
    let mut body = column![text(l.pick("位置与书签", "Locations & bookmarks")).size(16)].spacing(5);
    let locations = app
        .current
        .and_then(|i| app.worlds.get(i))
        .and_then(|w| app.source_locations.get(&w.root));
    body = body.push(
        row![
            button(l.pick("存档玩家位置", "Local player")).on_press_maybe(
                locations
                    .and_then(|v| v.player.as_ref())
                    .map(|_| Message::ViewState(Action::Player))
            ),
            button(l.pick("世界出生点", "World spawn")).on_press_maybe(
                locations
                    .and_then(|v| v.spawn.as_ref())
                    .map(|_| Message::ViewState(Action::Spawn))
            )
        ]
        .spacing(5),
    );
    body = body.push(
        row![
            text_input(
                l.pick("书签名称", "Bookmark name"),
                &app.view_state.bookmark_name
            )
            .on_input(|v| Message::ViewState(Action::Name(v)))
            .on_submit(Message::ViewState(Action::Save)),
            button(l.pick("保存", "Save")).on_press(Message::ViewState(Action::Save))
        ]
        .spacing(5),
    );
    if let Some(entry) = app.view_key().and_then(|key| app.view_state.entry(&key)) {
        for bookmark in &entry.bookmarks {
            body = body.push(
                row![
                    button(text(&bookmark.name))
                        .on_press(Message::ViewState(Action::Jump(bookmark.name.clone()))),
                    button(l.pick("移除", "Remove"))
                        .on_press(Message::ViewState(Action::Delete(bookmark.name.clone())))
                ]
                .spacing(5),
            );
        }
    }
    if !app.view_state.status.is_empty() {
        body = body.push(text(&app.view_state.status).size(12));
    }
    if let Some(error) = &app.view_state.persistence_error {
        body = body.push(text(error).size(12));
    }
    if let Some(locations) = locations {
        for diagnostic in &locations.diagnostics {
            body = body.push(text(diagnostic).size(12));
        }
    }
    body.into()
}
/// 跨维度定位先走主更新入口，统一撤销旧源任务、实体选择和材质范围。
pub(crate) fn dimension_jump(
    app: &App,
    message: &Message,
) -> Result<Option<(usize, Location)>, String> {
    let Message::ViewState(action) = message else {
        return Ok(None);
    };
    if !matches!(
        action,
        Action::Player | Action::Spawn | Action::PlayerSource(..)
    ) {
        return Ok(None);
    }
    let current = app.current.ok_or("请先打开玩家所属存档")?;
    let world = app.worlds.get(current).ok_or("玩家所属存档已关闭")?;
    let location = match action {
        Action::Player => app
            .source_locations
            .get(&world.root)
            .and_then(|locations| locations.player.as_ref())
            .ok_or("存档未提供本地玩家位置")?,
        Action::Spawn => app
            .source_locations
            .get(&world.root)
            .and_then(|locations| locations.spawn.as_ref())
            .ok_or("存档未提供出生点位置")?,
        Action::PlayerSource(owner, location) if app.current == Some(*owner) => location,
        Action::PlayerSource(..) => return Err("玩家观察目标不属于当前世界".into()),
        _ => unreachable!("仅位置命令进入维度定位"),
    };
    let offset = if matches!(action, Action::PlayerSource(..)) {
        0.
    } else {
        1.62
    };
    let index = validate_location(world, location, offset)?;
    Ok((index != app.dimension_index).then(|| (index, location.clone())))
}
/// 切换维度前先核实完整观察目标；非法源位置不能先改变当前维度。
fn validate_location(
    world: &mcedit2_world::WorldSummary,
    location: &Location,
    offset: f64,
) -> Result<usize, String> {
    position(location.position)?;
    let mut camera = location.position;
    camera[1] += offset;
    position(camera)?;
    if location
        .rotation
        .is_some_and(|rotation| rotation.into_iter().any(|value| !value.is_finite()))
    {
        return Err("玩家来源朝向含非有限数".into());
    }
    world
        .dimensions
        .iter()
        .position(|dimension| dimension.name == location.dimension)
        .ok_or_else(|| format!("目标维度 {} 没有已索引的源区块", location.dimension))
}
pub(crate) fn locations(world: &mcedit2_world::WorldSummary) -> Locations {
    match mcedit2_world::player_catalog::locations(world) {
        Ok(locations) => {
            for error in &locations.diagnostics {
                log::error!("[view-state] {error}");
            }
            locations
        }
        Err(error) => {
            log::error!("[view-state] 读取源位置失败：{error}");
            Locations {
                diagnostics: vec![error.to_string()],
                ..Default::default()
            }
        }
    }
}
