//! 2D overhead overview of the exact same original source surface used by
//! the 3D renderer. Never read another chunk, invent distant terrain or
//! return an approximate LOD sample to the editor.
use crate::{isometric::Face, palette, textures::TextureBank, Message};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use mcedit2_world::surface::{SurfaceView, SurfaceVoxel};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub(crate) struct TopDown {
    pub(crate) selection_handle: Option<crate::selection_handle::Layer>,
    pub brush: Option<crate::brush_stroke::Config>,
    pub brush_key: Option<(usize, usize)>,
    pub layers: Option<crate::scene_layers::View>,
    pub chunk_column: Option<crate::chunk_outline::Column>,
    pub source: Arc<SurfaceView>,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub selected: Option<(i32, i32, i32)>,
    pub texture_bank: Option<Arc<TextureBank>>,
    pub show_textures: bool,
    pub(crate) block_layers: crate::block_visibility::Layers,
    pub entity_frame: Option<Arc<crate::entity_geometry::Frame>>,
    pub grid_cache: Arc<crate::projected_entities::GridCache>,
    pub lighting: Arc<crate::lighting::Table>,
    pub quality: crate::gpu_mesh::Quality,
    pub source_revision: u64,
}
struct Projection {
    scale: f32,
    origin: Point,
    x: i32,
    z: i32,
}
impl Projection {
    fn new(v: &SurfaceView, size: Size, zoom: f32, pan_x: f32, pan_y: f32) -> Option<Self> {
        if v.width == 0 || v.depth == 0 || size.width <= 0.0 || size.height <= 0.0 {
            return None;
        }
        let scale = (size.width / v.width as f32).min(size.height / v.depth as f32)
            * zoom.clamp(
                crate::view_navigation::MIN_ZOOM,
                crate::view_navigation::MAX_ZOOM,
            );
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        Some(Self {
            scale,
            x: v.x,
            z: v.z,
            origin: Point::new(
                (size.width - v.width as f32 * scale) * 0.5 + pan_x,
                (size.height - v.depth as f32 * scale) * 0.5 + pan_y,
            ),
        })
    }
    fn point(&self, x: f64, z: f64) -> Point {
        // Only convert the delta to f32, not the +/-30m source coordinate.
        Point::new(
            self.origin.x + (x - self.x as f64) as f32 * self.scale,
            self.origin.y + (z - self.z as f64) as f32 * self.scale,
        )
    }
    fn cell(&self, p: Point) -> Option<(i32, i32)> {
        let dx = ((p.x - self.origin.x) / self.scale).floor() as i64;
        let dz = ((p.y - self.origin.y) / self.scale).floor() as i64;
        Some((
            i32::try_from(self.x as i64 + dx).ok()?,
            i32::try_from(self.z as i64 + dz).ok()?,
        ))
    }
}
/// 俯视源窗口、格网、标尺及拾取使用同一投影，不重复推导屏幕范围。
pub(crate) fn projection(
    s: &SurfaceView,
    size: Size,
    navigation: crate::view_navigation::Viewport,
) -> Option<(crate::projected_entities::Mapping, ([f64; 3], [f64; 3]))> {
    let p = Projection::new(s, size, navigation.zoom, navigation.pan.x, navigation.pan.y)?;
    let max_x = i64::from(s.x) + i64::from(s.width) - 1;
    let max_z = i64::from(s.z) + i64::from(s.depth) - 1;
    Some((
        crate::projected_entities::Mapping::Orthogonal {
            origin: [f64::from(s.x) - 0.5, 0., f64::from(s.z) - 0.5],
            screen: [p.origin.x, p.origin.y],
            right: [1., 0., 0.],
            down: [0., 0., 1.],
            depth: [0., 1., 0.],
            scale: f64::from(p.scale),
        },
        (
            [
                f64::from(s.x) - 0.5,
                f64::from(s.y_min) - 0.5,
                f64::from(s.z) - 0.5,
            ],
            [
                max_x as f64 + 0.5,
                f64::from(s.y_max) + 0.5,
                max_z as f64 + 0.5,
            ],
        ),
    ))
}
fn tint(v: &SurfaceVoxel) -> Color {
    palette::tint(&v.state, v.kind, true).unwrap_or_else(|| palette::unknown_tint(&v.state, v.kind))
}
pub(crate) struct FaceStyle {
    pub rgba: [f32; 4],
    pub uv: [[f32; 2]; 4],
    pub mode: u32,
    pub page: u32,
}
/// 原俯视格网与独立小地图共用最高源面的贴图、Alpha 和真实群系颜色。
pub(crate) fn top_face_style(
    voxel: &SurfaceVoxel,
    show_textures: bool,
    bank: Option<&TextureBank>,
    colors: Option<&mut crate::textures::TintContext<'_>>,
    atlas: Option<&crate::textures::TextureAtlas>,
) -> FaceStyle {
    let color = tint(voxel);
    let mut result = FaceStyle {
        rgba: [color.r, color.g, color.b, color.a],
        uv: [atlas.map_or([0.; 2], |atlas| atlas.white.sample(0.5, 0.5)); 4],
        mode: 0,
        page: atlas.map_or(0, |atlas| atlas.white.page),
    };
    if show_textures {
        if let Some((uv, alpha, page)) = bank.and_then(|bank| {
            bank.atlas_face_style_at(&voxel.state, Face::Top, 0, (voxel.x, voxel.y, voxel.z))
        }) {
            let tint = colors.map_or([1.; 3], |colors| {
                colors.face(
                    &voxel.state,
                    Face::Top,
                    0,
                    (voxel.x, voxel.y, voxel.z),
                    voxel.biome.as_deref(),
                )
            });
            result = FaceStyle {
                rgba: [tint[0], tint[1], tint[2], 1.],
                uv,
                mode: alpha.gpu_bits(),
                page,
            };
        }
    }
    result
}
/// Draw a bounded sample of the same top-face atlas UV used in 3D.
/// Canvas has no shared GPU sampler, so resolution adapts to the screen cell.
fn textured_cell(
    frame: &mut Frame,
    voxel: &SurfaceVoxel,
    origin: Point,
    scale: f32,
    bank: &TextureBank,
    colors: &mut crate::textures::TintContext<'_>,
    light: [f32; 3],
) -> bool {
    let Some((uv, _, page)) =
        bank.atlas_face_style_at(&voxel.state, Face::Top, 0, (voxel.x, voxel.y, voxel.z))
    else {
        return false;
    };
    let steps = if scale >= 24.0 {
        8
    } else if scale >= 8.0 {
        4
    } else {
        1
    };
    let step = scale / steps as f32;
    let tint = colors.face(
        &voxel.state,
        Face::Top,
        0,
        (voxel.x, voxel.y, voxel.z),
        voxel.biome.as_deref(),
    );
    let tint: [f32; 3] = std::array::from_fn(|i| tint[i] * light[i]);
    for y in 0..steps {
        for x in 0..steps {
            let u = (x as f32 + 0.5) / steps as f32;
            let v = (y as f32 + 0.5) / steps as f32;
            let a = 1.0 - u;
            let b = 1.0 - v;
            let uv_point = [
                uv[0][0] * a * b + uv[1][0] * u * b + uv[2][0] * u * v + uv[3][0] * a * v,
                uv[0][1] * a * b + uv[1][1] * u * b + uv[2][1] * u * v + uv[3][1] * a * v,
            ];
            let Some(rgba) = bank.atlas_pixel(uv_point, page) else {
                return false;
            };
            if rgba[3] == 0 {
                continue;
            }
            frame.fill(
                &Path::rectangle(
                    Point::new(origin.x + x as f32 * step, origin.y + y as f32 * step),
                    Size::new(step + 0.01, step + 0.01),
                ),
                Color::from_rgba(
                    rgba[0] as f32 / 255.0 * tint[0],
                    rgba[1] as f32 / 255.0 * tint[1],
                    rgba[2] as f32 / 255.0 * tint[2],
                    rgba[3] as f32 / 255.0,
                ),
            );
        }
    }
    true
}
fn exact_visible(
    v: &SurfaceView,
    bank: Option<&TextureBank>,
    layers: crate::block_visibility::Layers,
) -> BTreeMap<(i32, i32), usize> {
    let mut tops: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    for (index, voxel) in v.voxels.iter().enumerate() {
        if !layers.visible(&voxel.state, bank) {
            continue;
        }
        if voxel.span != 1 {
            continue;
        }
        let key = (voxel.x, voxel.z);
        if tops.get(&key).is_none_or(|&old| v.voxels[old].y < voxel.y) {
            tops.insert(key, index);
        }
    }
    tops
}
impl TopDown {
    fn voxel_visible(&self, voxel: &SurfaceVoxel) -> bool {
        self.block_layers
            .visible(&voxel.state, self.texture_bank.as_deref())
    }
    fn selection_view(&self, size: Size) -> Option<crate::selection_handle::View> {
        let (mapping, clip) = self.entity_projection(size)?;
        Some(crate::selection_handle::View::Planar(mapping, size, clip))
    }
    fn layer_layout(&self, size: Size) -> Option<crate::scene_layers::PlanarLayout> {
        let (mapping, clip) = self.entity_projection(size)?;
        Some(crate::scene_layers::PlanarLayout {
            mapping,
            clip,
            size,
        })
    }
    fn grid_frame(
        &self,
        size: Size,
    ) -> Result<
        Option<(
            Arc<crate::projected_entities::Rendered>,
            iced::widget::image::Handle,
        )>,
        String,
    > {
        let Some(frame) = self.entity_frame.as_ref() else {
            if self.grid_cache.pending() {
                self.grid_cache.cancel();
            }
            return Ok(None);
        };
        let Some(base) = self.texture_bank.as_ref().and_then(|bank| bank.atlas()) else {
            return Ok(None);
        };
        if !crate::textures::atlas_compatible(&frame.atlas, base) {
            self.grid_cache.cancel();
            return Ok(None);
        }
        let Some((mapping, clip)) = self.entity_projection(size) else {
            return Ok(None);
        };
        let key = crate::projected_entities::GridKey {
            frame: Arc::clone(frame),
            mapping,
            clip,
            size,
            quality: self.quality,
            source_revision: self.source_revision,
            lighting: std::sync::Arc::clone(&self.lighting),
        };
        if self.grid_cache.needs(&key) {
            let view = self.clone();
            let atlas = Arc::clone(&frame.atlas);
            self.grid_cache.request_with(key.clone(), move |cancelled| {
                view.terrain_vertices(size, &atlas, cancelled)
            });
        }
        if let Some(error) = self.grid_cache.error(&key) {
            return Err(error);
        }
        Ok(self.grid_cache.ready(&key))
    }
    /// 与俯视范围相同的原始地形顶面，作为实体联合光栅的深度来源。
    pub(crate) fn terrain_vertices(
        &self,
        size: Size,
        atlas: &crate::textures::TextureAtlas,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Option<Vec<crate::gpu_mesh::Vertex>> {
        let projection = Projection::new(&self.source, size, self.zoom, self.pan_x, self.pan_y)?;
        let (mapping, clip) = self.entity_projection(size)?;
        let mut tops = BTreeMap::<(i32, i32), usize>::new();
        for (index, voxel) in self.source.voxels.iter().enumerate() {
            if !self.voxel_visible(voxel) {
                continue;
            }
            if index % 1024 == 0 && cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
            if voxel.span != 1 {
                log::error!("[entities] 俯视源含聚合 LOD，无法提供精确地形深度");
                return None;
            }
            // 先从候选中移除本帧已实际建模的旗帜，再从真实已有源里选下面的最高地形。
            // 不在选好最高格之后跳过，也不合成源窗口没有提供的下层方块。
            if self
                .entity_frame
                .as_ref()
                .zip(self.texture_bank.as_deref())
                .is_some_and(|(frame, bank)| {
                    frame
                        .block_visual_faces([voxel.x, voxel.y, voxel.z], &voxel.state, bank, atlas)
                        .is_some_and(|(origin, faces)| {
                            faces.iter().any(|quad| {
                                crate::projected_entities::quad_has_visible_projection(
                                    quad, origin, mapping, clip,
                                )
                            })
                        })
                })
            {
                continue;
            }
            let key = (voxel.x, voxel.z);
            if tops
                .get(&key)
                .is_none_or(|&old| self.source.voxels[old].y < voxel.y)
            {
                tops.insert(key, index);
            }
        }
        let mut out = Vec::new();
        let mut colors = self
            .texture_bank
            .as_ref()
            .map(|bank| bank.tint_context(&self.source.biomes));
        for (slot, index) in tops.into_values().enumerate() {
            if slot % 1024 == 0 && cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
            let voxel = &self.source.voxels[index];
            let at = projection.point(voxel.x as f64, voxel.z as f64);
            let d = projection.scale * f32::from(voxel.span);
            let points = [
                at,
                Point::new(at.x + d, at.y),
                Point::new(at.x + d, at.y + d),
                Point::new(at.x, at.y + d),
            ];
            let style = top_face_style(
                voxel,
                self.show_textures,
                self.texture_bank.as_deref(),
                colors.as_mut(),
                Some(atlas),
            );
            crate::projected_entities::grid_face(
                &mut out,
                points,
                voxel.y as f32 + 0.5,
                style.rgba,
                style.uv,
                style.mode,
                style.page,
                crate::lighting::FULLBRIGHT,
            );
        }
        Some(out)
    }
    /// 俯视地图与地形使用相同格网、平移和源窗口。
    pub(crate) fn entity_projection(
        &self,
        size: Size,
    ) -> Option<(crate::projected_entities::Mapping, ([f64; 3], [f64; 3]))> {
        projection(
            &self.source,
            size,
            crate::view_navigation::Viewport {
                zoom: self.zoom,
                pan: iced::Vector::new(self.pan_x, self.pan_y),
            },
        )
    }
    pub(crate) fn element<'a>(
        self,
        app: &crate::App,
        height: iced::Length,
    ) -> iced::Element<'a, Message> {
        let picker = self.clone();
        let projection = self.clone();
        crate::library::scene_target(
            app,
            crate::view_navigation::Target::TopDown,
            iced::widget::canvas(self)
                .width(iced::Length::Fill)
                .height(height)
                .into(),
            move |point, size| {
                Some(crate::brush_stroke::InputFrame::surface(
                    Arc::clone(&picker.source),
                    crate::view_navigation::Target::TopDown,
                    picker
                        .pick(point, size)
                        .map(|position| (position, Face::Top)),
                ))
            },
            move |size| projection.selection_view(size),
        )
    }
    fn pick(&self, p: Point, size: Size) -> Option<(i32, i32, i32)> {
        if self.source.width > 64 || self.source.depth > 64 {
            return None;
        }
        let proj = Projection::new(&self.source, size, self.zoom, self.pan_x, self.pan_y)?;
        let (x, z) = proj.cell(p)?;
        if x < self.source.x
            || z < self.source.z
            || x as i64 >= self.source.x as i64 + self.source.width as i64
            || z as i64 >= self.source.z as i64 + self.source.depth as i64
        {
            return None;
        }
        // 只有已经实际呈现且仍对应当前导航 / 源修订的联合帧，才改变旧格网拾取。
        let managed = self.entity_frame.as_ref().and_then(|frame| {
            let (mapping, clip) = self.entity_projection(size)?;
            let key = crate::projected_entities::GridKey {
                frame: Arc::clone(frame),
                mapping,
                clip,
                size,
                quality: self.quality,
                source_revision: self.source_revision,
                lighting: std::sync::Arc::clone(&self.lighting),
            };
            let (shown, _) = self.grid_cache.ready(&key)?;
            if !self.grid_cache.is_presented(&shown) {
                return None;
            }
            let projected = shown.projected.as_ref()?;
            if !Arc::ptr_eq(&projected.frame, frame) {
                return None;
            }
            Some((
                Arc::clone(frame),
                Arc::clone(shown.atlas.as_ref()?),
                mapping,
                clip,
            ))
        });
        self.source
            .voxels
            .iter()
            .filter(|voxel| {
                self.voxel_visible(voxel)
                    && voxel.span == 1
                    && voxel.x == x
                    && voxel.z == z
                    && !managed
                        .as_ref()
                        .zip(self.texture_bank.as_deref())
                        .is_some_and(|((frame, atlas, mapping, clip), bank)| {
                            frame
                                .block_visual_faces(
                                    [voxel.x, voxel.y, voxel.z],
                                    &voxel.state,
                                    bank,
                                    atlas,
                                )
                                .is_some_and(|(origin, faces)| {
                                    faces.iter().any(|quad| {
                                        crate::projected_entities::quad_has_visible_projection(
                                            quad, origin, *mapping, *clip,
                                        )
                                    })
                                })
                        })
            })
            .max_by_key(|voxel| voxel.y)
            .map(|voxel| (x, voxel.y, z))
    }
}
impl canvas::Program<Message> for TopDown {
    type State = crate::scene_layers::MapState;
    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        self.lighting.enter(&state.light_report);
        let fail = |error: String| {
            *self.grid_cache.presented.lock().expect("格网呈现锁被毒化") = None;
            self.lighting
                .report(&state.light_report, Err(error.clone()));
            vec![crate::lighting::error_geometry(
                renderer,
                bounds.size(),
                &error,
            )]
        };
        let snapshot = match self
            .texture_bank
            .as_ref()
            .map(|bank| bank.render_snapshot())
            .transpose()
        {
            Ok(bank) => bank,
            Err(error) => {
                log::error!("[canvas] 格网动画快照失败：{error}");
                frame.fill_text(canvas::Text {
                    content: format!("材质动画失败：{error}"),
                    ..Default::default()
                });
                return vec![frame.into_geometry()];
            }
        };
        let texture_bank = snapshot.as_ref();

