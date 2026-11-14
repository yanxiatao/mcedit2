//! 原生船与矿车类型；显示、保存光照及火焰尺寸共用注册身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cargo {
    Empty,
    Chest,
    Furnace,
    Tnt,
    Hopper,
    Spawner,
    CommandBlock,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vehicle {
    Boat { chest: bool },
    Minecart { cargo: Cargo },
}
impl Vehicle {
    pub fn read(id: &str) -> Option<Self> {
        let cargo = match id {
            "minecraft:boat" => return Some(Self::Boat { chest: false }),
            "minecraft:chest_boat" => return Some(Self::Boat { chest: true }),
            "minecraft:minecart" => Cargo::Empty,
            "minecraft:chest_minecart" => Cargo::Chest,
            "minecraft:furnace_minecart" => Cargo::Furnace,
            "minecraft:tnt_minecart" => Cargo::Tnt,
            "minecraft:hopper_minecart" => Cargo::Hopper,
            "minecraft:spawner_minecart" => Cargo::Spawner,
            "minecraft:command_block_minecart" | "minecraft:commandblock_minecart" => {
                Cargo::CommandBlock
            }
            _ => return None,
        };
        Some(Self::Minecart { cargo })
    }
    pub fn check_version(self, id: &str, version: i32) -> Result<(), String> {
        match version {
            1343 if matches!(self, Self::Boat { chest: true })
                || id == "minecraft:command_block_minecart" =>
            {
                Err(format!("旧版没有原生载具注册 {id}"))
            }
            3465 | 3955 if id == "minecraft:commandblock_minecart" => {
                Err(format!("版本 {version} 不使用旧载具注册名 {id}"))
            }
            1343 | 3465 | 3955 => Ok(()),
            _ => Err(format!("尚未绑定 DataVersion={version} 的载具格式")),
        }
    }
}
