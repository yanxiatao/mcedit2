//! 程序实际内嵌的界面图标；原生界面和插件宿主读取同一份源资源。
pub const TOOLBAR: &[(&str, &[u8])] = &[
    (
        "select_blocks",
        include_bytes!("../../../assets/toolbar/select_blocks.png"),
    ),
    ("brush", include_bytes!("../../../assets/toolbar/brush.png")),
    ("clone", include_bytes!("../../../assets/toolbar/clone.png")),
    ("move", include_bytes!("../../../assets/toolbar/move.png")),
    (
        "flood_fill",
        include_bytes!("../../../assets/toolbar/flood_fill.png"),
    ),
    (
        "edit_block",
        include_bytes!("../../../assets/toolbar/edit_block.png"),
    ),
    (
        "edit_entity",
        include_bytes!("../../../assets/toolbar/edit_entity.png"),
    ),
    (
        "edit_chunk",
        include_bytes!("../../../assets/toolbar/edit_chunk.png"),
    ),
    (
        "generate",
        include_bytes!("../../../assets/toolbar/generate.png"),
    ),
    (
        "history",
        include_bytes!("../../../assets/toolbar/history.png"),
    ),
    (
        "library",
        include_bytes!("../../../assets/toolbar/library.png"),
    ),
    (
        "edit_map",
        include_bytes!("../../../assets/toolbar/edit_map.png"),
    ),
    (
        "edit_player",
        include_bytes!("../../../assets/toolbar/edit_player.png"),
    ),
    (
        "edit_worldinfo",
        include_bytes!("../../../assets/toolbar/edit_worldinfo.png"),
    ),
    (
        "toolbar_text",
        include_bytes!("../../../assets/toolbar/toolbar_text.png"),
    ),
];

pub fn toolbar_icon(name: &str) -> Result<&'static [u8], String> {
    TOOLBAR
        .iter()
        .find(|(saved, _)| *saved == name)
        .map(|(_, bytes)| *bytes)
        .ok_or_else(|| format!("程序没有内嵌工具图标：{name}"))
}