        let mut foreground = Frame::new(renderer, bounds.size());
        let combined = match self.grid_frame(bounds.size()) {
            Ok(frame) => frame,
            Err(error) => return fail(error),
        };
        let mut colors = texture_bank
            .as_ref()
            .map(|bank| bank.tint_context(&self.source.biomes));
        let Some(p) = Projection::new(
            &self.source,
            bounds.size(),
            self.zoom,
            self.pan_x,
            self.pan_y,
        ) else {
            return vec![frame.into_geometry()];
        };
        if self.source.width <= 64 && self.source.depth <= 64 {
            for (_, index) in exact_visible(
                &self.source,
                self.texture_bank.as_deref(),
                self.block_layers,
            ) {
                let voxel = &self.source.voxels[index];
                let origin = p.point(voxel.x as f64, voxel.z as f64);
                if origin.x + p.scale < 0.0
                    || origin.y + p.scale < 0.0
                    || origin.x > bounds.width
                    || origin.y > bounds.height
                {
                    continue;
                }
                let rect = Path::rectangle(origin, Size::new(p.scale, p.scale));
                if combined.is_some() {
                    continue;
                }
                let light = match self.lighting.rgb(crate::lighting::FULLBRIGHT) {
                    Ok(light) => light,
                    Err(error) => {
                        return fail(format!("源格 {},{},{}：{error}", voxel.x, voxel.y, voxel.z))
                    }
                };
                if combined.is_none()
                    && (!self.show_textures
                        || !texture_bank.is_some_and(|bank| {
                            colors.as_mut().is_some_and(|colors| {
                                textured_cell(
                                    &mut frame, voxel, origin, p.scale, bank, colors, light,
                                )
                            })
                        }))
                {
                    frame.fill(&rect, crate::lighting::shade(tint(voxel), light));
                }
            }
        } else if combined.is_none() {
            // Rasterize verified source samples in world space. A sampled
            // 4/8/16-cell patch covers its actual extent, not one arbitrary
            // 2-pixel dot. Never synthesize missing chunks or editor picks.
            let mut footprints: BTreeMap<(i32, i32), usize> = BTreeMap::new();
            for (index, voxel) in self.source.voxels.iter().enumerate() {
                if !self.voxel_visible(voxel) {
                    continue;
                }
                let key = (voxel.x, voxel.z);
                if footprints
                    .get(&key)
                    .is_none_or(|&old| self.source.voxels[old].y < voxel.y)
                {
                    footprints.insert(key, index);
                }
            }
            for (_, index) in footprints {
                let voxel = &self.source.voxels[index];
                let origin = p.point(voxel.x as f64, voxel.z as f64);
                let side = p.scale * voxel.span as f32;
                if origin.x + side < 0.0
                    || origin.y + side < 0.0
                    || origin.x > bounds.width
                    || origin.y > bounds.height
                {
                    continue;
                }
                let rect = Path::rectangle(origin, Size::new(side, side));
                let light = match self.lighting.rgb(crate::lighting::FULLBRIGHT) {
                    Ok(light) => light,
                    Err(error) => {
                        return fail(format!("源格 {},{},{}：{error}", voxel.x, voxel.y, voxel.z))
                    }
                };
                if !self.show_textures
                    || !texture_bank.is_some_and(|bank| {
                        colors.as_mut().is_some_and(|colors| {
                            textured_cell(&mut frame, voxel, origin, side, bank, colors, light)
                        })
                    })
                {
                    frame.fill(&rect, crate::lighting::shade(tint(voxel), light));
                }
            }
        }
        // 被旗帜实体层让出的真实下层也可以被选中，轮廓不再依赖原始最高整格候选。
        if let Some((x, _, z)) = self.selected.filter(|&(x, y, z)| {
            self.source
                .voxels
                .iter()
                .any(|v| v.span == 1 && (v.x, v.y, v.z) == (x, y, z))
        }) {
            let rect = Path::rectangle(
                p.point(f64::from(x), f64::from(z)),
                Size::new(p.scale, p.scale),
            );
            foreground.stroke(
                &rect,
                Stroke::default()
                    .with_width(2.0)
                    .with_color(Color::from_rgb(0.1, 1.0, 0.9)),
            );
        }
        let mut result = vec![frame.into_geometry()];
        if let Some((rendered, image)) = combined {
            let mut image_frame = Frame::new(renderer, bounds.size());
            image_frame.draw_image(
                Rectangle::with_size(bounds.size()),
                iced::advanced::image::Image::new(image)
                    .filter_method(iced::advanced::image::FilterMethod::Nearest),
            );
            result.push(image_frame.into_geometry());
            *self
                .grid_cache
                .presented
                .lock()
                .expect("俯视呈现状态锁被毒化") = Some(rendered);
        } else {
            *self
                .grid_cache
                .presented
                .lock()
                .expect("俯视呈现状态锁被毒化") = None;
        }
        state.brush.draw(
            &mut foreground,
            self.brush.as_ref(),
            cursor.position_in(bounds),
            |pnt| p.point(pnt[0] + 0.5, pnt[2] + 0.5),
            |point| {
                Some(crate::brush_stroke::InputFrame::surface(
                    Arc::clone(&self.source),
                    crate::view_navigation::Target::TopDown,
                    self.pick(point, bounds.size()).map(|p| (p, Face::Top)),
                ))
            },
            crate::brush_cursor::Projection::Planar(
                self.entity_projection(bounds.size())
                    .expect("有效俯视范围已有投影")
                    .0,
            ),
            crate::view_navigation::Target::TopDown,
        );
        if let (Some(column), Some((mapping, clip))) =
            (self.chunk_column, self.entity_projection(bounds.size()))
        {
            column.draw_planar(&mut foreground, mapping, clip);
        }
        if let (Some(layer), Some(view)) =
            (&self.selection_handle, self.selection_view(bounds.size()))
        {
            layer.draw(view, &mut foreground, cursor.position_in(bounds));
        }
        self.lighting.report(&state.light_report, Ok(()));
        result.push(foreground.into_geometry());
        if let (Some(layers), Some(layout)) = (&self.layers, self.layer_layout(bounds.size())) {
            result.push(layers.draw(&state.layers, renderer, layout, cursor.position_in(bounds)));
        } else {
            state.layers.clear();
        }
        result
    }
    fn update(
        &self,
        state: &mut Self::State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let result = (|| {
            if let Some(layer) = &self.selection_handle {
                if let Some(action) = layer.event(
                    crate::view_navigation::Target::TopDown,
                    self.selection_view(bounds.size()),
                    event,
                    bounds,
                    cursor,
                    |p| {
                        Some(crate::brush_stroke::InputFrame::surface(
                            Arc::clone(&self.source),
                            crate::view_navigation::Target::TopDown,
                            self.pick(p, bounds.size()).map(|p| (p, Face::Top)),
                        ))
                    },
                ) {
                    return Some(action);
                }
            }

            if let Some(action) = state.brush.event(
                self.brush.as_ref().filter(|_| !state.navigation.dragging()),
                self.brush_key,
                event,
                bounds,
                cursor,
                |point| {
                    Some(crate::brush_stroke::InputFrame::surface(
                        Arc::clone(&self.source),
                        crate::view_navigation::Target::TopDown,
                        self.pick(point, bounds.size()).map(|p| (p, Face::Top)),
                    ))
                },
            ) {
                return Some(action);
            }
            if let Some(action) = state.navigation.update(
                crate::view_navigation::Target::TopDown,
                event,
                bounds,
                cursor,
            ) {
                return Some(action);
            }
            if let (Some(layers), Some(layout)) = (&self.layers, self.layer_layout(bounds.size())) {
                if let Some(action) = layers.update(&state.layers, layout, event, bounds, cursor) {
                    return Some(action);
                }
            }
            if let Some(action) = state.cursor_hover.update(
                event,
                bounds,
                cursor,
                self.brush_key,
                self.source_revision,
                crate::view_navigation::Target::TopDown,
                |point| self.pick(point, bounds.size()),
            ) {
                return Some(action);
            }
            if let canvas::Event::Window(iced::window::Event::RedrawRequested(now)) = event {
                if let Err(error) = self.grid_frame(bounds.size()) {
                    self.lighting.enter(&state.light_report);
                    self.lighting.report(&state.light_report, Err(error));
                }
                if self.grid_cache.pending() {
                    return Some(canvas::Action::request_redraw_at(
                        *now + std::time::Duration::from_millis(16),
                    ));
                }
                if self.show_textures {
                    return self
                        .texture_bank
                        .as_ref()
                        .and_then(|bank| bank.atlas())
                        .and_then(|atlas| atlas.animation_deadline())
                        .map(canvas::Action::request_redraw_at);
                }
            }
            match event {
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    let point = cursor.position_in(bounds)?;
                    let shown = self
                        .grid_cache
                        .presented
                        .lock()
                        .expect("俯视呈现状态锁被毒化")
                        .as_ref()
                        .filter(|shown| shown.viewport.logical == bounds.size())
                        .cloned();
                    if let Some(shown) = shown {
                        match shown.pick(point, self.quality) {
                            Ok(Some(marker)) => {
                                return Some(
                                    canvas::Action::publish(Message::ProjectedEntitySelected(
                                        shown, marker,
                                    ))
                                    .and_capture(),
                                )
                            }
                            Ok(None) => {}
                            Err(error) => {
                                log::error!("[entities] 俯视拾取失败：{error}");
                                return Some(canvas::Action::capture());
                            }
                        }
                    }
                    let xyz = self.pick(point, bounds.size())?;
                    Some(
                        canvas::Action::publish(Message::PickSurfaceFace(
                            xyz.0,
                            xyz.1,
                            xyz.2,
                            Face::Top,
                        ))
                        .and_capture(),
                    )
                }
                canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if self.brush.is_some() => {
                    Some(canvas::Action::request_redraw())
                }
                _ => None,
            }
        })();
        result.or_else(|| {
            self.selection_handle
                .as_ref()
                .and_then(|layer| layer.hover_redraw(event))
        })
    }
    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(interaction) = self.selection_handle.as_ref().and_then(|layer| {
            layer.interaction(
                self.selection_view(bounds.size()),
                cursor.position_in(bounds),
            )
        }) {
            return interaction;
        }

        if state.navigation.dragging() || state.brush.active() {
            return mouse::Interaction::Grabbing;
        }
        if let (Some(layers), Some(layout), Some(point)) = (
            &self.layers,
            self.layer_layout(bounds.size()),
            cursor.position_in(bounds),
        ) {
            if layers.hit(&state.layers, layout, point).is_some() {
                return mouse::Interaction::Pointer;
            }
        }
        if cursor
            .position_in(bounds)
            .and_then(|point| self.pick(point, bounds.size()))
            .is_some()
        {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}
