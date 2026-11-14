//! 已应用的源窗口独立于自由飞行近景和输入草稿；切换投影不重新选择源数据。
use super::*;

#[cfg(test)]
#[path = "scene_selection_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Window {
    pub(super) key: (usize, usize),
    pub(super) bounds: Bounds,
    pub(super) volume: bool,
    pub(super) cut: Option<i32>,
}
pub(super) fn coordinates(bounds: Bounds) -> String {
    let [x, y, z] = bounds.minimum;
    let [xx, yy, zz] = bounds.maximum;
    format!("{x},{y},{z};{xx},{yy},{zz}")
}
impl App {
    /// 完整世界的共享网格只用于三维体积；显式地表 / 远景预览保留自己的源。
    pub(super) fn uses_world_mesh(&self) -> bool {
        !self.show_top_down
            && self.surface_exposed
            && self.selected_window().is_none()
            && self.far.is_none()
    }
    fn drop_foreign_scene(&mut self) {
        if self.surface.is_some() && !self.visible_scene_is_current() {
            self.surface = None;
            self.selected_surface = None;
            self.selected_surface_face = None;
            self.surface_hover = None;
            self.world_hover.clear();
            if self.selected_window().is_none() {
                self.selected_scene = None;
                self.window_camera = None;
            }
        }
    }
    pub(super) fn selected_window(&self) -> Option<Window> {
        self.selected_scene
            .filter(|w| self.current_key() == Some(w.key))
    }
    /// 校验和创建完整源快照任务成功后才修改已应用的显示语义。
    fn request_window(
        &mut self,
        bounds: Bounds,
        volume: bool,
        cut: Option<i32>,
    ) -> Result<Window, String> {
        self.drop_foreign_scene();
        parse_scene_window(&coordinates(bounds))?;
        let key = self.current_key().ok_or("请先选择 Java 世界")?;
        let cut = if volume { cut } else { None };
        if cut.is_some_and(|y| y < bounds.minimum[1] || y > bounds.maximum[1]) {
            return Err("剖切高度必须位于当前体积内".into());
        }
        let window = Window {
            key,
            bounds,
            volume,
            cut,
        };
        let preserve_selection = self.visible_scene_is_current()
            && (self.spectator || self.selected_window() == Some(window));
        self.enqueue_near(
            key,
            if volume {
                NearMode::Volume
            } else {
                NearMode::Surface
            },
            bounds,
            cut,
        )?;
        self.far = None;
        self.surface_exposed = volume;
        self.surface_cut_top = cut;
        // 新结果就绪前保留同一世界的最后有效快照。
        if !self.visible_scene_is_current() {
            self.surface = None;
        }
        if !preserve_selection {
            self.selected_surface = None;
            self.selected_surface_face = None;
            self.surface_hover = None;
            self.world_hover.clear();
        }
        self.surface_report = format!(
            "Loading original 3D {} in a read-only worker: {}.",
            if volume {
                "exposed volume"
            } else {
                "top surface"
            },
            coordinates(bounds)
        );
        log::info!(
            "[view] 读取{}窗口 {}，剖切 {:?}",
            if volume {
                "体积"
            } else {
                "完整高度地表"
            },
            coordinates(bounds),
            cut
        );
        Ok(window)
    }
    fn window_error(&mut self, error: String) {
        log::error!("[view] 源窗口请求被拒绝：{error}");
        self.surface_report = format!("3D source refused: {error}");
    }
    pub(super) fn read_scene_input(&mut self, volume: bool, select: bool) {
        self.drop_foreign_scene();
        let previous = self.selected_window();
        let explicit = select || previous.is_some();
        let result = parse_scene_window(&self.surface_area).and_then(|bounds| {
            let cut = if select && previous.is_none_or(|w| w.bounds != bounds) {
                None
            } else {
                self.surface_cut_top
            };
            self.request_window(bounds, volume, cut)
        });
        match result {
            Ok(window) => {
                if explicit {
                    self.selected_scene = Some(window);
                }
                if select && previous != Some(window) {
                    self.clear_stream_scene();
                }
                if explicit
                    && volume
                    && self.spectator
                    && !self.show_top_down
                    && previous
                        .is_none_or(|w| w.bounds != window.bounds || w.volume != window.volume)
                {
                    self.ensure_window_camera(window);
                }
            }
            Err(error) => self.window_error(error),
        }
    }
    pub(super) fn clear_stream_scene(&mut self) {
        self.world_orthographic_framing = Default::default();
        // 原有区块几何缓存保留；旧的流式帧不能替代用户选择的源窗口。
        self.spectator_stream = None;
        self.spectator_stream_view = None;
        self.spectator_stream_snapshot = None;
        self.spectator_stream_near = None;
        self.spectator_fallback = None;
        self.spectator_cache = Default::default();
        self.spectator_pending = Default::default();
        self.spectator_presented = Default::default();
        self.spectator_presented_view = Default::default();
    }
    pub(super) fn ensure_window_camera(&mut self, window: Window) {
        if self.window_camera.is_some_and(|framed| {
            framed.key == window.key
                && framed.bounds == window.bounds
                && framed.volume == window.volume
        }) || (!window.volume && (self.near_busy || self.near_job.is_some()))
        {
            return;
        }
        self.frame_selected_window(window);
    }
    pub(super) fn frame_selected_window(&mut self, window: Window) {
        let mut bounds = window.bounds;
        if !window.volume {
            if let Some((_, _, surface)) = self.surface.as_ref().filter(|(_, _, s)| {
                s.x == bounds.minimum[0]
                    && s.z == bounds.minimum[2]
                    && i64::from(s.width)
                        == i64::from(bounds.maximum[0]) - i64::from(bounds.minimum[0]) + 1
                    && i64::from(s.depth)
                        == i64::from(bounds.maximum[2]) - i64::from(bounds.minimum[2]) + 1
            }) {
                if let (Some(min), Some(max)) = (
                    surface.voxels.iter().map(|v| v.y).min(),
                    surface.voxels.iter().map(|v| v.y).max(),
                ) {
                    bounds.minimum[1] = min;
                    bounds.maximum[1] = max;
                }
            }
        }
        let center: [f64; 3] = std::array::from_fn(|i| {
            (f64::from(bounds.minimum[i]) + f64::from(bounds.maximum[i])) * 0.5
        });
        let radius = (0..3)
            .map(|i| (f64::from(bounds.maximum[i]) - f64::from(bounds.minimum[i]) + 1.).powi(2))
            .sum::<f64>()
            .sqrt()
            * 0.5;
        self.spectator_camera.yaw = 35.;
        self.spectator_camera.pitch = 25.;
        self.spectator_camera.zoom = 1.;
        let (_, _, forward) = self.spectator_camera.basis();
        self.spectator_camera.position =
            std::array::from_fn(|i| center[i] - f64::from(forward[i]) * radius * 4.5);
        self.spectator_keys.clear();
        self.window_camera = Some(window);
    }
    pub(super) fn reload_scene(&mut self) {
        if let Some(window) = self.selected_window() {
            if let Err(error) = self.request_window(window.bounds, window.volume, window.cut) {
                self.window_error(error);
            }
        } else {
            self.inspect_surface(!self.show_top_down);
        }
    }
    pub(super) fn follow_world(&mut self) {
        let Some(window) = self.selected_window() else {
            return;
        };
        let center = if self.spectator && !self.show_top_down {
            self.spectator_camera.position
        } else {
            std::array::from_fn(|i| {
                (f64::from(window.bounds.minimum[i]) + f64::from(window.bounds.maximum[i])) * 0.5
            })
        };
        let bounds = crate::view_state::source_around(center);
        match self.request_window(bounds, !self.show_top_down, None) {
            Ok(_) => {
                self.selected_scene = None;
                if !self.show_top_down {
                    self.use_gpu = true;
                }
                self.window_camera = None;
                self.surface_area = coordinates(bounds);
                self.clear_stream_scene();
            }
            Err(error) => self.window_error(error),
        }
    }
    pub(super) fn cut_scene(&mut self, delta: Option<i32>) {
        let previous = self.selected_window();
        let result = previous
            .map(|w| Ok(w.bounds))
            .unwrap_or_else(|| parse_scene_window(&self.surface_area))
            .and_then(|bounds| {
                let current = previous
                    .and_then(|w| w.cut)
                    .or(self.surface_cut_top)
                    .unwrap_or(bounds.maximum[1]);
                let cut = delta.map(|d| {
                    current
                        .saturating_add(d)
                        .clamp(bounds.minimum[1], bounds.maximum[1])
                });
                self.request_window(bounds, true, cut)
            });
        match result {
            Ok(window) => {
                self.selected_scene = Some(window);
                self.clear_stream_scene();
            }
            Err(error) => self.window_error(error),
        }
    }
    pub(super) fn window_caption(&self) -> String {
        match self.selected_window() {
            Some(w) if w.volume => format!(
                "{}：{}{}",
                self.language.pick("限定体积", "Bounded volume"),
                coordinates(w.bounds),
                w.cut.map_or(String::new(), |y| format!(" · Y≤{y}"))
            ),
            Some(w) => format!(
                "{}：X {}..{} / Z {}..{}",
                self.language
                    .pick("限定地表（完整高度）", "Bounded surface (full height)"),
                w.bounds.minimum[0],
                w.bounds.maximum[0],
                w.bounds.minimum[2],
                w.bounds.maximum[2]
            ),
            None => self
                .language
                .pick(
                    "世界视图（跟随相机或等轴平移）",
                    "World view (follow camera or orthographic pan)",
                )
                .into(),
        }
    }
}
