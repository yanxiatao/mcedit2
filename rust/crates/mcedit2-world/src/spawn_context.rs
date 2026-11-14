//! 从实际保存的 BE 解释动态碰撞；只计算刷怪谓词，不把资源模型当作碰撞形状。
use super::Flags;
use crate::sections::{State, Tag};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    sync::OnceLock,
};

type Box3 = [f64; 6];
const EPSILON: f64 = 1.0e-7;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Static,
    Shulker,
    MovingPiston,
    Piston,
}
#[derive(Deserialize)]
struct Block {
    kind: Kind,
    properties: Vec<String>,
    states: Vec<(Vec<String>, usize, bool)>,
}
#[derive(Deserialize)]
struct Data {
    data_version: i32,
    shapes: Vec<Vec<Box3>>,
    blocks: BTreeMap<String, Block>,
}
struct Entry {
    kind: Kind,
    shape: usize,
    narrow_offset: bool,
}
pub(crate) struct SavedContext {
    version: i32,
    states: HashMap<State, Entry>,
    shapes: Vec<Vec<Box3>>,
}
impl SavedContext {
    fn parse(text: &str, version: i32) -> Result<Self, String> {
        let data: Data = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if data.data_version != version {
            return Err("碰撞目录版本不符".into());
        }
        let mut states = HashMap::new();
        for (name, block) in data.blocks {
            for (values, shape, narrow_offset) in block.states {
                if values.len() != block.properties.len() || shape >= data.shapes.len() {
                    return Err(format!("碰撞目录状态无效：{name}"));
                }
                let mut properties: Vec<_> = block.properties.iter().cloned().zip(values).collect();
                properties.sort();
                states.insert(
                    State {
                        name: name.clone(),
                        properties,
                    },
                    Entry {
                        kind: block.kind,
                        shape,
                        narrow_offset,
                    },
                );
            }
        }
        Ok(Self {
            version,
            states,
            shapes: data.shapes,
        })
    }
    pub(crate) fn get(version: i32) -> Result<&'static Self, String> {
        static OLD: OnceLock<Result<SavedContext, String>> = OnceLock::new();
        static NEW: OnceLock<Result<SavedContext, String>> = OnceLock::new();
        let (slot, text) = match version {
            3465 => (&OLD, include_str!("../assets/spawn-context-1.20.1.json")),
            3955 => (&NEW, include_str!("../assets/spawn-context-1.21.1.json")),
            _ => return Err(format!("尚未核对 DataVersion={version} 的保存碰撞上下文")),
        };
        slot.get_or_init(|| Self::parse(text, version))
            .as_ref()
            .map_err(Clone::clone)
    }
    fn entry(&self, state: &State) -> Result<&Entry, String> {
        let mut key = state.clone();
        key.properties.sort();
        self.states
            .get(&key)
            .ok_or_else(|| format!("未核对碰撞状态：{state:?}"))
    }
    pub(crate) fn flags(&self, state: &State, entity: &Tag<'_>) -> Result<Flags, String> {
        let kind = self.entry(state)?.kind;
        let id = entity
            .require("id", 8)
            .and_then(Tag::string)
            .map_err(|e| e.to_string())?;
        match kind {
            Kind::Shulker => {
                if id != "minecraft:shulker_box" {
                    return Err(format!("潜影盒的实际 BE 类型不符：{id}"));
                }
                // 原生 load 只读取物品等保存字段，开启动画不保存；重新读取后为关闭的完整盒体。
                Ok(Flags(1))
            }
            Kind::MovingPiston => {
                let saved = crate::piston::read(entity, state, Some(self.version))?;
                let facing = i32::from(saved.facing);
                let direction = crate::piston::direction(saved.facing)?.map(f64::from);
                let shape = self.moving_shape(
                    &saved.moved,
                    facing,
                    direction,
                    saved.progress(),
                    saved.extending,
                    saved.source,
                )?;
                Ok(Flags(
                    u8::from(full_top(&shape)) | if full_cube(&shape) { 0 } else { 2 },
                ))
            }
            _ => Err(format!("状态没有保存上下文解释器：{state:?}")),
        }
    }
    fn moving_shape(
        &self,
        moved: &State,
        facing: i32,
        direction: [f64; 3],
        progress: f32,
        extending: bool,
        source: bool,
    ) -> Result<Vec<Box3>, String> {
        let moved_entry = self.entry(moved)?;
        let mut out = Vec::new();
        if !extending && source && moved_entry.kind == Kind::Piston {
            let mut base = moved.clone();
            set(&mut base, "extended", "true");
            out.extend_from_slice(&self.shapes[self.entry(&base)?.shape]);
        }
        let head;
        let collision = if source {
            head = State {
                name: "minecraft:piston_head".into(),
                properties: vec![
                    (
                        "facing".into(),
                        ["down", "up", "north", "south", "west", "east"][facing as usize].into(),
                    ),
                    (
                        "short".into(),
                        (extending != (1.0_f32 - progress < 0.25)).to_string(),
                    ),
                    ("type".into(), "normal".into()),
                ],
            };
            self.entry(&head)?
        } else {
            moved_entry
        };
        if !source && collision.kind == Kind::MovingPiston {
            return Err("移动活塞的被移动状态再次引用同一个移动活塞上下文".into());
        }
        // 目录已逐状态证明：这些位置偏移形状在 X 或 Z 上严格窄于一格。
        // 非源移动只有此形状，任意平移都不可能覆盖完整顶面或完整格体。
        if collision.narrow_offset {
            return Ok(out);
        }
        let delta = f64::from(if extending {
            progress - 1.0
        } else {
            1.0 - progress
        });
        for b in &self.shapes[collision.shape] {
            let mut shifted = *b;
            for axis in 0..3 {
                shifted[axis] += direction[axis] * delta;
                shifted[axis + 3] += direction[axis] * delta;
            }
            out.push(shifted);
        }
        Ok(out)
    }
}
fn set(state: &mut State, key: &str, value: &str) {
    state
        .properties
        .iter_mut()
        .find(|(name, _)| name == key)
        .unwrap()
        .1 = value.into();
}
fn snap(v: f64) -> f64 {
    if v.abs() <= EPSILON {
        0.
    } else if (v - 1.).abs() <= EPSILON {
        1.
    } else {
        v
    }
}
fn coordinates(boxes: &[Box3], axis: usize) -> Vec<f64> {
    let mut coordinates = vec![0., 1.];
    coordinates.extend(
        boxes
            .iter()
            .flat_map(|b| [snap(b[axis]), snap(b[axis + 3])]),
    );
    coordinates.sort_by(f64::total_cmp);
    coordinates.dedup_by(|a, b| (*a - *b).abs() <= EPSILON);
    coordinates
}
fn contains(b: &Box3, point: [f64; 3]) -> bool {
    (0..3).all(|axis| snap(b[axis]) <= point[axis] && point[axis] < snap(b[axis + 3]))
}
fn full_cube(boxes: &[Box3]) -> bool {
    if boxes.is_empty()
        || boxes
            .iter()
            .any(|b| (0..3).any(|axis| snap(b[axis]) < 0. || snap(b[axis + 3]) > 1.))
    {
        return false;
    }
    let axes = [
        coordinates(boxes, 0),
        coordinates(boxes, 1),
        coordinates(boxes, 2),
    ];
    axes[0].windows(2).all(|x| {
        axes[1].windows(2).all(|y| {
            axes[2].windows(2).all(|z| {
                let point = [
                    (x[0] + x[1]) * 0.5,
                    (y[0] + y[1]) * 0.5,
                    (z[0] + z[1]) * 0.5,
                ];
                boxes.iter().any(|b| contains(b, point))
            })
        })
    })
}
fn full_top(boxes: &[Box3]) -> bool {
    // VoxelShape.getFaceShape(UP) 在 1-1e-7 处取实际体素切片，再与完整格面作 NOT_SAME。
    let slice = boxes
        .iter()
        .filter(|b| b[1] <= 0.9999999 && b[4] > 0.9999999)
        .copied()
        .collect::<Vec<_>>();
    if slice.is_empty()
        || slice.iter().any(|b| {
            [0, 2]
                .into_iter()
                .any(|axis| snap(b[axis]) < 0. || snap(b[axis + 3]) > 1.)
        })
    {
        return false;
    }
    let x = coordinates(&slice, 0);
    let z = coordinates(&slice, 2);
    x.windows(2).all(|x| {
        z.windows(2).all(|z| {
            let px = (x[0] + x[1]) * 0.5;
            let pz = (z[0] + z[1]) * 0.5;
            slice
                .iter()
                .any(|b| snap(b[0]) <= px && px < snap(b[3]) && snap(b[2]) <= pz && pz < snap(b[5]))
        })
    })
}
