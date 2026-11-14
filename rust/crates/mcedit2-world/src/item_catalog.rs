//! 准确客户端公开注册表导出的物品定义；桌面显示与脚本使用同一份来源。
/// 只返回已核实的数据版本，不将相近版本或资源模型当作注册表。
pub fn json(version: i32) -> Option<&'static str> {
    match version {
        1343 => Some(include_str!("../assets/items/1343.json")),
        3465 => Some(include_str!("../assets/items/3465.json")),
        3955 => Some(include_str!("../assets/items/3955.json")),
        _ => None,
    }
}
