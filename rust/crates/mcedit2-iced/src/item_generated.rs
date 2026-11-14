//! 生成物品的通用薄片：实际静态像素或全部有效动画帧，按原版边界跨度构造侧面。
use crate::{
    isometric::Face,
    models::{Box3, Part},
    resource_models::items::{Quad, Scene},
    textures::{texture_animation::Sprite, TextureBank, Tile},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Default, Debug)]
pub(crate) struct Cache(
    Mutex<
        Option<(
            crate::textures::VisibilityIdentity,
            BTreeMap<String, Arc<Vec<Span>>>,
        )>,
    >,
);
impl Cache {
    fn get(
        &self,
        bank: &TextureBank,
        texture: &str,
        pixels: &Pixels<'_>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Arc<Vec<Span>>, String> {
        let identity = bank.visibility_id();
        if let Some(spans) = self
            .0
            .lock()
            .expect("生成物品轮廓缓存锁被毒化")
            .as_ref()
            .filter(|(source, _)| *source == identity)
            .and_then(|(_, items)| items.get(texture))
            .cloned()
        {
            return Ok(spans);
        }
        let spans = Arc::new(spans(pixels, cancelled)?);
        let mut state = self.0.lock().expect("生成物品轮廓发布锁被毒化");
        if state.as_ref().is_none_or(|(source, _)| *source != identity) {
            *state = Some((identity, BTreeMap::new()));
        }
        Ok(Arc::clone(
            state
                .as_mut()
                .expect("当前资源轮廓缓存已建立")
                .1
                .entry(texture.into())
                .or_insert(spans),
        ))
    }
}

