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
    pub source: Arc<SurfaceView>,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub selected: Option<(i32, i32, i32)>,
    pub texture_bank: Option<Arc<TextureBank>>,
    pub show_textures: bool,
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
        let scale =
            (size.width / v.width as f32).min(size.height / v.depth as f32) * zoom.clamp(0.5, 3.0);
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
fn tint(v: &SurfaceVoxel) -> Color {
    palette::tint(&v.state, v.kind, true).unwrap_or_else(|| palette::unknown_tint(&v.state, v.kind))
}
/// Draw a bounded sample of the same top-face atlas UV used in 3D.
/// Canvas has no shared GPU sampler, so resolution adapts to the screen cell.
fn textured_cell(
    frame: &mut Frame,
    voxel: &SurfaceVoxel,
    origin: Point,
    scale: f32,
    bank: &TextureBank,
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
            let tint = bank.face_tint_with_biome_at(
                &voxel.state,
                Face::Top,
                0,
                (voxel.x, voxel.y, voxel.z),
                voxel.biome.as_deref(),
            );
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
fn exact_visible(v: &SurfaceView) -> BTreeMap<(i32, i32), usize> {
    let mut tops: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    for (index, voxel) in v.voxels.iter().enumerate() {
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
        exact_visible(&self.source)
            .get(&(x, z))
            .map(|&index| (x, self.source.voxels[index].y, z))
    }
}
impl canvas::Program<Message> for TopDown {
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
            for (_, index) in exact_visible(&self.source) {
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
                if !self.show_textures
                    || !self
                        .texture_bank
                        .as_ref()
                        .is_some_and(|bank| textured_cell(&mut frame, voxel, origin, p.scale, bank))
                {
                    frame.fill(&rect, tint(voxel));
                }
                if self.selected == Some((voxel.x, voxel.y, voxel.z)) {
                    frame.stroke(
                        &rect,
                        Stroke::default()
                            .with_width(2.0)
                            .with_color(Color::from_rgb(0.1, 1.0, 0.9)),
                    );
                }
            }
        } else {
            // Rasterize verified source samples in world space. A sampled
            // 4/8/16-cell patch covers its actual extent, not one arbitrary
            // 2-pixel dot. Never synthesize missing chunks or editor picks.
            let mut footprints: BTreeMap<(i32, i32), usize> = BTreeMap::new();
            for (index, voxel) in self.source.voxels.iter().enumerate() {
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
                if !self.show_textures
                    || !self
                        .texture_bank
                        .as_ref()
                        .is_some_and(|bank| textured_cell(&mut frame, voxel, origin, side, bank))
                {
                    frame.fill(&rect, tint(voxel));
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
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let xyz = self.pick(cursor.position_in(bounds)?, bounds.size())?;
                Some(
                    canvas::Action::publish(Message::PickSurface(xyz.0, xyz.1, xyz.2))
                        .and_capture(),
                )
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                cursor.position_in(bounds)?;
                let dy = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => y,
                    mouse::ScrollDelta::Pixels { y, .. } => y / 80.0,
                };
                if dy.abs() < 0.05 {
                    return None;
                }
                Some(
                    canvas::Action::publish(Message::ZoomSurface(if dy > 0.0 { 1 } else { -1 }))
                        .and_capture(),
                )
            }
            _ => None,
        }
    }
    fn mouse_interaction(
        &self,
        _: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
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
mod tests {
    use super::*;
    use mcedit2_world::sections::State;
    use mcedit2_world::surface::{VoxelMaterial, ALL_FACES};
    fn scene() -> SurfaceView {
        SurfaceView {
            x: -16,
            z: 29_999_980,
            width: 16,
            depth: 16,
            y_min: -64,
            y_max: 319,
            missing_columns: 0,
            source_chunks: 1,
            source_tiles: vec![],
            voxels: vec![SurfaceVoxel {
                x: -16,
                y: 100,
                z: 29_999_980,
                span: 1,
                state: State {
                    name: "minecraft:stone".into(),
                    properties: vec![],
                },
                biome: None,
                original_block_entity: false,
                staged: false,
                faces: ALL_FACES,
                kind: VoxelMaterial::Solid,
            }],
        }
    }
    #[test]
    fn signed_world_border_projected_coordinates_roundtrip() {
        let s = scene();
        let size = Size::new(800.0, 440.0);
        let p = Projection::new(&s, size, 1.0, 0.0, 0.0).unwrap();
        let q = p.point(-15.5, 29_999_980.5);
        assert_eq!(p.cell(q), Some((-16, 29_999_980)));
    }
    #[test]
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
    #[test]
    fn exact_picks_never_promote_sparse_far_samples() {
        let mut s = scene();
        let mut lower = s.voxels[0].clone();
        lower.y = 80;
        s.voxels.push(lower);
        let view = TopDown {
            source: Arc::new(s),
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            selected: None,
            texture_bank: None,
            show_textures: false,
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
            source: Arc::new(far),
            ..view
        };
        assert!(far_view.pick(Point::new(400.0, 220.0), size).is_none());
    }
}
