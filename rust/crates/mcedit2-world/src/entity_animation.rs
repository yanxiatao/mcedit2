//! 原生关键帧常量的无客户端播放：共享线性 / Catmull-Rom 插值及部件累加语义。
use crate::entity_model::{Catalog, PartPose};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Target {
    Position,
    Rotation,
    Scale,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Interpolation {
    Linear,
    CatmullRom,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Keyframe {
    time: f32,
    value: [f32; 3],
    interpolation: Interpolation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Channel {
    target: Target,
    keyframes: Vec<Keyframe>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    length: f32,
    looping: bool,
    bones: BTreeMap<String, Vec<Channel>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Animations {
    data_version: i32,
    definitions: BTreeMap<String, Definition>,
}
/// 调用方提供真实模型 root() 对应路径与已解释的时间，不由文件名或 UUID 生成运行态。
pub struct Sample<'a> {
    pub animation: &'a str,
    pub root: &'a str,
    pub elapsed_ms: i64,
    pub weight: f32,
}
impl Animations {
    pub fn builtin(version: i32) -> Result<&'static Self, String> {
        static MID: OnceLock<Result<Animations, String>> = OnceLock::new();
        static NEW: OnceLock<Result<Animations, String>> = OnceLock::new();
        let (slot, json) = match version {
            3465 => (
                &MID,
                include_str!("../assets/entity-animations-1.20.1.json"),
            ),
            3955 => (
                &NEW,
                include_str!("../assets/entity-animations-1.21.1.json"),
            ),
            _ => return Err(format!("DataVersion={version} 没有已绑定的关键帧定义")),
        };
        slot.get_or_init(|| {
            let data = Self::from_json(json)?;
            if data.data_version != version {
                return Err("关键帧目录版本不符".into());
            }
            Ok(data)
        })
        .as_ref()
        .map_err(Clone::clone)
    }
    pub fn from_json(json: &str) -> Result<Self, String> {
        let data: Self = serde_json::from_str(json).map_err(|error| error.to_string())?;
        for (name, definition) in &data.definitions {
            if definition.looping && definition.length <= 0. {
                return Err(format!("循环动画 {name} 必须有正时长"));
            }
            for (bone, channels) in &definition.bones {
                for channel in channels {
                    if channel.keyframes.is_empty()
                        || channel
                            .keyframes
                            .windows(2)
                            .any(|pair| pair[0].time > pair[1].time)
                    {
                        return Err(format!("动画 {name} 的 {bone} 关键帧必须非空并按时间排列"));
                    }
                }
            }
        }
        Ok(data)
    }
    /// 在当前构造 / 已叠加姿态上累加，不覆写调用方保存姿态、可见性或其他动画。
    pub fn apply(
        &self,
        catalog: &Catalog,
        layer: &str,
        poses: &mut BTreeMap<String, PartPose>,
        sample: Sample<'_>,
    ) -> Result<(), String> {
        let definition = self
            .definitions
            .get(sample.animation)
            .ok_or_else(|| format!("未知关键帧定义 {}", sample.animation))?;
        let seconds = sample.elapsed_ms as f32 / 1000.;
        let time = if definition.looping {
            seconds % definition.length
        } else {
            seconds
        };
        for (bone, channels) in &definition.bones {
            let path = bone_path(catalog, layer, sample.root, bone)?;
            let mut pose = catalog.resolved_pose(layer, &path, poses)?;
            for channel in channels {
                let value = channel.sample(time, sample.weight);
                let (target, label) = match channel.target {
                    Target::Position => (&mut pose.pivot, "pivot"),
                    Target::Rotation => (&mut pose.rotation, "rotation"),
                    Target::Scale => (&mut pose.scale, "scale"),
                };
                let target = target
                    .as_mut()
                    .ok_or_else(|| format!("动画部件 {layer}:{path} 缺少实际 {label}"))?;
                for i in 0..3 {
                    target[i] += value[i];
                }
            }
            poses.insert(path, pose);
        }
        Ok(())
    }
}
fn bone_path(catalog: &Catalog, layer: &str, root: &str, bone: &str) -> Result<String, String> {
    if bone == "root" {
        catalog.resolved_pose(layer, root, &BTreeMap::new())?;
        return Ok(root.into());
    }
    let mut candidates = catalog.parts(layer)?.filter(|path| {
        *path != root
            && (root.is_empty()
                || path
                    .strip_prefix(root)
                    .is_some_and(|suffix| suffix.starts_with('/')))
            && path.rsplit('/').next() == Some(bone)
    });
    let path = candidates
        .next()
        .ok_or_else(|| format!("动画部件 {layer}:{root} 没有实际骨骼 {bone}"))?;
    if candidates.next().is_some() {
        return Err(format!("动画部件 {layer}:{root} 的骨骼 {bone} 不唯一"));
    }
    Ok(path.into())
}
impl Channel {
    fn sample(&self, time: f32, weight: f32) -> [f32; 3] {
        let frames = &self.keyframes;
        // 原生 binarySearch 找到第一个 time <= timestamp，再取前一帧。
        let previous = frames
            .partition_point(|frame| time > frame.time)
            .saturating_sub(1);
        let next = (previous + 1).min(frames.len() - 1);
        let fraction = if previous == next {
            0.
        } else {
            ((time - frames[previous].time) / (frames[next].time - frames[previous].time))
                .clamp(0., 1.)
        };
        std::array::from_fn(|i| {
            let a = frames[previous].value[i];
            let b = frames[next].value[i];
            match frames[next].interpolation {
                Interpolation::Linear => (a + (b - a) * fraction) * weight,
                Interpolation::CatmullRom => {
                    let before = frames[previous.saturating_sub(1)].value[i];
                    let after = frames[(next + 1).min(frames.len() - 1)].value[i];
                    // Mth.catmullRom 的 float 运算次序；每个通道使用目标帧的插值器。
                    0.5 * (2. * a
                        + (b - before) * fraction
                        + (2. * before - 5. * a + 4. * b - after) * fraction * fraction
                        + (3. * a - before - 3. * b + after) * fraction * fraction * fraction)
                        * weight
                }
            }
        })
    }
}
