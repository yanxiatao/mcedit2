//! 工具光标和覆盖层的原生几何；坐标来自工具定义，不修改或伪造存档对象。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Primitive {
    pub points: Vec<[f64; 3]>,
    pub closed: bool,
    pub fill: Option<[f32; 4]>,
    pub stroke: Option<[f32; 4]>,
    pub width: f32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub cursor: Vec<Primitive>,
    pub overlay: Vec<Primitive>,
}
impl Scene {
    pub fn validate(&self) -> Result<(), String> {
        for primitive in self.cursor.iter().chain(&self.overlay) {
            if primitive.points.len() < 2
                || (primitive.fill.is_some() && (!primitive.closed || primitive.points.len() < 3))
                || (primitive.fill.is_none() && primitive.stroke.is_none())
                || !primitive.width.is_finite()
                || primitive.width <= 0.
                || primitive
                    .points
                    .iter()
                    .flatten()
                    .any(|value| !value.is_finite())
                || primitive
                    .fill
                    .iter()
                    .chain(&primitive.stroke)
                    .flatten()
                    .any(|value| !value.is_finite() || !(0. ..=1.).contains(value))
            {
                return Err("工具场景几何、颜色或线宽无效".into());
            }
        }
        Ok(())
    }
    pub fn offset(&mut self, delta: [f64; 3]) {
        for primitive in self.cursor.iter_mut().chain(&mut self.overlay) {
            for point in &mut primitive.points {
                for axis in 0..3 {
                    point[axis] += delta[axis];
                }
            }
        }
    }
}
