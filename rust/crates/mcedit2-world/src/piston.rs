//! 移动活塞的保存载荷及客户端绘制部件；不推进 tick，不从碰撞盒推测资源模型。
use crate::sections::{self, State, Tag};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub state: State,
    pub moved: State,
    pub facing: u8,
    progress: u32,
    pub extending: bool,
    pub source: bool,
    pub version: i32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub state: State,
    pub translation: [f32; 3],
    pub sample_offset: [i32; 3],
    pub cull: bool,
}
pub fn is_piston_id(id: &str) -> bool {
    id == "minecraft:piston"
}
pub fn is_piston_state_name(name: &str) -> bool {
    matches!(
        name,
        "minecraft:moving_piston" | "minecraft:piston_extension"
    )
}
pub fn parse(compound: &[u8], state: &State, version: Option<i32>) -> Result<Source, String> {
    let mut bytes = vec![10, 0, 0];
    bytes.extend_from_slice(compound);
    read(
        &sections::parse_root(&bytes).map_err(|e| e.to_string())?,
        state,
        version,
    )
}
pub(crate) fn read(tag: &Tag<'_>, state: &State, version: Option<i32>) -> Result<Source, String> {
    let version = version.ok_or("移动活塞缺少源 DataVersion")?;
    if !matches!(version, 1343 | 3465 | 3955) {
        return Err(format!("未核对 DataVersion={version} 的移动活塞保存格式"));
    }
    let id = tag
        .require("id", 8)
        .and_then(Tag::string)
        .map_err(|e| e.to_string())?;
    if !is_piston_id(&id) {
        return Err(format!("移动活塞的实际 BE 类型不符：{id}"));
    }
    if !is_piston_state_name(&state.name) && !state.name.starts_with("numeric:") {
        return Err(format!("移动活塞载荷与实际方块 {} 不匹配", state.name));
    }
    let moved = if version == 1343 {
        let id = tag
            .require("blockId", 3)
            .and_then(Tag::i32)
            .map_err(|e| e.to_string())?;
        let data = tag
            .require("blockData", 3)
            .and_then(Tag::i32)
            .map_err(|e| e.to_string())?;
        if !(0..=4095).contains(&id) || !(0..=15).contains(&data) {
            return Err("旧移动活塞的方块 ID / metadata 越界".into());
        }
        State {
            name: format!("numeric:{id}"),
            properties: vec![("data".into(), data.to_string())],
        }
    } else {
        sections::block_state(tag.require("blockState", 10).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
    };
    let facing = tag
        .require("facing", 3)
        .and_then(Tag::i32)
        .map_err(|e| e.to_string())?;
    if !(0..6).contains(&facing) {
        return Err(format!("移动活塞 facing={facing} 无效"));
    }
    let progress = tag.require("progress", 5).map_err(|e| e.to_string())?;
    let progress = f32::from_be_bytes(
        progress
            .raw
            .try_into()
            .map_err(|_| "移动活塞 progress 需要 Float")?,
    );
    if !progress.is_finite() {
        return Err("移动活塞 progress 必须为有限数".into());
    }
    Ok(Source {
        state: state.clone(),
        moved,
        facing: facing as u8,
        progress: progress.to_bits(),
        extending: tag
            .require("extending", 1)
            .and_then(Tag::byte)
            .map_err(|e| e.to_string())?
            != 0,
        source: tag
            .require("source", 1)
            .and_then(Tag::byte)
            .map_err(|e| e.to_string())?
            != 0,
        version,
    })
}
pub fn direction(facing: u8) -> Result<[i32; 3], String> {
    match facing {
        0 => Ok([0, -1, 0]),
        1 => Ok([0, 1, 0]),
        2 => Ok([0, 0, -1]),
        3 => Ok([0, 0, 1]),
        4 => Ok([-1, 0, 0]),
        5 => Ok([1, 0, 0]),
        _ => Err(format!("移动活塞 facing={facing} 无效")),
    }
}
fn set(state: &mut State, key: &str, value: &str) -> Result<(), String> {
    let item = state
        .properties
        .iter_mut()
        .find(|(name, _)| name == key)
        .ok_or_else(|| format!("移动状态 {} 没有 {key} 属性", state.name))?;
    item.1 = value.into();
    Ok(())
}
impl Source {
    pub fn progress(&self) -> f32 {
        f32::from_bits(self.progress)
    }
    pub fn translation(&self) -> Result<[f32; 3], String> {
        let delta = if self.extending {
            self.progress() - 1.0
        } else {
            1.0 - self.progress()
        };
        Ok(direction(self.facing)?.map(|v| v as f32 * delta))
    }
    /// moved 是准确客户端资源注册表解释后的状态，旧数字状态由调用方绑定，不能猜别名。
    pub fn render_parts(&self, moved: &State) -> Result<Vec<Part>, String> {
        let legacy = self.version == 1343;
        let progress = self.progress();
        if matches!(
            moved.name.as_str(),
            "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
        ) || (legacy && progress >= 1.0)
        {
            return Ok(Vec::new());
        }
        if is_piston_state_name(&moved.name) {
            return Err("移动活塞的被移动状态再次引用同一个移动活塞上下文".into());
        }
        let direction = direction(self.facing)?;
        let movement = direction.map(|v| if self.extending { v } else { -v });
        let sample_offset = if legacy { [0; 3] } else { movement.map(|v| -v) };
        let translation = self.translation()?;
        let mut state = moved.clone();
        if moved.name == "minecraft:piston_head" && progress <= if legacy { 0.25 } else { 4.0 } {
            set(
                &mut state,
                "short",
                if legacy || progress <= 0.5 {
                    "true"
                } else {
                    "false"
                },
            )?;
            return Ok(vec![Part {
                state,
                translation,
                sample_offset,
                cull: legacy,
            }]);
        }
        if self.source && !self.extending {
            let facing = state
                .properties
                .iter()
                .find_map(|(k, v)| (k == "facing").then_some(v.clone()))
                .ok_or("源活塞缺少实际 facing")?;
            let head = State {
                name: "minecraft:piston_head".into(),
                properties: vec![
                    ("facing".into(), facing),
                    ("short".into(), (progress >= 0.5).to_string()),
                    (
                        "type".into(),
                        if state.name == "minecraft:sticky_piston" {
                            "sticky"
                        } else {
                            "normal"
                        }
                        .into(),
                    ),
                ],
            };
            set(&mut state, "extended", "true")?;
            return Ok(vec![
                Part {
                    state: head,
                    translation,
                    sample_offset,
                    cull: legacy,
                },
                Part {
                    state,
                    translation: [0.; 3],
                    sample_offset: [0; 3],
                    cull: true,
                },
            ]);
        }
        Ok(vec![Part {
            state,
            translation,
            sample_offset,
            cull: false,
        }])
    }
}
