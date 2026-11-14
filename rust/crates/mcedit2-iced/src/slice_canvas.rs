//! Read-only, interactive 2D horizontal / vertical Java world slice maps.
//! An absent chunk/Section is never shown as source air. All positions are
//! derived from the authenticated Slice/VerticalSlice produced by the backend.
use crate::{isometric::Face, palette, textures::TextureBank, Message};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use mcedit2_world::editing::{Slice, SliceCell, VerticalAxis, VerticalSlice};
use mcedit2_world::surface::{material, VoxelMaterial};

#[derive(Clone, Copy)]
pub(crate) enum Plane<'a> {
    Horizontal(&'a Slice),
    Vertical(&'a VerticalSlice),
}
impl Plane<'_> {
    /// 地形、实体、拾取与坐标标尺共用同一格网投影。
    pub(crate) fn projection(
        self,
        size: Size,
        navigation: crate::view_navigation::Viewport,
    ) -> Option<(crate::projected_entities::Mapping, ([f64; 3], [f64; 3]))> {
        use crate::projected_entities::Mapping;
        let (rows, cols) = match self {
            Self::Horizontal(slice) => (slice.rows.len(), slice.rows.first().map_or(0, Vec::len)),
            Self::Vertical(slice) => (slice.rows.len(), slice.rows.first().map_or(0, Vec::len)),
        };
        let layout = GridLayout::new(size, rows, cols, navigation)?;
        let (bounds, origin, right, down, depth) = match self {
            Self::Horizontal(slice) => (
                slice.bounds,
                [
                    slice.bounds.minimum[0] as f64 - 0.5,
                    slice.bounds.minimum[1] as f64,
                    slice.bounds.minimum[2] as f64 - 0.5,
                ],
                [1., 0., 0.],
                [0., 0., 1.],
                [0., 1., 0.],
            ),
            Self::Vertical(slice) => {
                let b = slice.bounds;
                match slice.axis {
                    VerticalAxis::AlongX => (
                        b,
                        [
                            b.minimum[0] as f64 - 0.5,
                            b.maximum[1] as f64 + 0.5,
                            b.minimum[2] as f64,
                        ],
                        [1., 0., 0.],
                        [0., -1., 0.],
                        [0., 0., -1.],
                    ),
                    VerticalAxis::AlongZ => (
                        b,
                        [
                            b.minimum[0] as f64,
                            b.maximum[1] as f64 + 0.5,
                            b.minimum[2] as f64 - 0.5,
                        ],
                        [0., 0., 1.],
                        [0., -1., 0.],
                        [1., 0., 0.],
                    ),
                }
            }
        };
        let clip = (
            bounds.minimum.map(|n| f64::from(n) - 0.5),
            bounds.maximum.map(|n| f64::from(n) + 0.5),
        );
        Some((
            Mapping::Orthogonal {
                origin,
                screen: [layout.origin.x, layout.origin.y],
                right,
                down,
                depth,
                scale: f64::from(layout.cell),
            },
            clip,
        ))
    }
    fn face(self) -> Face {
        match self {
            Self::Horizontal(_) => Face::Top,
            Self::Vertical(v) => match v.axis {
                VerticalAxis::AlongX => Face::NegZ,
                VerticalAxis::AlongZ => Face::PosX,
            },
        }
    }
}
#[derive(Clone)]
pub(crate) struct Map2D<'a> {
    pub(crate) selection_handle: Option<crate::selection_handle::Layer>,
    pub brush: Option<crate::brush_stroke::Config>,
    pub brush_key: Option<(usize, usize)>,
    pub brush_source: Option<crate::brush_stroke::Source>,
    pub layers: Option<crate::scene_layers::View>,
    pub chunk_column: Option<crate::chunk_outline::Column>,
    pub navigation: crate::view_navigation::Viewport,
    pub navigation_target: crate::view_navigation::Target,
    pub plane: Plane<'a>,
    pub selected: Option<(i32, i32, i32)>,
    /// The same read-only resource atlas used by the 3D source renderer.
    pub texture_bank: Option<&'a TextureBank>,
    pub show_textures: bool,
    pub(crate) block_layers: crate::block_visibility::Layers,
    pub entity_frame: Option<std::sync::Arc<crate::entity_geometry::Frame>>,
    pub grid_cache: std::sync::Arc<crate::projected_entities::GridCache>,
    pub lighting: std::sync::Arc<crate::lighting::Table>,
    pub quality: crate::gpu_mesh::Quality,
    pub source_revision: u64,
}
#[derive(Clone, Copy, Debug)]
struct GridLayout {
    origin: Point,
    cell: f32,
    rows: usize,
    cols: usize,
}
impl GridLayout {
    fn new(
        size: Size,
        rows: usize,
        cols: usize,
        navigation: crate::view_navigation::Viewport,
    ) -> Option<Self> {
        if rows == 0 || cols == 0 || rows > 64 || cols > 64 {
            return None;
        }
        if size.width <= 0.0 || size.height <= 0.0 {
            return None;
        }
        let cell = (size.width / cols as f32)
            .min(size.height / rows as f32)
            .min(28.0)
            * navigation.zoom;
        if !cell.is_finite() || cell <= 0.0 {
            return None;
        }
        let w = cell * cols as f32;
        let h = cell * rows as f32;
        Some(Self {
            origin: Point::new(
                (size.width - w) * 0.5 + navigation.pan.x,
                (size.height - h) * 0.5 + navigation.pan.y,
            ),
            cell,
            rows,
            cols,
        })
    }
    fn hit(&self, p: Point) -> Option<(usize, usize)> {
        let x = p.x - self.origin.x;
        let y = p.y - self.origin.y;
        if x < 0.0
            || y < 0.0
            || x >= self.cell * self.cols as f32
            || y >= self.cell * self.rows as f32
        {
            return None;
        }
        let col = (x / self.cell).floor() as usize;
        let row = (y / self.cell).floor() as usize;
        if row >= self.rows || col >= self.cols {
            return None;
        }
        Some((row, col))
    }
    fn at(&self, row: usize, col: usize) -> Point {
        Point::new(
            self.origin.x + col as f32 * self.cell,
            self.origin.y + row as f32 * self.cell,
        )
    }
}
impl<'a> Map2D<'a> {
    fn selection_view(&self, size: Size) -> Option<crate::selection_handle::View> {
        let (mapping, clip) = self.entity_projection(size)?;
        Some(crate::selection_handle::View::Planar(mapping, size, clip))
    }
    pub(crate) fn element(
        self,
        app: &crate::App,
        height: iced::Length,
    ) -> iced::Element<'a, Message> {
        let picker = self.clone();
        let projection = self.clone();
        let target = self.navigation_target;
        crate::library::scene_target(
            app,
            target,
            iced::widget::canvas(self)
                .width(iced::Length::Fill)
                .height(height)
                .into(),
            move |point, size| picker.brush_frame(point, size),
            move |size| projection.selection_view(size),
        )
    }
    fn brush_frame(&self, point: Point, size: Size) -> Option<crate::brush_stroke::InputFrame> {
        use crate::{
            brush_stroke::{InputFrame, Sample},
            picking::Owner,
        };
        let source = self.brush_source.clone()?;
        let sample = self
            .hit(point, size)
            .map(|(row, col)| {
                let (cell, (x, y, z)) = self.cell(row, col).expect("切片命中坐标已经校验");
                match cell {
                    SliceCell::Present { state, .. } => Ok(Sample::Block {
                        owner: Owner {
                            position: [x, y, z],
                            state: std::sync::Arc::new(state.clone()),
                        },
                        face: self.plane.face(),
                    }),
                    SliceCell::MissingChunk => Err(format!("切片目标 ({x},{y},{z}) 缺少源区块")),
                    SliceCell::MissingSection => Err(format!("切片目标 ({x},{y},{z}) 缺少源区段")),
                }
            })
            .transpose();
        Some(InputFrame { source, sample })
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
            std::sync::Arc<crate::projected_entities::Rendered>,
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
        let Some(base) = self.texture_bank.and_then(|bank| bank.atlas()) else {
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
            frame: std::sync::Arc::clone(frame),
            mapping,
            clip,
            size,
            quality: self.quality,
            source_revision: self.source_revision,
            lighting: std::sync::Arc::clone(&self.lighting),
        };
        if self.grid_cache.needs(&key) {
            let Some(terrain) = self.terrain_vertices(size, &frame.atlas) else {
                return Ok(None);
            };
            self.grid_cache.request(key.clone(), terrain);
        }
        if let Some(error) = self.grid_cache.error(&key) {
            return Err(error);
        }
        Ok(self.grid_cache.ready(&key))
    }
    /// 将已认证的格网源面提供给同深度 CPU/GPU 合成；缺失和空气不伪造遮挡。
    pub(crate) fn terrain_vertices(
        &self,
        size: Size,
        atlas: &crate::textures::TextureAtlas,
    ) -> Option<Vec<crate::gpu_mesh::Vertex>> {
        let (rows, cols) = self.dims();
        let layout = GridLayout::new(size, rows, cols, self.navigation)?;
        let (mapping, clip) = self.entity_projection(size)?;
        let mut out = Vec::new();
        for row in 0..rows {
            for col in 0..cols {
                let (cell, xyz) = self.cell(row, col)?;
                if !self.cell_visible(cell) {
                    continue;
                }
                let SliceCell::Present { state, .. } = cell else {
                    continue;
                };
                if matches!(
                    state.name.as_str(),
                    "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "numeric:0"
                ) {
                    continue;
                }
                // 此地形只随同一 GridKey 的完整实体光栅发布；未完成时 draw 仍画原格网。
                if self
                    .entity_frame
                    .as_ref()
                    .zip(self.texture_bank)
                    .is_some_and(|(frame, bank)| {
                        frame
                            .block_visual_faces([xyz.0, xyz.1, xyz.2], state, bank, atlas)
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
                let at = layout.at(row, col);
                let d = layout.cell;
                let points = [
                    at,
                    Point::new(at.x + d, at.y),
                    Point::new(at.x + d, at.y + d),
                    Point::new(at.x, at.y + d),
                ];
                let fallback = fill_color(cell);
                let mut rgba = [fallback.r, fallback.g, fallback.b, 1.];
                let mut uv = [atlas.white.sample(0.5, 0.5); 4];
                let mut page = atlas.white.page;
                let mut mode = 0;
                if self.show_textures {
                    if let Some(bank) = self.texture_bank {
                        if let Some((sample, alpha, p)) =
                            bank.atlas_face_style_at(state, self.plane.face(), 0, xyz)
                        {
                            uv = match self.plane.face() {
                                Face::NegZ => [sample[3], sample[2], sample[1], sample[0]],
                                Face::PosX => [sample[1], sample[2], sample[3], sample[0]],
                                _ => sample,
                            };
                            page = p;
                            mode = alpha.gpu_bits();
                            rgba = [1.; 4];
                        }
                    }
                }
                crate::projected_entities::grid_face(
                    &mut out,
                    points,
                    0.5,
                    rgba,
                    uv,
                    mode,
                    page,
                    crate::lighting::FULLBRIGHT,
                );
            }
        }
        Some(out)
    }
    /// 与当前格网的导航完全一致；范围沿切片轴保留一格厚度。
    pub(crate) fn entity_projection(
        &self,
        size: Size,
    ) -> Option<(crate::projected_entities::Mapping, ([f64; 3], [f64; 3]))> {
        self.plane.projection(size, self.navigation)
    }
    fn dims(&self) -> (usize, usize) {
        match self.plane {
            Plane::Horizontal(slice) => (slice.rows.len(), slice.rows.first().map_or(0, Vec::len)),
            Plane::Vertical(slice) => (slice.rows.len(), slice.rows.first().map_or(0, Vec::len)),
        }
    }
    fn cell(&self, row: usize, col: usize) -> Option<(&SliceCell, (i32, i32, i32))> {
        match self.plane {
            Plane::Horizontal(slice) => {
                let cell = slice.rows.get(row)?.get(col)?;
                let x = slice.bounds.minimum[0].checked_add(col as i32)?;
                let z = slice.bounds.minimum[2].checked_add(row as i32)?;
                Some((cell, (x, slice.bounds.minimum[1], z)))
            }
            Plane::Vertical(slice) => {
                let cell = slice.rows.get(row)?.get(col)?;
                Some((cell, slice.coordinate(row, col)?))
            }
        }
    }
    fn cell_visible(&self, cell: &SliceCell) -> bool {
        match cell {
            SliceCell::Present { state, .. } => self.block_layers.visible(state, self.texture_bank),
            _ => true,
        }
    }
    fn hit(&self, p: Point, size: Size) -> Option<(usize, usize)> {
        let (rows, cols) = self.dims();
        let layout = GridLayout::new(size, rows, cols, self.navigation)?;
        let (row, col) = layout.hit(p)?;
        let (cell, _) = self.cell(row, col)?;
        if !self.cell_visible(cell) {
            return None;
        }
        Some((row, col))
    }
}
fn fill_color(cell: &SliceCell) -> Color {
    match cell {
        SliceCell::MissingChunk => Color::from_rgb(0.16, 0.13, 0.20),
        SliceCell::MissingSection => Color::from_rgb(0.31, 0.23, 0.12),
        SliceCell::Present { state, .. } => {
            if matches!(
                state.name.as_str(),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "numeric:0"
            ) {
                return Color::from_rgb(0.20, 0.22, 0.25);
            }
            let kind = material(state);
            let mut color = palette::tint(state, kind, true)
                .unwrap_or_else(|| palette::unknown_tint(state, kind));
            // A 2D map is a single opaque slice; retain a readable color
            // even when the 3D material represents translucent foliage/glass.
            if kind != VoxelMaterial::Solid {
                color.a = 1.0;
            }
            color
        }
    }
}
/// UV corners use the same [top-left, top-right, bottom-right, bottom-left]
/// convention as the existing 3D source face. Do not silently ignore
/// resource-pack UV rotation/cropping or blockstate model variants.
fn sample_uv(uv: [[f32; 2]; 4], u: f32, v: f32) -> [f32; 2] {
    let a = uv[0];
    let b = uv[1];
    let c = uv[2];
    let d = uv[3];
    [
        a[0] * (1.0 - u) * (1.0 - v) + b[0] * u * (1.0 - v) + c[0] * u * v + d[0] * (1.0 - u) * v,
        a[1] * (1.0 - u) * (1.0 - v) + b[1] * u * (1.0 - v) + c[1] * u * v + d[1] * (1.0 - u) * v,
    ]
}
/// Bounded 2D texture preview quality. Sparse maps have more pixels per
/// source block, while 64x64 maps must not rasterize millions of quads.
fn texture_samples(cell: f32, source_cells: usize) -> usize {
    let budget = if source_cells > 1024 {
        2
    } else if source_cells > 256 {
        4
    } else {
        6
    };
    ((cell / 4.0).floor() as usize).clamp(1, budget)
}
fn draw_texture(
    frame: &mut Frame,
    bank: &TextureBank,
    source: (&mcedit2_world::sections::State, Face),
    xyz: (i32, i32, i32),
    at: Point,
    cell: f32,
    samples: usize,
    light: [f32; 3],
) {
    let (state, face) = source;
    let Some((uv, _, page)) = bank.atlas_face_style_at(state, face, 0, xyz) else {
        return;
    };
    let uv = match face {
        Face::NegZ => [uv[3], uv[2], uv[1], uv[0]],
        Face::PosX => [uv[1], uv[2], uv[3], uv[0]],
        _ => uv,
    };
    let step = cell / samples as f32;
    for iy in 0..samples {
        for ix in 0..samples {
            let u = (ix as f32 + 0.5) / samples as f32;
            let v = (iy as f32 + 0.5) / samples as f32;
            let Some([r, g, b, a]) = bank.atlas_pixel(sample_uv(uv, u, v), page) else {
                continue;
            };
            if a == 0 {
                continue;
            }
            frame.fill(
                &Path::rectangle(
                    Point::new(at.x + ix as f32 * step, at.y + iy as f32 * step),
                    Size::new(step + 0.05, step + 0.05),
                ),
                Color::from_rgba(
                    r as f32 / 255.0 * light[0],
                    g as f32 / 255.0 * light[1],
                    b as f32 / 255.0 * light[2],
                    a as f32 / 255.0,
                ),
            );
        }
    }
}
impl canvas::Program<Message> for Map2D<'_> {
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
        let (rows, cols) = self.dims();
        if let Some(grid) = GridLayout::new(bounds.size(), rows, cols, self.navigation) {
            let samples = texture_samples(grid.cell, rows * cols);
            let atlas = if self.show_textures {
                texture_bank
            } else {
                None
            };
            for row in 0..rows {
                for col in 0..cols {
                    let Some((cell, xyz)) = self.cell(row, col) else {
                        continue;
                    };
                    if !self.cell_visible(cell) {
                        continue;
                    }
                    let at = grid.at(row, col);
                    if at.x + grid.cell < 0.
                        || at.y + grid.cell < 0.
                        || at.x > bounds.width
                        || at.y > bounds.height
                    {
                        continue;
                    }
                    let rect = Path::rectangle(at, Size::new(grid.cell, grid.cell));
                    let present_solid = matches!(cell, SliceCell::Present {state,..} if !matches!(state.name.as_str(),"minecraft:air"|"minecraft:cave_air"|"minecraft:void_air"|"numeric:0"));
                    let light = [1.; 3];
                    if combined.is_none() || !present_solid {
                        frame.fill(&rect, crate::lighting::shade(fill_color(cell), light));
                    }
                    // Draw a bounded top-face texture preview over the
                    // original-state color. A textureless/modded block still
                    // has its stable palette color, never fabricated texture.
                    if let (None, Some(bank), SliceCell::Present { state, .. }) =
                        (&combined, atlas, cell)
                    {
                        if !matches!(
                            state.name.as_str(),
                            "minecraft:air"
                                | "minecraft:cave_air"
                                | "minecraft:void_air"
                                | "numeric:0"
                        ) {
                            draw_texture(
                                &mut frame,
                                bank,
                                (state, self.plane.face()),
                                xyz,
                                at,
                                grid.cell,
                                samples,
                                light,
                            );
                        }
                    }
                    if grid.cell >= 5.0 {
                        foreground.stroke(
                            &rect,
                            Stroke::default()
                                .with_color(Color::from_rgba(0.06, 0.07, 0.08, 0.45))
                                .with_width(0.7),
                        );
                    }
                    if matches!(cell, SliceCell::MissingSection) && grid.cell >= 7.0 {
                        // Distinguishable from black/dark actual air or an
                        // unindexed/missing chunk, even without the legend.
                        foreground.stroke(
                            &Path::line(
                                Point::new(at.x + 2.0, at.y + 2.0),
                                Point::new(at.x + grid.cell - 2.0, at.y + grid.cell - 2.0),
                            ),
                            Stroke::default()
                                .with_color(Color::from_rgb(0.92, 0.66, 0.24))
                                .with_width(1.0),
                        );
                    }
                    if matches!(
                        cell,
                        SliceCell::Present {
                            original_block_entity: true,
                            ..
                        }
                    ) && grid.cell >= 6.0
                    {
                        let mid = grid.cell * 0.5;
                        let side = (grid.cell * 0.27).max(2.0);
                        let mark = Path::rectangle(
                            Point::new(at.x + mid - side * 0.5, at.y + mid - side * 0.5),
                            Size::new(side, side),
                        );
                        foreground.fill(&mark, Color::from_rgb(1.0, 0.72, 0.18));
                    }
                    if self.selected == Some(xyz) {
                        foreground.stroke(
                            &rect,
                            Stroke::default()
                                .with_color(Color::from_rgb(0.12, 1.0, 0.9))
                                .with_width(2.2),
                        );
                    }
                }
            }
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
                .expect("切片呈现状态锁被毒化") = Some(rendered);
        } else {
            *self
                .grid_cache
                .presented
                .lock()
                .expect("切片呈现状态锁被毒化") = None;
        }
        if let Some(grid) = GridLayout::new(bounds.size(), rows, cols, self.navigation) {
            state.brush.draw(
                &mut foreground,
                self.brush.as_ref(),
                cursor.position_in(bounds),
                |p| {
                    let (col, row) = match self.plane {
                        Plane::Horizontal(s) => (
                            p[0] - f64::from(s.bounds.minimum[0]),
                            p[2] - f64::from(s.bounds.minimum[2]),
                        ),
                        Plane::Vertical(s) => (
                            (match s.axis {
                                VerticalAxis::AlongX => p[0] - f64::from(s.bounds.minimum[0]),
                                VerticalAxis::AlongZ => p[2] - f64::from(s.bounds.minimum[2]),
                            }),
                            f64::from(s.bounds.maximum[1]) - p[1],
                        ),
                    };
                    Point::new(
                        grid.origin.x + (col + 0.5) as f32 * grid.cell,
                        grid.origin.y + (row + 0.5) as f32 * grid.cell,
                    )
                },
                |point| self.brush_frame(point, bounds.size()),
                crate::brush_cursor::Projection::Planar(
                    self.entity_projection(bounds.size())
                        .expect("有效切片已有投影")
                        .0,
                ),
                self.navigation_target,
            );
        }
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
                    self.navigation_target,
                    self.selection_view(bounds.size()),
                    event,
                    bounds,
                    cursor,
                    |p| self.brush_frame(p, bounds.size()),
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
                |point| self.brush_frame(point, bounds.size()),
            ) {
                return Some(action);
            }
            if let Some(action) =
                state
                    .navigation
                    .update(self.navigation_target, event, bounds, cursor)
            {
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
                self.navigation_target,
                |point| {
                    let (row, col) = self.hit(point, bounds.size())?;
                    self.cell(row, col).map(|(_, position)| position)
                },
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
                        .and_then(|bank| bank.atlas())
                        .and_then(|atlas| atlas.animation_deadline())
                        .map(canvas::Action::request_redraw_at);
                }
            }
            if !matches!(
                event,
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            ) {
                return (self.brush.is_some()
                    && matches!(
                        event,
                        canvas::Event::Mouse(mouse::Event::CursorMoved { .. })
                    ))
                .then(canvas::Action::request_redraw);
            }
            let point = cursor.position_in(bounds)?;
            let shown = self
                .grid_cache
                .presented
                .lock()
                .expect("切片呈现状态锁被毒化")
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
                        log::error!("[entities] 切片拾取失败：{error}");
                        return Some(canvas::Action::capture());
                    }
                }
            }
            let (row, col) = self.hit(point, bounds.size())?;
            let (cell, xyz) = self.cell(row, col)?;
            if !matches!(cell, SliceCell::Present { .. }) {
                return None;
            }
            let message = match self.plane {
                Plane::Horizontal(_) => Message::PickSlice(xyz.0, xyz.1, xyz.2),
                Plane::Vertical(_) => Message::PickVertical(row, col),
            };
            Some(canvas::Action::publish(message).and_capture())
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
        let valid = cursor
            .position_in(bounds)
            .and_then(|p| self.hit(p, bounds.size()))
            .and_then(|(row, col)| self.cell(row, col))
            .is_some_and(|(cell, _)| matches!(cell, SliceCell::Present { .. }));
        if valid {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcedit2_world::editing::{Bounds, VerticalAxis};
    use mcedit2_world::sections::State;
    fn known() -> SliceCell {
        SliceCell::Present {
            state: State {
                name: "modded:unknown".into(),
                properties: vec![],
            },
            original_block_entity: false,
            staged_creation: false,
            light: Default::default(),
        }
    }

    #[test]
    fn accurate_64_square_grid_selects_edge_without_off_by_one() {
        let grid = GridLayout::new(Size::new(480.0, 420.0), 64, 64, Default::default()).unwrap();
        assert_eq!(
            grid.hit(Point::new(grid.origin.x, grid.origin.y)),
            Some((0, 0))
        );
        let last = grid.at(63, 63);
        assert_eq!(
            grid.hit(Point::new(
                last.x + grid.cell * 0.5,
                last.y + grid.cell * 0.5
            )),
            Some((63, 63))
        );
        assert_eq!(
            grid.hit(Point::new(grid.origin.x + 64.0 * grid.cell, grid.origin.y)),
            None
        );
        assert_eq!(
            grid.hit(Point::new(grid.origin.x - 0.01, grid.origin.y)),
            None
        );
    }

    #[test]
    fn source_xyz_is_preserved_in_both_plane_orientations() {
        let b = Bounds::new((-16, -64, -17), (-15, -64, -16)).unwrap();
        let slice = Slice {
            bounds: b,
            source_chunks: 2,
            present: 4,
            missing_chunks: 0,
            missing_sections: 0,
            block_entities: 0,
            rows: vec![vec![known(), known()], vec![known(), known()]],
        };
        let map = Map2D {
            selection_handle: None,
            chunk_column: None,
            brush: None,
            brush_key: None,
            brush_source: None,
            layers: None,
            plane: Plane::Horizontal(&slice),
            navigation: Default::default(),
            navigation_target: crate::view_navigation::Target::Horizontal,
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
        assert_eq!(map.cell(1, 0).unwrap().1, (-16, -64, -16));
        assert_eq!(map.cell(0, 1).unwrap().1, (-15, -64, -17));
        let b = Bounds::new((-16, -64, -17), (-15, -63, -17)).unwrap();
        let vertical = VerticalSlice {
            bounds: b,
            axis: VerticalAxis::AlongX,
            source_chunks: 1,
            block_entities: 0,
            missing_chunks: 0,
            missing_sections: 0,
            rows: vec![vec![known(), known()], vec![known(), known()]],
        };
        let map = Map2D {
            selection_handle: None,
            chunk_column: None,
            brush: None,
            brush_key: None,
            brush_source: None,
            layers: None,
            plane: Plane::Vertical(&vertical),
            navigation: Default::default(),
            navigation_target: crate::view_navigation::Target::Vertical,
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
        assert_eq!(map.cell(0, 1).unwrap().1, (-15, -63, -17));
        assert_eq!(map.cell(1, 0).unwrap().1, (-16, -64, -17));
    }

    #[test]
    fn uv_corners_and_texture_sampling_are_bounded() {
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert_eq!(sample_uv(uv, 0.0, 0.0), [0.0, 0.0]);
        assert_eq!(sample_uv(uv, 1.0, 1.0), [1.0, 1.0]);
        assert_eq!(sample_uv(uv, 0.5, 0.5), [0.5, 0.5]);
        assert_eq!(texture_samples(7.0, 4096), 1);
        assert_eq!(texture_samples(28.0, 4096), 2);
        assert_eq!(texture_samples(28.0, 64), 6);
        assert!(texture_samples(28.0, 4096).pow(2) * 4096 <= 16384);
    }

    #[test]
    fn absent_source_is_visually_distinct_from_air() {
        let air = SliceCell::Present {
            state: State {
                name: "minecraft:air".into(),
                properties: vec![],
            },
            original_block_entity: false,
            staged_creation: false,
            light: Default::default(),
        };
        assert_ne!(fill_color(&air), fill_color(&SliceCell::MissingChunk));
        assert_ne!(fill_color(&air), fill_color(&SliceCell::MissingSection));
        assert_ne!(
            fill_color(&SliceCell::MissingChunk),
            fill_color(&SliceCell::MissingSection)
        );
    }

    #[test]
    fn all_slice_planes_keep_exact_source_coordinates_after_anchored_zoom_and_pan() {
        use crate::view_navigation::{Action, Target, Viewport};
        let size = Size::new(640., 400.);
        let center = Point::new(320., 200.);
        let horizontal = Slice {
            bounds: Bounds::new((-16, -64, -17), (-15, -64, -16)).unwrap(),
            source_chunks: 1,
            present: 4,
            missing_chunks: 0,
            missing_sections: 0,
            block_entities: 0,
            rows: vec![vec![known(); 2]; 2],
        };
        for axis in [None, Some(VerticalAxis::AlongX), Some(VerticalAxis::AlongZ)] {
            let vertical = VerticalSlice {
                bounds: match axis {
                    Some(VerticalAxis::AlongZ) => {
                        Bounds::new((-16, -64, -17), (-16, -63, -16)).unwrap()
                    }
                    _ => Bounds::new((-16, -64, -17), (-15, -63, -17)).unwrap(),
                },
                axis: axis.unwrap_or(VerticalAxis::AlongX),
                source_chunks: 1,
                block_entities: 0,
                missing_chunks: 0,
                missing_sections: 0,
                rows: vec![vec![known(); 2]; 2],
            };
            let plane = if axis.is_none() {
                Plane::Horizontal(&horizontal)
            } else {
                Plane::Vertical(&vertical)
            };
            let mut map = Map2D {
                selection_handle: None,
                chunk_column: None,
                brush: None,
                brush_key: None,
                brush_source: None,
                layers: None,
                plane,
                selected: None,
                texture_bank: None,
                show_textures: false,
                block_layers: Default::default(),
                entity_frame: None,
                grid_cache: Default::default(),
                lighting: crate::lighting::Table::unlit(),
                quality: Default::default(),
                source_revision: 0,
                navigation: Viewport::default(),
                navigation_target: Target::Horizontal,
            };
            let layout = GridLayout::new(size, 2, 2, map.navigation).unwrap();
            let anchor = layout.at(1, 0) + iced::Vector::new(layout.cell * 0.5, layout.cell * 0.5);
            let expected = map.cell(1, 0).unwrap().1;
            for steps in [4., -2., 8.] {
                map.navigation
                    .apply(Action::Zoom {
                        steps,
                        anchor,
                        center,
                    })
                    .unwrap();
                assert_eq!(map.hit(anchor, size), Some((1, 0)));
                assert_eq!(map.cell(1, 0).unwrap().1, expected);
            }
            let delta = iced::Vector::new(-19., 31.);
            map.navigation.apply(Action::Pan(delta)).unwrap();
            assert_eq!(map.hit(anchor + delta, size), Some((1, 0)));
        }
    }

    #[test]
    fn scrolled_slice_uses_transformed_cursor_for_source_pick_and_wheel() {
        use canvas::Program;
        let slice = Slice {
            bounds: Bounds::new((-16, -64, -17), (-15, -64, -16)).unwrap(),
            source_chunks: 1,
            present: 4,
            missing_chunks: 0,
            missing_sections: 0,
            block_entities: 0,
            rows: vec![vec![known(); 2]; 2],
        };
        let map = Map2D {
            selection_handle: None,
            chunk_column: None,
            brush: None,
            brush_key: None,
            brush_source: None,
            layers: None,
            plane: Plane::Horizontal(&slice),
            selected: None,
            texture_bank: None,
            show_textures: false,
            block_layers: Default::default(),
            entity_frame: None,
            grid_cache: Default::default(),
            lighting: crate::lighting::Table::unlit(),
            quality: Default::default(),
            source_revision: 0,
            navigation: Default::default(),
            navigation_target: crate::view_navigation::Target::Horizontal,
        };
        let bounds = Rectangle {
            x: 20.,
            y: 500.,
            width: 200.,
            height: 100.,
        };
        let cursor = mouse::Cursor::Available(Point::new(100., 550.));
        let mut state = crate::scene_layers::MapState::default();
        map.update(
            &mut state,
            &canvas::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(100., 70.),
            }),
            bounds,
            cursor,
        );
        let click = map
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds,
                cursor,
            )
            .unwrap();
        assert!(matches!(
            click.into_inner().0,
            Some(Message::PickSlice(-16, -64, -16))
        ));
        let wheel = map
            .update(
                &mut state,
                &canvas::Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Lines { x: 0., y: 1. },
                }),
                bounds,
                cursor,
            )
            .unwrap();
        assert!(
            matches!(wheel.into_inner().0,Some(Message::Navigate(_,crate::view_navigation::Action::Zoom{anchor,..})) if anchor==Point::new(80.,50.))
        );
    }
}
