"""用原生结构 API 生成实心方块体；预览与暂存由宿主执行。"""
from mcedit2.api import register_generator


@register_generator("生成实心结构", options=[
    {"name": "宽度", "type": "int", "value": 3, "min": 1, "max": 16},
    {"name": "高度", "type": "int", "value": 3, "min": 1, "max": 16},
    {"name": "长度", "type": "int", "value": 3, "min": 1, "max": 16},
    {"name": "方块", "type": "blocktype", "value": "minecraft:stone"},
])
def solid_structure(context):
    size = (context.options["宽度"], context.options["高度"], context.options["长度"])
    count = size[0] * size[1] * size[2]
    context.log("生成 %d 个方块；结构锚点为当前选区最小坐标。" % count)
    return context.structure(size, [context.options["方块"]] * count)
