//! 原版 MCEdit2 旧版数字状态和渲染映射。
/// 准确 1.12.2 api/aqk 通过 fa.a(int) 解码箱子方向，竖直面改为 north。
/// 供普通、陷阱和末影箱共享；不用于其他数字方块或版本。
pub fn chest_facing_1_12(metadata: u8) -> Option<&'static str> {
    if metadata > 15 {
        return None;
    }
    Some(match metadata % 6 {
        3 => "south",
        4 => "west",
        5 => "east",
        _ => "north",
    })
}
/// 客户端 e(IBlockState) 写出 fa.a() 的标准索引，消除去重别名的编码歧义。
pub fn chest_metadata_1_12(facing: &str) -> Option<u8> {
    match facing {
        "north" => Some(2),
        "south" => Some(3),
        "west" => Some(4),
        "east" => Some(5),
        _ => None,
    }
}
pub fn mapping(version: &str) -> Option<&'static str> {
    match version {
        "1.8" => Some(include_str!("../assets/legacy-render/1.8.json")),
        "1.9" => Some(include_str!("../assets/legacy-render/1.9.json")),
        "1.10" => Some(include_str!("../assets/legacy-render/1.10.json")),
        "1.11" => Some(include_str!("../assets/legacy-render/1.11.json")),
        "1.12" => Some(include_str!("../assets/legacy-render/1.12.json")),
        _ => None,
    }
}
