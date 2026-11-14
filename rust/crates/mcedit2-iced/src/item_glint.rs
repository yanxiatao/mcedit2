//! 原版附魔纹理、TextureMat 与 SRC_COLOR / ONE 合成；仅在相同几何深度绘制。
use crate::textures::{TextureBank, Tile};
use std::{
    borrow::Cow,
    sync::OnceLock,
    time::{Duration, Instant},
};
pub(crate) const FRAME_MS: u64 = 50;
fn start() -> &'static Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now)
}
pub(crate) fn tick() -> u64 {
    (start().elapsed().as_millis() / u128::from(FRAME_MS))
        .try_into()
        .expect("物品动画时钟超出 u64")
}
pub(crate) fn deadline() -> Instant {
    *start() + Duration::from_millis((tick() + 1) * FRAME_MS)
}
pub(crate) fn texture_dependency(id: &str) -> bool {
    matches!(
        id,
        "minecraft:misc/enchanted_item_glint" | "minecraft:misc/enchanted_glint_item"
    )
}
pub(crate) struct Pass<'a> {
    texture: Cow<'a, Tile>,
    matrix: [[f32; 3]; 2],
    rgb: [f32; 3],
}
impl Pass<'_> {
    pub fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        let uv = self.matrix.map(|m| m[0] * uv[0] + m[1] * uv[1] + m[2]);
        let x = (uv[0].rem_euclid(1.) * self.texture.width as f32).floor() as usize;
        let y = (uv[1].rem_euclid(1.) * self.texture.height as f32).floor() as usize;
        let index = (y * self.texture.width as usize + x) * 4;
        std::array::from_fn(|i| {
            self.texture.rgba[index + i] as f32 / 255. * if i < 3 { self.rgb[i] } else { 1. }
        })
    }
}
fn matrix(scale: f32, angle: f32, translation: [f32; 2]) -> [[f32; 3]; 2] {
    let (s, c) = angle.to_radians().sin_cos();
    [
        [scale * c, -scale * s, translation[0]],
        [scale * s, scale * c, translation[1]],
    ]
}
pub(crate) fn prepare(
    bank: &TextureBank,
    version: Option<i32>,
    millis: u64,
) -> Result<Vec<Pass<'_>>, String> {
    let texture = if version == Some(1343) {
        "minecraft:misc/enchanted_item_glint"
    } else {
        "minecraft:misc/enchanted_glint_item"
    };
    let tile = bank
        .item_tile(texture)
        .ok_or_else(|| format!("附魔光效纹理不存在：{texture}"))?;
    if version == Some(1343) {
        // 旧版矩阵为 S*T*R；平移在缩放内，两个通道分别采用 3000 / 4873 ms 周期。
        return Ok([(3000, -50., 1.), (4873, 10., -1.)]
            .into_iter()
            .map(|(period, angle, sign)| Pass {
                texture: tile.clone(),
                matrix: matrix(
                    8.,
                    angle,
                    [sign * (millis % period) as f32 / period as f32, 0.],
                ),
                rgb: [128. / 255., 64. / 255., 204. / 255.],
            })
            .collect());
    }
    // 原版默认 glintSpeed=0.5、glintStrength=0.75；T*R*S，不加入世界雾。
    let phase = millis * 4;
    Ok(vec![Pass {
        texture: tile,
        matrix: matrix(
            8.,
            10.,
            [
                -((phase % 110000) as f32) / 110000.,
                (phase % 30000) as f32 / 30000.,
            ],
        ),
        rgb: [0.75; 3],
    }])
}
