//! Java 客户端的方块坐标种子与 LegacyRandomSource；不以状态名哈希替代游戏随机序列。
use crate::java_random::LegacyRandom;
use mcedit2_world::sections::State;

#[derive(Clone, Copy)]
pub(super) struct SeedPosition {
    offset: [i32; 3],
}
impl SeedPosition {
    pub(super) fn new(state: &State) -> Self {
        let property = |name: &str| {
            state
                .properties
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        // 此名单来自 1.21.1 内建注册表的真实 DoorBlock / DoublePlantBlock / BedBlock。
        // 资源名相似的模组方块不能据此推断它们覆盖了 getSeed。
        let lower = matches!(
            state.name.as_str(),
            "minecraft:tall_seagrass"
                | "minecraft:sunflower"
                | "minecraft:lilac"
                | "minecraft:rose_bush"
                | "minecraft:peony"
                | "minecraft:tall_grass"
                | "minecraft:large_fern"
                | "minecraft:pitcher_crop"
                | "minecraft:pitcher_plant"
                | "minecraft:small_dripleaf"
                | "minecraft:oak_door"
                | "minecraft:iron_door"
                | "minecraft:spruce_door"
                | "minecraft:birch_door"
                | "minecraft:jungle_door"
                | "minecraft:acacia_door"
                | "minecraft:cherry_door"
                | "minecraft:dark_oak_door"
                | "minecraft:mangrove_door"
                | "minecraft:bamboo_door"
                | "minecraft:crimson_door"
                | "minecraft:warped_door"
                | "minecraft:copper_door"
                | "minecraft:exposed_copper_door"
                | "minecraft:oxidized_copper_door"
                | "minecraft:weathered_copper_door"
                | "minecraft:waxed_copper_door"
                | "minecraft:waxed_exposed_copper_door"
                | "minecraft:waxed_oxidized_copper_door"
                | "minecraft:waxed_weathered_copper_door"
        );
        let bed = matches!(
            state.name.as_str(),
            "minecraft:white_bed"
                | "minecraft:orange_bed"
                | "minecraft:magenta_bed"
                | "minecraft:light_blue_bed"
                | "minecraft:yellow_bed"
                | "minecraft:lime_bed"
                | "minecraft:pink_bed"
                | "minecraft:gray_bed"
                | "minecraft:light_gray_bed"
                | "minecraft:cyan_bed"
                | "minecraft:purple_bed"
                | "minecraft:blue_bed"
                | "minecraft:brown_bed"
                | "minecraft:green_bed"
                | "minecraft:red_bed"
                | "minecraft:black_bed"
        );
        let mut offset = [0; 3];
        if lower && property("half") == Some("upper") {
            offset[1] = -1;
        } else if bed && property("part") == Some("foot") {
            offset = match property("facing") {
                Some("north") => [0, 0, -1],
                Some("south") => [0, 0, 1],
                Some("east") => [1, 0, 0],
                Some("west") => [-1, 0, 0],
                _ => [0; 3],
            };
        }
        Self { offset }
    }
    pub(super) fn at(self, pos: (i32, i32, i32), multipart: bool) -> i64 {
        let seed = position_seed([
            pos.0.wrapping_add(self.offset[0]),
            pos.1.wrapping_add(self.offset[1]),
            pos.2.wrapping_add(self.offset[2]),
        ]);
        if multipart {
            LegacyRandom::new(seed).next_long()
        } else {
            seed
        }
    }
}
pub(super) fn position_seed([x, y, z]: [i32; 3]) -> i64 {
    position_seed_legacy([x, y, z]) >> 16
}
/// 旧 MathHelper 返回完整 long，截断在旧加权模型内执行。
pub(super) fn position_seed_legacy([x, y, z]: [i32; 3]) -> i64 {
    // x 的乘法先按 Java int 溢出，再提升为 long；整体保持有符号右移。
    let n = i64::from(x.wrapping_mul(3_129_871))
        ^ i64::from(z).wrapping_mul(116_129_781)
        ^ i64::from(y);
    n.wrapping_mul(n)
        .wrapping_mul(42_317_861)
        .wrapping_add(n.wrapping_mul(11))
}
pub(super) fn legacy_weighted_ticket(seed: i64, total: u32) -> u32 {
    ((seed as i32 >> 16).abs() as u32) % total
}
pub(super) enum OrderedOptions<'a> {
    Source(&'a [serde_json::Value]),
    Legacy(Vec<&'a serde_json::Value>),
}
impl<'a> OrderedOptions<'a> {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Source(values) => values.len(),
            Self::Legacy(values) => values.len(),
        }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &'a serde_json::Value> + '_ {
        (0..self.len()).map(move |i| match self {
            Self::Source(values) => &values[i],
            Self::Legacy(values) => values[i],
        })
    }
}
/// 旧加权模型按权重稳定降序排序；现代直接借用原始 JSON，不再分配排序列表。
pub(super) fn ordered_options(
    value: &serde_json::Value,
    legacy: bool,
) -> Option<OrderedOptions<'_>> {
    let array = value.as_array()?;
    if !legacy {
        return Some(OrderedOptions::Source(array));
    }
    let mut entries = array
        .iter()
        .map(|value| Some((weight(value)?, value)))
        .collect::<Option<Vec<_>>>()?;
    entries.sort_by_key(|(weight, _)| std::cmp::Reverse(*weight));
    Some(OrderedOptions::Legacy(
        entries.into_iter().map(|(_, value)| value).collect(),
    ))
}
pub(super) fn weighted_ticket(seed: i64, total: u32) -> u32 {
    let value = (LegacyRandom::new(seed).next_long() as i32).wrapping_abs() % total as i32;
    // Math.abs(MIN_VALUE) 仍为负；原版区间遍历因此返回首项，与下标 0 等价。
    value.max(0) as u32
}
pub(super) fn weight(value: &serde_json::Value) -> Option<u32> {
    let weight = value
        .get("weight")
        .map(serde_json::Value::as_u64)
        .unwrap_or(Some(1))?;
    (weight > 0 && weight <= i32::MAX as u64).then_some(weight as u32)
}
