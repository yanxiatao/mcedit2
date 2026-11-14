//! 工具的真实指针手势。射线使用 Minecraft 方块坐标，空处不伪造状态。
use mcedit2_world::{editing::Bounds, sections::State};
use serde::{Deserialize, Serialize};

/// 激活、参数更新和正常停用都使用实际文档；普通请求未声明时执行真实输入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPhase {
    Activate,
    Options,
    Action,
    Deactivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Move,
    Press,
    Drag,
    Release,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ray {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    pub position: [i32; 3],
    /// None 明确表示工作平面上的几何落点。
    pub state: Option<State>,
    pub normal: [i32; 3],
    pub point: Option<[f64; 3]>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sample {
    pub phase: Phase,
    pub screen: [f32; 2],
    pub ray: Ray,
    pub location: Option<Location>,
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gesture {
    pub view: String,
    pub samples: Vec<Sample>,
}
impl Gesture {
    pub fn is_hover(&self) -> bool {
        self.samples.len() == 1 && self.samples[0].phase == Phase::Move
    }
    pub fn verify_snapshot(
        &self,
        clip: &mcedit2_world::clipboard::BlockClipboard,
    ) -> Result<(), String> {
        for sample in &self.samples {
            let Some(location) = &sample.location else {
                continue;
            };
            let mut local = [0; 3];
            for axis in 0..3 {
                local[axis] = location.position[axis]
                    .checked_sub(clip.source_min[axis])
                    .ok_or("工具触点坐标溢出")?;
                if local[axis] < 0 || local[axis] >= i32::from(clip.size[axis]) {
                    return Err("工具触点超出实际冻结范围".into());
                }
            }
            let index = (local[1] as usize * usize::from(clip.size[2]) + local[2] as usize)
                * usize::from(clip.size[0])
                + local[0] as usize;
            if clip.mask.as_ref().is_some_and(|mask| !mask[index]) {
                return Err("工具触点位于未声明格".into());
            }
            if location
                .state
                .as_ref()
                .is_some_and(|state| state != &clip.blocks[index])
            {
                return Err("工具触点的真实方块状态已改变".into());
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.view.is_empty()
            || (!self.is_hover()
                && (self.samples.len() < 2
                    || self.samples.first().map(|sample| sample.phase) != Some(Phase::Press)
                    || self.samples.last().map(|sample| sample.phase) != Some(Phase::Release)
                    || self.samples[1..self.samples.len() - 1]
                        .iter()
                        .any(|sample| sample.phase != Phase::Drag)))
        {
            return Err("工具输入需要单次真实悬停，或完整按下 / 拖动 / 松开顺序".into());
        }
        for sample in &self.samples {
            if !sample.screen.iter().all(|value| value.is_finite())
                || !sample
                    .ray
                    .origin
                    .iter()
                    .chain(&sample.ray.direction)
                    .all(|value| value.is_finite())
                || sample.ray.direction.iter().all(|value| *value == 0.)
            {
                return Err("工具手势的屏幕位置或射线无效".into());
            }
            if let Some(location) = &sample.location {
                if location
                    .normal
                    .iter()
                    .map(|value| i64::from(*value).abs())
                    .sum::<i64>()
                    != 1
                    || location
                        .point
                        .is_some_and(|point| !point.iter().all(|value| value.is_finite()))
                {
                    return Err("工具落点的方向或交点无效".into());
                }
                if let Some(state) = &location.state {
                    if mcedit2_world::editing::parse_state(&mcedit2_world::editing::render_state(
                        state,
                    ))
                    .map_err(|error| error.to_string())?
                        != *state
                    {
                        return Err("工具落点状态无效".into());
                    }
                }
            }
        }
        Ok(())
    }
    pub fn bounds(&self) -> Result<Option<Bounds>, String> {
        let mut positions = self
            .samples
            .iter()
            .filter_map(|sample| sample.location.as_ref().map(|location| location.position));
        let Some(first) = positions.next() else {
            return Ok(None);
        };
        let (mut minimum, mut maximum) = (first, first);
        for position in positions {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(position[axis]);
                maximum[axis] = maximum[axis].max(position[axis]);
            }
        }
        let bounds = Bounds { minimum, maximum };
        bounds.volume().map_err(|error| error.to_string())?;
        Ok(Some(bounds))
    }
    pub fn offset(&mut self, delta: [i32; 3]) -> Result<(), String> {
        for sample in &mut self.samples {
            for axis in 0..3 {
                sample.ray.origin[axis] += f64::from(delta[axis]);
                if let Some(location) = &mut sample.location {
                    location.position[axis] = location.position[axis]
                        .checked_add(delta[axis])
                        .ok_or("工具坐标偏移溢出")?;
                    if let Some(point) = &mut location.point {
                        point[axis] += f64::from(delta[axis]);
                    }
                }
            }
        }
        self.validate()
    }
}
