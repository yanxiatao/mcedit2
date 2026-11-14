"""原 World Editing Demo：真实方块替换、猪生命值和僵尸保存名称。"""
import json

from mcedit2.api import register_command


@register_command("原版世界编辑示例", options=[
    {"type": "blocktype", "name": "replaceWith", "text": "将普通石头替换为", "value": "minecraft:glass"},
    {"type": "float", "name": "pigHealth", "text": "猪的生命值", "value": 10, "min": 0, "max": 200},
    {"type": "text", "name": "zombieName", "text": "僵尸的名称", "placeholder": "Dinnerbone"},
])
def world_editing(context):
    dimension, selection = context.dimension, context.selection
    entities = list(dimension.getEntities(selection))
    zombies = [entity for entity in entities if entity.id in ("Zombie", "minecraft:zombie")]
    name = context.options["zombieName"]
    if zombies:
        if dimension.data_version == 1343:
            saved_name = name
        elif dimension.data_version in (3465, 3955):
            saved_name = json.dumps({"text": name}, ensure_ascii=False)
        else:
            raise ValueError("未核实此来源版本的 CustomName 文本格式")
    stone = dimension.blocktypes["minecraft:stone"]
    changed_blocks = 0
    for position in selection.positions:
        if dimension.getBlock(*position) == stone:
            dimension.setBlock(*position, context.options["replaceWith"])
            changed_blocks += 1
    pigs = [entity for entity in entities if entity.id in ("Pig", "minecraft:pig")]
    for entity in pigs:
        entity.Health = context.options["pigHealth"]
    for entity in zombies:
        entity.CustomName = saved_name
    context.log("石头 %d 格，猪 %d 只，僵尸 %d 只；修改进入同一个候选。" %
                (changed_blocks, len(pigs), len(zombies)))
