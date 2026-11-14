//! 地图导入的整幅原图 / 色表结果；缩略像素在转换线程生成，画布只叠加分块与选择。
use super::{Action, Message};
use iced::{
    mouse,
    widget::{
        canvas::{self, Frame, Path, Stroke},
        column, image, row, text,
    },
    Color, Element, Length, Point, Rectangle, Size,
};
use mcedit2_world::{map_data, map_edit::Mosaic};
use std::sync::Arc;

const EDGE: u32 = 768;
// 两张总览的上限，计入解码 / 缩放转换的同一内存预算。
pub(super) const MEMORY: usize = 2 * EDGE as usize * EDGE as usize * 4;

#[derive(Debug, Clone)]
pub(crate) struct Preview {
    original_size: [u32; 2],
    source: image::Handle,
    converted: image::Handle,
}
fn thumbnail(
    width: u32,
    height: u32,
    pixel: impl Fn(u32, u32) -> Result<[u8; 4], String>,
    stop: &dyn Fn() -> bool,
) -> Result<image::Handle, String> {
    let edge = width.max(height);
    let w = if edge > EDGE {
        (width as u64 * EDGE as u64 / edge as u64).max(1) as u32
    } else {
        width
    };
    let h = if edge > EDGE {
        (height as u64 * EDGE as u64 / edge as u64).max(1) as u32
    } else {
        height
    };
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        if stop() {
            return Err("地图拼图预览已取消".into());
        }
        let sy = (y as u64 * height as u64 / h as u64) as u32;
        for x in 0..w {
            let sx = (x as u64 * width as u64 / w as u64) as u32;
            rgba.extend_from_slice(&pixel(sx, sy)?);
        }
    }
    Ok(image::Handle::from_rgba(w, h, rgba))
}
impl Preview {
    /// 输入正是此次量化使用的 RGBA，包含实际缩放 / 留白，不从路径另读图片。
    pub(super) fn new(
        mosaic: &Mosaic,
        rgba: &[u8],
        original_size: [u32; 2],
        stop: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        let [columns, rows] = grid(mosaic);
        let source = thumbnail(
            mosaic.width,
            mosaic.height,
            |x, y| {
                let offset = (y as usize * mosaic.width as usize + x as usize) * 4;
                Ok(rgba[offset..offset + 4]
                    .try_into()
                    .expect("量化输入为 RGBA"))
            },
            stop,
        )?;
        let palette: Vec<_> = (0..=255u8)
            .map(|code| map_data::color_for_version(mosaic.data_version, code))
            .collect();
        let converted = thumbnail(
            columns * 128,
            rows * 128,
            |x, y| {
                let tile = &mosaic.tiles[(y / 128 * columns + x / 128) as usize];
                let code = tile.colors[((y % 128) * 128 + x % 128) as usize] as u8;
                palette[code as usize].ok_or_else(|| format!("地图色表没有颜色 {code}"))
            },
            stop,
        )?;
        Ok(Self {
            original_size,
            source,
            converted,
        })
    }
}
fn grid(mosaic: &Mosaic) -> [u32; 2] {
    [mosaic.width.div_ceil(128), mosaic.height.div_ceil(128)]
}
fn fit(size: Size, mosaic: &Mosaic) -> Rectangle {
    let [columns, rows] = grid(mosaic);
    let scale = (size.width / columns as f32).min(size.height / rows as f32);
    let width = columns as f32 * scale;
    let height = rows as f32 * scale;
    Rectangle {
        x: (size.width - width) * 0.5,
        y: (size.height - height) * 0.5,
        width,
        height,
    }
}
pub(super) fn view<'a>(
    mosaic: &'a Arc<Mosaic>,
    preview: &'a Preview,
    selected: Option<usize>,
) -> Element<'a, Message> {
    let [columns, rows] = grid(mosaic);
    let pane = |source| {
        canvas::Canvas::new(Overview {
            mosaic,
            preview,
            selected,
            source,
        })
        .width(Length::Fill)
        .height(260)
    };
    column![
        text(format!(
            "原图 {}×{} · 拼图 {columns} 列 × {rows} 行 · 点击任一块查看",
            preview.original_size[0], preview.original_size[1]
        )),
        row![
            column![text("原图与分块"), pane(true)].width(Length::Fill),
            column![text("地图颜色与排布"), pane(false)].width(Length::Fill),
        ]
        .spacing(10),
    ]
    .spacing(6)
    .into()
}
struct Overview<'a> {
    mosaic: &'a Arc<Mosaic>,
    preview: &'a Preview,
    selected: Option<usize>,
    source: bool,
}
impl Overview<'_> {
    fn hit(&self, bounds: Rectangle, cursor: mouse::Cursor) -> Option<usize> {
        let p = cursor.position_in(bounds)?;
        let rect = fit(bounds.size(), self.mosaic);
        if p.x < rect.x || p.y < rect.y || p.x >= rect.x + rect.width || p.y >= rect.y + rect.height
        {
            return None;
        }
        let [columns, rows] = grid(self.mosaic);
        let x = (((p.x - rect.x) / rect.width * columns as f32) as u32).min(columns - 1);
        let y = (((p.y - rect.y) / rect.height * rows as f32) as u32).min(rows - 1);
        Some((y * columns + x) as usize)
    }
}
impl canvas::Program<Message> for Overview<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        theme: &iced::Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let rect = fit(bounds.size(), self.mosaic);
        let colors = theme.extended_palette();
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors.background.weak.color);
        // 透明 / 空白部分用固定棋盘显示，两幅图采用同一块坐标。
        let cell = 12.;
        for y in 0..(rect.height / cell).ceil() as u32 {
            for x in 0..(rect.width / cell).ceil() as u32 {
                frame.fill_rectangle(
                    Point::new(rect.x + x as f32 * cell, rect.y + y as f32 * cell),
                    Size::new(
                        cell.min(rect.width - x as f32 * cell),
                        cell.min(rect.height - y as f32 * cell),
                    ),
                    if (x + y) % 2 == 0 {
                        Color::from_rgb(0.25, 0.25, 0.25)
                    } else {
                        Color::from_rgb(0.34, 0.34, 0.34)
                    },
                );
            }
        }
        let [columns, rows] = grid(self.mosaic);
        let mut image_rect = rect;
        let handle = if self.source {
            image_rect.width *= self.mosaic.width as f32 / (columns * 128) as f32;
            image_rect.height *= self.mosaic.height as f32 / (rows * 128) as f32;
            &self.preview.source
        } else {
            &self.preview.converted
        };
        frame.draw_image(
            image_rect,
            iced::advanced::image::Image::new(handle.clone())
                .filter_method(iced::advanced::image::FilterMethod::Nearest),
        );
        let stroke = Stroke::default().with_color(Color::from_rgba(0., 0., 0., 0.65));
        for x in 0..=columns {
            let px = rect.x + rect.width * x as f32 / columns as f32;
            frame.stroke(
                &Path::line(Point::new(px, rect.y), Point::new(px, rect.y + rect.height)),
                stroke,
            );
        }
        for y in 0..=rows {
            let py = rect.y + rect.height * y as f32 / rows as f32;
            frame.stroke(
                &Path::line(Point::new(rect.x, py), Point::new(rect.x + rect.width, py)),
                stroke,
            );
        }
        if let Some(tile) = self.selected.and_then(|index| self.mosaic.tiles.get(index)) {
            frame.stroke(
                &Path::rectangle(
                    Point::new(
                        rect.x + rect.width * tile.column as f32 / columns as f32,
                        rect.y + rect.height * tile.row as f32 / rows as f32,
                    ),
                    Size::new(rect.width / columns as f32, rect.height / rows as f32),
                ),
                Stroke::default()
                    .with_width(3.)
                    .with_color(colors.primary.strong.color),
            );
        }
        vec![frame.into_geometry()]
    }
    fn update(
        &self,
        _: &mut (),
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if matches!(
            event,
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) {
            let index = self.hit(bounds, cursor)?;
            return Some(
                canvas::Action::publish(Message::MapEditor(Action::MosaicTile(
                    Arc::clone(self.mosaic),
                    index,
                )))
                .and_capture(),
            );
        }
        None
    }
    fn mouse_interaction(
        &self,
        _: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.hit(bounds, cursor).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}
