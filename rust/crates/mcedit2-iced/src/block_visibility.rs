//! 世界方块图层随不可变场景携带；无资源时也服从普通地形开关。
use crate::textures::TextureBank;
use mcedit2_world::sections::State;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layers {
    pub blocks: bool,
    pub block_entities: bool,
}

impl Default for Layers {
    fn default() -> Self {
        Self {
            blocks: true,
            block_entities: true,
        }
    }
}

impl Layers {
    pub(crate) fn visible(self, state: &State, bank: Option<&TextureBank>) -> bool {
        if self.blocks == self.block_entities {
            return self.blocks;
        }
        if bank.is_some_and(|bank| bank.is_block_entity_model(state)) {
            self.block_entities
        } else {
            self.blocks
        }
    }
}
