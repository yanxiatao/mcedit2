"""保留原 Simple Options Demo 的全部字段、默认值和选择值类型。"""
from mcedit2.api import register_command


@register_command("原版选项表单示例", options=[
    {"type": "label", "text": "这是一段会自动换行的选项说明。"},
    {"type": "label", "text": "多行说明\n第二行\n第三行"},
    {"type": "int", "name": "myIntOption", "text": "整数", "value": 50, "min": 0, "max": 75},
    {"type": "int", "name": "myIntOption2", "text": "整数（无上下限）", "value": 75},
    {"type": "float", "name": "myFloatOption", "text": "小数", "value": 1.5, "min": 0, "max": 7.5},
    {"type": "float", "name": "myFloatOption2", "text": "小数（无上下限）", "value": 2.5},
    {"type": "bool", "name": "myBoolOption", "text": "布尔选项"},
    {"type": "text", "name": "myTextOption", "text": "文本（提示）", "placeholder": "提示文本"},
    {"type": "text", "name": "myTextOption2", "text": "文本（默认值）", "value": "Default Text Value"},
    {"type": "choice", "name": "myChoiceOption", "text": "不同类型的选择值", "choices": [
        ("文本", "choice1"), ("整数", 50), ("小数", 4.5),
    ]},
    {"type": "blocktype", "name": "myBlocktypeOption", "text": "方块", "value": "minecraft:glass"},
])
def simple_options(context):
    for name in ("myIntOption", "myIntOption2", "myFloatOption", "myFloatOption2",
                 "myBoolOption", "myTextOption", "myTextOption2", "myChoiceOption", "myBlocktypeOption"):
        context.log("%s: %s" % (name, context.options[name]))
