//! Java 动画源与离散的 20 Hz 上传时序；源图片只保留一次。
use super::{AlphaMode, Tile};

#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub index: u32,
    pub duration: u32,
}
#[derive(Clone, Debug)]
pub(crate) struct Sequence {
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub count: u32,
    pub duration: u32,
    /// 空表表示全部物理帧顺序播放，不按图片帧数分配表。
    pub steps: Vec<Step>,
    pub interpolate: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub index: u32,
    pub next: u32,
    pub elapsed: u32,
    pub duration: u32,
}
impl Sequence {
    pub fn stamp(&self, tick: u64) -> Stamp {
        let (index, next, elapsed, duration) = if self.steps.is_empty() {
            let cycle = u64::from(self.count) * u64::from(self.duration);
            let at = tick % cycle;
            let index = (at / u64::from(self.duration)) as u32;
            (
                index,
                (index + 1) % self.count,
                (at % u64::from(self.duration)) as u32,
                self.duration,
            )
        } else {
            let cycle = self
                .steps
                .iter()
                .map(|s| u64::from(s.duration))
                .sum::<u64>();
            let mut at = tick % cycle;
            let position = self
                .steps
                .iter()
                .position(|s| {
                    if at < u64::from(s.duration) {
                        true
                    } else {
                        at -= u64::from(s.duration);
                        false
                    }
                })
                .expect("有效动画时序覆盖整个周期");
            let step = &self.steps[position];
            (
                step.index,
                self.steps[(position + 1) % self.steps.len()].index,
                at as u32,
                step.duration,
            )
        };
        if !self.interpolate || index == next || elapsed == 0 {
            Stamp {
                index,
                next: index,
                elapsed: 0,
                duration: 1,
            }
        } else {
            Stamp {
                index,
                next,
                elapsed,
                duration,
            }
        }
    }
}
#[derive(Debug)]
pub(crate) struct Sprite {
    pub source: Tile,
    pub sequence: Sequence,
}
impl Sprite {
    pub fn indices(&self) -> Box<dyn Iterator<Item = u32> + '_> {
        if self.sequence.steps.is_empty() {
            Box::new(0..self.sequence.count)
        } else {
            Box::new(
                self.sequence
                    .steps
                    .iter()
                    .map(|step| step.index)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter(),
            )
        }
    }
    pub fn bytes(&self) -> usize {
        self.source.rgba.len() + self.sequence.steps.len() * std::mem::size_of::<Step>()
    }
    pub fn tile(&self, stamp: Stamp) -> Tile {
        let s = &self.sequence;
        let mut rgba = Vec::with_capacity((s.width as usize) * (s.height as usize) * 4);
        let row = |index: u32, y: u32| {
            let x = index % s.columns * s.width;
            let y = index / s.columns * s.height + y;
            let begin = ((y as usize) * self.source.width as usize + x as usize) * 4;
            &self.source.rgba[begin..begin + s.width as usize * 4]
        };
        for y in 0..s.height {
            let a = row(stamp.index, y);
            if stamp.index == stamp.next {
                rgba.extend_from_slice(a);
            } else {
                let b = row(stamp.next, y);
                // SpriteContents.InterpolationData 只插值 RGB，Alpha 保留当前帧；double 后向零截断。
                let weight = 1. - f64::from(stamp.elapsed) / f64::from(stamp.duration);
                for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
                    rgba.extend_from_slice(&[
                        (weight * f64::from(a[0]) + (1. - weight) * f64::from(b[0])) as u8,
                        (weight * f64::from(a[1]) + (1. - weight) * f64::from(b[1])) as u8,
                        (weight * f64::from(a[2]) + (1. - weight) * f64::from(b[2])) as u8,
                        a[3],
                    ]);
                }
            }
        }
        Tile {
            width: s.width,
            height: s.height,
            rgba,
        }
    }
    /// 分类覆盖实际播放的全部帧，帧切换不改变网格的透明通道或邻面证明。
    pub fn alpha_mode(&self) -> AlphaMode {
        let s = &self.sequence;
        let mut zero = false;
        for index in self.indices() {
            let x = index % s.columns * s.width;
            let y = index / s.columns * s.height;
            for y in y..y + s.height {
                let start = ((y as usize) * self.source.width as usize + x as usize) * 4;
                for alpha in self.source.rgba[start + 3..start + s.width as usize * 4]
                    .iter()
                    .step_by(4)
                {
                    match *alpha {
                        0 => zero = true,
                        255 => {}
                        _ => return AlphaMode::Blend,
                    }
                }
            }
        }
        if zero {
            AlphaMode::Cutout
        } else {
            AlphaMode::Opaque
        }
    }
}
