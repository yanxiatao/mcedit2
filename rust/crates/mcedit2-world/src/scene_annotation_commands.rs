//! 命令类别图层复用真实方块实体，不复制来源或创建编辑对象。
use super::{Kind, LayerMask, Marker};
use crate::{entity_catalog::EntityVisual, nbt_document::Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandLayer {
    Colors,
    Locations,
}
impl CommandLayer {
    pub const ALL: [Self; 2] = [Self::Colors, Self::Locations];
    pub const MASK: LayerMask = Self::Colors.bit() | Self::Locations.bit();
    pub const fn bit(self) -> LayerMask {
        1 << match self {
            Self::Colors => 11,
            Self::Locations => 12,
        }
    }
}

pub(super) fn is_command_id(id: &str) -> bool {
    matches!(
        id,
        "Control"
            | "minecraft:command_block"
            | "minecraft:chain_command_block"
            | "minecraft:repeating_command_block"
    )
}

pub(super) fn name(visual: &EntityVisual) -> Result<Option<&str>, String> {
    let text = match visual.appearance.get("Command") {
        Some(Value::String(text)) => text.as_str(),
        None => "", // 原命令字段的未写入初值为空字符串。
        Some(_) => return Err("命令方块 Command 必须为 String".into()),
    };
    Ok(text
        .split_whitespace()
        .next()
        .map(|name| name.strip_prefix('/').unwrap_or(name)))
}

impl Marker {
    pub fn is_command_block(&self) -> bool {
        self.kind == Kind::BlockEntity && self.id.as_deref().is_some_and(is_command_id)
    }

    /// 命令方块的位置 / 填色单独控制，其他方块实体位置不包含它们。
    pub fn layers(&self) -> LayerMask {
        if self.is_command_block() {
            CommandLayer::MASK
        } else {
            self.kind.bit()
        }
    }

    /// 由命令名称生成稳定类别颜色；不执行命令，不按命令清单枚举。
    pub fn command_color(&self) -> Result<Option<[f32; 3]>, String> {
        if !self.is_command_block() {
            return Ok(None);
        }
        if self.visual_error.is_some() {
            return Ok(Some([1., 0.2, 0.2]));
        }
        let command = self.visual.as_deref().map(name).transpose()?.flatten();
        let Some(command) = command.filter(|name| !name.is_empty()) else {
            return Ok(Some([1., 1., 0.2]));
        };
        let command = command.to_lowercase();
        let command = command.strip_prefix("minecraft:").unwrap_or(&command);
        let hash = Sha256::digest(command.as_bytes());
        let hue = f32::from(u16::from_be_bytes([hash[0], hash[1]])) * (6. / 65536.);
        let part = hue.fract();
        let rgb = match hue as u8 {
            0 => [1., part, 0.],
            1 => [1. - part, 1., 0.],
            2 => [0., 1., part],
            3 => [0., 1. - part, 1.],
            4 => [part, 0., 1.],
            5 => [1., 0., 1. - part],
            _ => unreachable!("SHA-256 类别色相在 [0, 6) 内"),
        };
        Ok(Some(rgb.map(|value| 0.18 + value * 0.77)))
    }
}