enum Pixels<'a> {
    Static(&'a Tile),
    Animated(&'a Sprite),
}
impl Pixels<'_> {
    fn size(&self) -> [u32; 2] {
        match self {
            Self::Static(tile) => [tile.width, tile.height],
            Self::Animated(sprite) => [sprite.sequence.width, sprite.sequence.height],
        }
    }
    fn frames(&self) -> Box<dyn Iterator<Item = u32> + '_> {
        match self {
            Self::Static(_) => Box::new(std::iter::once(0)),
            Self::Animated(sprite) => sprite.indices(),
        }
    }
    fn opaque(&self, frame: u32, x: i64, y: i64) -> bool {
        let [w, h] = self.size();
        if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
            return false;
        }
        let (tile, x, y) = match self {
            Self::Static(tile) => (*tile, x as usize, y as usize),
            Self::Animated(sprite) => (
                &sprite.source,
                (frame % sprite.sequence.columns * w) as usize + x as usize,
                (frame / sprite.sequence.columns * h) as usize + y as usize,
            ),
        };
        tile.rgba[(y * tile.width as usize + x) * 4 + 3] != 0
    }
}
#[derive(Clone, Copy, Debug)]
struct Span {
    side: usize,
    anchor: u32,
    min: u32,
    max: u32,
}
fn spans(pixels: &Pixels<'_>, cancelled: &dyn Fn() -> bool) -> Result<Vec<Span>, String> {
    let [w, h] = pixels.size();
    let mut rows: [Vec<Option<usize>>; 4] = [
        vec![None; h as usize],
        vec![None; h as usize],
        vec![None; w as usize],
        vec![None; w as usize],
    ];
    let mut spans: Vec<Span> = Vec::new();
    for frame in pixels.frames() {
        for y in 0..h {
            if cancelled() {
                return Err("生成物品轮廓已取消".into());
            }
            for x in 0..w {
                if !pixels.opaque(frame, i64::from(x), i64::from(y)) {
                    continue;
                }
                for (side, (dx, dy)) in [(0, -1), (0, 1), (-1, 0), (1, 0)].into_iter().enumerate() {
                    if pixels.opaque(frame, i64::from(x) + dx, i64::from(y) + dy) {
                        continue;
                    }
                    let (anchor, pos) = if side < 2 { (y, x) } else { (x, y) };
                    if let Some(index) = rows[side][anchor as usize] {
                        let span = &mut spans[index];
                        span.min = span.min.min(pos);
                        span.max = span.max.max(pos);
                    } else {
                        rows[side][anchor as usize] = Some(spans.len());
                        spans.push(Span {
                            side,
                            anchor,
                            min: pos,
                            max: pos,
                        });
                    }
                }
            }
        }
    }
    Ok(spans)
}
fn quad(xyz: [[f32; 3]; 4], uv: [[f32; 2]; 4], texture: &str, index: u32) -> Quad {
    Quad {
        xyz,
        uv,
        texture: texture.into(),
        tint: Some(index),
        shade: false,
        colors: [[1.; 4]; 4],
        winding: -1.,
        cull: true,
        mode: None,
    }
}
impl Span {
    fn face(self, size: [u32; 2], texture: &str, index: u32) -> Result<Quad, String> {
        let [w, h] = size.map(|v| v as f32);
        let (x_scale, y_scale) = (16. / w, 16. / h);
        let (min, max, anchor) = (self.min as f32, self.max as f32 + 1., self.anchor as f32);
        let (from, to, face, uv) = match self.side {
            0 => (
                [min * x_scale, 16. - anchor * y_scale, 7.5],
                [max * x_scale, 16. - anchor * y_scale, 8.5],
                Face::Top,
                [
                    min * x_scale,
                    anchor * y_scale,
                    max * x_scale,
                    (anchor + 1.) * y_scale,
                ],
            ),
            1 => (
                [min * x_scale, 16. - (anchor + 1.) * y_scale, 7.5],
                [max * x_scale, 16. - (anchor + 1.) * y_scale, 8.5],
                Face::Bottom,
                [
                    min * x_scale,
                    anchor * y_scale,
                    max * x_scale,
                    (anchor + 1.) * y_scale,
                ],
            ),
            2 => (
                [anchor * x_scale, 16. - min * y_scale, 7.5],
                [anchor * x_scale, 16. - max * y_scale, 8.5],
                Face::PosX,
                [
                    anchor * x_scale,
                    max * y_scale,
                    (anchor + 1.) * x_scale,
                    min * y_scale,
                ],
            ),
            3 => (
                [(anchor + 1.) * x_scale, 16. - min * y_scale, 7.5],
                [(anchor + 1.) * x_scale, 16. - max * y_scale, 8.5],
                Face::NegX,
                [
                    anchor * x_scale,
                    max * y_scale,
                    (anchor + 1.) * x_scale,
                    min * y_scale,
                ],
            ),
            _ => unreachable!("四种轮廓方向"),
        };
        let part = Part {
            bounds: Box3::from_model_units_plane(from, to).ok_or("生成物品侧面范围无效")?,
            rotation: None,
            variant_x: 0,
            variant_y: 0,
            root: None,
        };
        let corners = [
            [uv[0], uv[1]],
            [uv[2], uv[1]],
            [uv[2], uv[3]],
            [uv[0], uv[3]],
        ]
        .map(|p| p.map(|v| v / 16.));
        let xyz = part.face_vertices(face.corners());
        let mut output = quad(
            xyz,
            face.java_uv_indices().map(|i| corners[i as usize]),
            texture,
            index,
        );
        output.winding = if crate::world_mesh::winding_reversed(&xyz, part.normal(face.normal())) {
            -1.
        } else {
            1.
        };
        Ok(output)
    }
}
pub(crate) fn append(
    bank: &TextureBank,
    scene: &mut Scene,
    world: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    for (texture, index) in &scene.layers {
        let pixels = match bank.item_animation(texture) {
            Some(sprite) => Pixels::Animated(sprite),
            None => Pixels::Static(
                bank.get(texture)
                    .ok_or_else(|| format!("物品纹理不存在：{texture}"))?,
            ),
        };
        scene.quads.push(quad(
            [
                [-0.5, 0.5, 1. / 32.],
                [0.5, 0.5, 1. / 32.],
                [0.5, -0.5, 1. / 32.],
                [-0.5, -0.5, 1. / 32.],
            ],
            [[0., 0.], [1., 0.], [1., 1.], [0., 1.]],
            texture,
            *index,
        ));
        scene.quads.push(quad(
            [
                [0.5, 0.5, -1. / 32.],
                [-0.5, 0.5, -1. / 32.],
                [-0.5, -0.5, -1. / 32.],
                [0.5, -0.5, -1. / 32.],
            ],
            [[1., 0.], [0., 0.], [0., 1.], [1., 1.]],
            texture,
            *index,
        ));
        // 平面 GUI 中严格侧向的边不贡献像素；世界场景必须保留完整轮廓。
        if !world && scene.transform.0[0][2] == 0. && scene.transform.0[1][2] == 0. {
            continue;
        }
        for span in bank
            .generated_items()
            .get(bank, texture, &pixels, cancelled)?
            .iter()
        {
            scene.quads.push(span.face(pixels.size(), texture, *index)?);
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "item_generated_tests.rs"]
pub(crate) mod tests;
