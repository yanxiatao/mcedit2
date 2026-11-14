"""原生工具与真实方块实体 Inspector 示例。"""
from mcedit2.api import register_inspector, register_tool
from . import legacy_line


@register_tool("点选方块并生成替换预览", options=[
    {"name": "方块", "type": "blocktype", "value": "minecraft:stone"},
])
def replace_clicked(context):
    event = context.tool_event
    x, y, z = event["hit"]
    context.log("%s 点选 (%d, %d, %d)，倍率 %s" % (event["view"], x, y, z, event["zoom"]))
    context.dimension.setBlock(x, y, z, context.options["方块"])


@register_tool("拖过方块并生成替换预览", gesture=True, persistent=True, options=[
    {"name": "方块", "type": "blocktype", "value": "minecraft:stone"},
])
def replace_dragged(context):
    positions = set()
    for sample in context.gesture["samples"]:
        location = sample["location"]
        # 空处与工作平面仅有几何信息，不作为真实方块替换目标。
        if location is not None and location["state"] is not None:
            positions.add(tuple(location["position"]))
    for x, y, z in sorted(positions):
        context.dimension.setBlock(x, y, z, context.options["方块"])
    context.state["手势次数"] = context.state.get("手势次数", 0) + 1
    context.log("%s 第 %d 次手势，采样 %d 次，替换 %d 个真实触点" % (
        context.gesture["view"], context.state["手势次数"],
        len(context.gesture["samples"]), len(positions)))


@register_inspector("编辑方块实体自定义名称", options=[
    {"name": "名称 JSON", "type": "text", "value": '{"text":"MCEdit2"}'},
])
def edit_custom_name(context):
    source = context.inspector
    fields = source["nbt"]["value"]
    values = [field["value"] for field in fields if field["name"] == "CustomName"]
    if len(values) > 1:
        raise ValueError("方块实体存在重复 CustomName")
    value = {"type": "string", "value": context.options["名称 JSON"]}
    context.log("方块实体位置 %s，源区块 SHA-256 %s" % (source["position"], source["source_sha"]))
    if values:
        if values[0]["type"] != "string":
            raise ValueError("CustomName 不是字符串，拒绝覆盖")
        return {"operation": "replace", "path": "/CustomName", "value": value}
    return {"operation": "insert_field", "path": "", "name": "CustomName", "value": value}
