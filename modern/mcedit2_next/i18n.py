"""Extensible UI localization for MCEdit2 Next.

Built in: English and Simplified Chinese. Further JSON packs can be placed
under ~/.mcedit2-next/locales/<language>.json (string keys and values only).
Translations never execute code or modify world files.
"""
from __future__ import annotations

from pathlib import Path
import json
from PySide6.QtCore import QLocale

ZH_CN = {
    "MCEdit2 Next — Python 3 / Qt6 experimental editor": "MCEdit2 Next — Python 3 / Qt6 实验版",
    "File": "文件", "Edit": "编辑", "Plugins": "插件", "Language": "语言",
    "View": "视图", "Tool": "工具", "Block": "方块",
    "Paint": "绘制", "Select": "选择",
    "2D slice": "二维切片", "3D OpenGL": "三维 OpenGL",
    "Selection Y min": "选择区域 Y 下限", "max": "上限",
    "Dimension": "维度", "3D chunk radius": "三维视距（区块）",
    "RAM cache": "内存缓存（Section）",
    "West": "西", "East": "东", "North": "北", "South": "南",
    "Open world…": "打开存档…",
    "Save…": "保存…",
    "Restore native region backup…": "恢复原生地区文件备份…",
    "Import structure (.nbt / .schem / .schematic)…": "导入结构文件（.nbt / .schem / .schematic）…",
    "Export selected structure (.nbt / .schem / .schematic)…": "导出选择区域为结构文件…",
    "Add resource pack / client JAR…": "添加材质包 / 客户端 JAR…",
    "Clear resource archives": "清除材质资源",
    "Undo": "撤销", "Redo": "重做", "Copy selection": "复制选区",
    "Copy with BE NBT (experimental)…": "复制方块实体 NBT（实验性）…",
    "Clone selection by offset…": "按坐标偏移克隆选区…",
    "Move selection by offset…": "按坐标偏移移动选区…",
    "Rotate selection clockwise 90°": "将选区顺时针旋转 90°",
    "Rotate selection counterclockwise 90°": "将选区逆时针旋转 90°",
    "Mirror selection East/West (X)": "选区沿东西方向镜像（X）",
    "Mirror selection North/South (Z)": "选区沿南北方向镜像（Z）",
    "Paste with BE NBT (experimental)…": "粘贴方块实体 NBT（实验性）…",
    "Hollow selection (shell)…": "空心填充选区（外壳）…",
    "Sphere at view center…": "在视图中心创建球体…",
    "Line between selection corners": "沿选区两角绘制直线",
    "Paste at center": "粘贴到视图中心", "Fill selection": "填充选区",
    "Flood-fill connected blocks within selection": "在选区内连通填充相同方块",
    "Replace selection": "替换选区方块",
    "Inspect BE at selection origin…": "查看选区起点的方块实体 NBT…",
    "Inspect entities in current chunk…": "检查当前区块的实体 NBT…",
    "Go to coordinates…": "跳转到坐标…",
    "Preload visible Sections": "预加载视野内的 Section",
    "Preload whole generated map…": "预加载整个已生成存档…",
    "Manage Plugins…": "管理插件…",
    "Brush modes": "画笔模式",
    "Default: selected block": "默认：所选方块",
    "Use only a disposable copy of a world.": "请仅在离线存档副本上进行测试。",
}
BUILTIN = {"en_US": {}, "zh_CN": ZH_CN}


class Localizer:
    def __init__(self, language=None, user_dir=None):
        self.root = Path(user_dir) if user_dir is not None else (
            Path.home() / ".mcedit2-next" / "locales")
        initial = language or ("zh_CN" if
            QLocale.system().language() == QLocale.Language.Chinese else "en_US")
        self.language = initial
        self.user = {}
        self.set_language(initial)

    def available(self):
        names = set(BUILTIN)
        if self.root.is_dir():
            names.update(path.stem for path in self.root.glob("*.json")
                         if path.is_file())
        return tuple(sorted(names))

    def set_language(self, language):
        self.language = language
        self.user = {}
        filename = self.root / f"{language}.json"
        if filename.is_file():
            try:
                data = json.loads(filename.read_text(encoding="utf-8"))
                if isinstance(data, dict):
                    self.user = {str(k): v for k, v in data.items()
                                 if isinstance(k, str) and isinstance(v, str)}
            except (OSError, ValueError):
                self.user = {}

    def text(self, source):
        return self.user.get(source, BUILTIN.get(self.language, {}).get(source, source))
