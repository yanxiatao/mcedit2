//! Read-only, interactive 2D horizontal / vertical Java world slice maps.
//! An absent chunk/Section is never shown as source air. All positions are
//! derived from the authenticated Slice/VerticalSlice produced by the backend.
use crate::{isometric::Face, palette, textures::TextureBank, Message};
use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{mouse, Color, Point, Rectangle, Renderer, Size, Theme};
use mcedit2_world::editing::{Slice, SliceCell, VerticalSlice};
use mcedit2_world::surface::{material, VoxelMaterial};

#[derive(Clone, Copy)]
pub(crate) enum Plane<'a> {
    Horizontal(&'a Slice),
    Vertical(&'a VerticalSlice),
}
pub(crate) struct Map2D<'a> {
    pub plane: Plane<'a>,
    pub selected: Option<(i32, i32, i32)>,
    /// The same read-only resource atlas used by the 3D source renderer.
    pub texture_bank: Option<&'a TextureBank>,
    pub show_textures: bool,
}
#[derive(Clone, Copy, Debug)]
struct GridLayout {
    origin: Point,
    cell: f32,
    rows: usize,
    cols: usize,
}
impl GridLayout {
    fn new(size: Size, rows: usize, cols: usize) -> Option<Self> {
        if rows == 0 || cols == 0 || rows > 64 || cols > 64 {
            return None;
        }
        if size.width <= 0.0 || size.height <= 0.0 {
            return None;
        }
        let cell = (size.width / cols as f32)
            .min(size.height / rows as f32)
            .min(28.0);
        if !cell.is_finite() || cell <= 0.0 {
            return None;
        }
        let w = cell * cols as f32;
        let h = cell * rows as f32;
        Some(Self {
            origin: Point::new((size.width - w) * 0.5, (size.height - h) * 0.5),
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
impl Map2D<'_> {
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
    fn hit(&self, p: Point, size: Size) -> Option<(usize, usize)> {
        let (rows, cols) = self.dims();
        let layout = GridLayout::new(size, rows, cols)?;
        let (row, col) = layout.hit(p)?;
        self.cell(row, col)?;
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
    state: &mcedit2_world::sections::State,
    xyz: (i32, i32, i32),
    at: Point,
    cell: f32,
    samples: usize,
) {
    let Some((uv, _, page)) = bank.atlas_face_style_at(state, Face::Top, 0, xyz) else {
        return;
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
                    r as f32 / 255.0,
                    g as f32 / 255.0,
                    b as f32 / 255.0,
                    a as f32 / 255.0,
                ),
            );
        }
    }
}
impl canvas::Program<Message> for Map2D<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (rows, cols) = self.dims();
        if let Some(grid) = GridLayout::new(bounds.size(), rows, cols) {
            let samples = texture_samples(grid.cell, rows * cols);
            let atlas = if self.show_textures {
                self.texture_bank
            } else {
                None
            };
            for row in 0..rows {
                for col in 0..cols {
                    let Some((cell, xyz)) = self.cell(row, col) else {
                        continue;
                    };
                    let at = grid.at(row, col);
                    let rect = Path::rectangle(at, Size::new(grid.cell, grid.cell));
                    frame.fill(&rect, fill_color(cell));
                    // Draw a bounded top-face texture preview over the
                    // original-state color. A textureless/modded block still
                    // has its stable palette color, never fabricated texture.
                    if let (Some(bank), SliceCell::Present { state, .. }) = (atlas, cell) {
                        if !matches!(
                            state.name.as_str(),
                            "minecraft:air"
                                | "minecraft:cave_air"
                                | "minecraft:void_air"
                                | "numeric:0"
                        ) {
                            draw_texture(&mut frame, bank, state, xyz, at, grid.cell, samples);
                        }
                    }
                    if grid.cell >= 5.0 {
                        frame.stroke(
                            &rect,
                            Stroke::default()
                                .with_color(Color::from_rgba(0.06, 0.07, 0.08, 0.45))
                                .with_width(0.7),
                        );
                    }
                    if matches!(cell, SliceCell::MissingSection) && grid.cell >= 7.0 {
                        // Distinguishable from black/dark actual air or an
                        // unindexed/missing chunk, even without the legend.
                        frame.stroke(
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
                        frame.fill(&mark, Color::from_rgb(1.0, 0.72, 0.18));
                    }
                    if self.selected == Some(xyz) {
                        frame.stroke(
                            &rect,
                            Stroke::default()
                                .with_color(Color::from_rgb(0.12, 1.0, 0.9))
                                .with_width(2.2),
                        );
                    }
                }
            }
        }
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        _: &mut (),
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !matches!(
            event,
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) {
            return None;
        }
        let (row, col) = self.hit(cursor.position_in(bounds)?, bounds.size())?;
        let (cell, xyz) = self.cell(row, col)?;
        if !matches!(cell, SliceCell::Present { .. }) {
            return None;
        }
        let message = match self.plane {
            Plane::Horizontal(_) => Message::PickSlice(xyz.0, xyz.1, xyz.2),
            Plane::Vertical(_) => Message::PickVertical(row, col),
        };
        Some(canvas::Action::publish(message).and_capture())
    }
    fn mouse_interaction(
        &self,
        _: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
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
        }
    }
    #[test]
    fn accurate_64_square_grid_selects_edge_without_off_by_one() {
        let grid = GridLayout::new(Size::new(480.0, 420.0), 64, 64).unwrap();
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
            plane: Plane::Horizontal(&slice),
            selected: None,
            texture_bank: None,
            show_textures: false,
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
            plane: Plane::Vertical(&vertical),
            selected: None,
            texture_bank: None,
            show_textures: false,
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
        };
        assert_ne!(fill_color(&air), fill_color(&SliceCell::MissingChunk));
        assert_ne!(fill_color(&air), fill_color(&SliceCell::MissingSection));
        assert_ne!(
            fill_color(&SliceCell::MissingChunk),
            fill_color(&SliceCell::MissingSection)
        );
    }
}