#[cfg(test)]
pub(crate) fn run_test_category(category: crate::test_categories::Category) {
    tests::run_test_category(category);
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{VoxelMaterial, ALL_FACES};
    fn scene() -> SurfaceView {
        SurfaceView {
            biomes: Default::default(),
            x: -16,
            z: 29_999_980,
            width: 16,
            depth: 16,
            y_min: -64,
            y_max: 319,
            missing_columns: 0,
            source_chunks: 1,
            source_tiles: vec![],
            voxels: vec![SurfaceVoxel::new(
                [-16, 100, 29_999_980],
                mcedit2_world::surface::VoxelData {
                    span: 1,
                    fluid: None,
                    state: State {
                        name: "minecraft:stone".into(),
                        properties: vec![],
                    }
                    .into(),
                    biome: None,
                    original_block_entity: false,
                    staged: false,
                    faces: ALL_FACES,
                    kind: VoxelMaterial::Solid,
                    light: Default::default(),
                },
            )],
        }
    }

    fn signed_world_border_projected_coordinates_roundtrip() {
        let s = scene();
        let size = Size::new(800.0, 440.0);
        let p = Projection::new(&s, size, 1.0, 0.0, 0.0).unwrap();
        let q = p.point(-15.5, 29_999_980.5);
        assert_eq!(p.cell(q), Some((-16, 29_999_980)));
    }

    fn far_map_footprints_stay_world_aligned() {
        let mut s = scene();
        s.width = 128;
        s.depth = 128;
        s.voxels[0].span = 16;
        let p = Projection::new(&s, Size::new(800.0, 800.0), 1.0, 0.0, 0.0).unwrap();
        let start = p.point(s.voxels[0].x as f64, s.voxels[0].z as f64);
        let end = p.point(s.voxels[0].x as f64 + 16.0, s.voxels[0].z as f64 + 16.0);
        assert!(((end.x - start.x) - p.scale * 16.0).abs() < 0.01);
        assert!(((end.y - start.y) - p.scale * 16.0).abs() < 0.01);
    }

    fn exact_picks_never_promote_sparse_far_samples() {
        let mut s = scene();
        let mut lower = s.voxels[0].clone();
        lower.y = 80;
        s.voxels.push(lower);
        let view = TopDown {
            selection_handle: None,
            chunk_column: None,
            brush: None,
            brush_key: None,
            layers: None,
            source: Arc::new(s),
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            selected: None,
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            entity_frame: None,
            grid_cache: Default::default(),
            lighting: crate::lighting::Table::unlit(),
            quality: Default::default(),
            source_revision: 0,
        };
        let size = Size::new(800.0, 440.0);
        let p = Projection::new(&view.source, size, 1.0, 0.0, 0.0)
            .unwrap()
            .point(-15.5, 29_999_980.5);
        assert_eq!(view.pick(p, size), Some((-16, 100, 29_999_980)));
        let mut far = (*view.source).clone();
        far.width = 8192;
        far.depth = 8192;
        far.voxels[0].span = 16;
        let far_view = TopDown {
            selection_handle: None,
            chunk_column: None,
            brush: None,
            brush_key: None,
            layers: None,
            source: Arc::new(far),
            ..view
        };
        assert!(far_view.pick(Point::new(400.0, 220.0), size).is_none());
    }

    fn anchored_zoom_and_pan_keep_the_same_original_top_down_pick_at_world_border() {
        use crate::view_navigation::{Action, Viewport};
        let mut view = TopDown {
            selection_handle: None,
            chunk_column: None,
            brush: None,
            brush_key: None,
            layers: None,
            source: Arc::new(scene()),
            zoom: 1.,
            pan_x: 0.,
            pan_y: 0.,
            selected: None,
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            entity_frame: None,
            grid_cache: Default::default(),
            lighting: crate::lighting::Table::unlit(),
            quality: Default::default(),
            source_revision: 0,
        };
        let size = Size::new(800., 440.);
        let original = view.source.voxels[0].clone();
        let center = Point::new(size.width * 0.5, size.height * 0.5);
        let anchor = Projection::new(&view.source, size, 1., 0., 0.)
            .unwrap()
            .point(original.x as f64 + 0.5, original.z as f64 + 0.5);
        let mut nav = Viewport::default();
        for steps in [5., -3., 12., -6.] {
            nav.apply(Action::Zoom {
                steps,
                anchor,
                center,
            })
            .unwrap();
            view.zoom = nav.zoom;
            view.pan_x = nav.pan.x;
            view.pan_y = nav.pan.y;
            assert_eq!(
                view.pick(anchor, size),
                Some((original.x, original.y, original.z))
            );
        }
        let delta = iced::Vector::new(17., 23.);
        nav.apply(Action::Pan(delta)).unwrap();
        view.pan_x = nav.pan.x;
        view.pan_y = nav.pan.y;
        assert_eq!(
            view.pick(anchor + delta, size),
            Some((original.x, original.y, original.z))
        );
    }

    #[cfg(test)]
    pub(crate) fn run_test_category(category: crate::test_categories::Category) {
        if category == crate::test_categories::Category::Navigation {
            signed_world_border_projected_coordinates_roundtrip();
            far_map_footprints_stay_world_aligned();
            exact_picks_never_promote_sparse_far_samples();
            anchored_zoom_and_pan_keep_the_same_original_top_down_pick_at_world_border();
        }
    }
}
