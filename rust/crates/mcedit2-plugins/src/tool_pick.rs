//! 旧工具动作只请求切换原生工具，不携带或隐式提交编辑载荷。
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Builtin {
    Select,
    Move,
    Clone,
    Brush,
    FloodFill,
    Generate,
    InspectChunk,
    InspectBlock,
    InspectEntity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Pick {
    Builtin { tool: Builtin },
    Plugin { command: String },
}
impl Pick {
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Plugin { command } = self {
            if !command.starts_with("tool:") || command == "tool:" {
                return Err("工具切换请求需要实际工具命令标识".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconFile {
    pub source: String,
    pub data: String,
    pub size: Option<[u32; 2]>,
    pub mode: u8,
    pub state: u8,
}
pub fn validate_icon(files: &[IconFile]) -> Result<(), String> {
    if files.is_empty()
        || files.iter().any(|file| {
            file.data.is_empty()
                || file.mode > 3
                || file.state > 1
                || file.size.is_some_and(|size| {
                    size.contains(&0) || size.iter().any(|value| *value > i32::MAX as u32)
                })
        })
    {
        return Err("图标载荷、模式、状态或尺寸无效".into());
    }
    Ok(())
}

fn repeat_default() -> bool {
    true
}
fn window_context() -> u8 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionState {
    #[serde(default)]
    pub action_id: Option<u64>,
    pub enabled: bool,
    pub visible: bool,
    pub checkable: bool,
    pub checked: bool,
    pub help: String,
    #[serde(default)]
    pub shortcuts: Vec<Vec<String>>,
    #[serde(default = "window_context")]
    pub shortcut_context: u8,
    #[serde(default = "repeat_default")]
    pub auto_repeat: bool,
    #[serde(default)]
    pub icon: Option<Vec<IconFile>>,
}
impl ActionState {
    pub fn validate(&self) -> Result<(), String> {
        if self.checked && !self.checkable
            || self.shortcut_context > 3
            || self.shortcuts.iter().any(|sequence| {
                sequence.is_empty()
                    || sequence.len() > 4
                    || sequence.iter().any(|key| key.is_empty())
            })
            || self.action_id == Some(0)
        {
            return Err("插件动作的勾选状态或说明无效".into());
        }
        if let Some(files) = &self.icon {
            validate_icon(files)?;
        }
        Ok(())
    }
}
