"""使用旧版 MCEdit2 SimpleCommandPlugin 接口替换当前选区的方块。"""
from mcedit2.plugins import registerPluginCommand, SimpleCommandPlugin


@registerPluginCommand
class ReplaceSelection(SimpleCommandPlugin):
    displayName = "按原始状态替换选区"
    options = [
        {"type": "blocktype", "name": "source", "text": "原始状态", "value": "minecraft:stone"},
        {"type": "blocktype", "name": "target", "text": "替换状态", "value": "minecraft:glass"},
    ]

    def perform(self, dimension, selection, options):
        count = 0
        for position in selection.positions:
            if dimension.getBlock(*position) == options["source"]:
                dimension.setBlock(*position, options["target"])
                count += 1
        print("已生成 %d 个方块的替换候选；请在编辑器预览后暂存。" % count)
