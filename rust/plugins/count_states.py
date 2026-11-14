"""原生 Rust 重写版的 Python 命令接口示例：只读统计完整选区状态。"""
from mcedit2.api import register_command


@register_command("统计选区方块状态")
def count_states(context):
    counts = {}
    for position in context.selection.positions:
        state = str(context.dimension.getBlock(*position))
        counts[state] = counts.get(state, 0) + 1
    for state in sorted(counts):
        context.log("%s × %d" % (state, counts[state]))
