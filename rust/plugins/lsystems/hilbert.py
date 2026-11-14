"""二维希尔伯特曲线：保留原版四子格方向规则和最大二次幂范围。"""
from mcedit2.plugins import registerGeneratePlugin
from mcedit2.synth.l_system import Symbol, Geometric
from mcedit2.synth.l_system_plugin import LSystemPlugin
from mcedit2.util.bresenham import bresenham

_OFFSETS = ((0, 0), (0, 1), (1, 1), (1, 0))


class _Point(Symbol):
    """有限规则的终结格，不再重复替换自身。"""


class Cell(Symbol):
    def replace(self):
        if self.cellSize <= 2:
            yield _Point(origin=self.origin)
            return
        half = self.cellSize // 2
        indices = (self.initialIndex,) * 3 + ((self.initialIndex + 2) % 4,)
        forwards = (False, True, True, False) if self.forward else (True, False, False, True)
        offsets = _OFFSETS[self.initialIndex:] + _OFFSETS[:self.initialIndex]
        direction = 1 if self.forward else -1
        x, y, z = self.origin
        for index, (initial, forward) in enumerate(zip(indices, forwards)):
            dx, dz = offsets[direction * index]
            yield Cell(origin=(x + dx * half, y, z + dz * half), cellSize=half,
                       initialIndex=initial, forward=forward)


class HilbertCurveShell(Geometric):
    def replace(self):
        size = 1 << (min(self.width, self.length).bit_length() - 1)
        yield Cell(origin=tuple(self.origin), cellSize=size, initialIndex=0, forward=True)


@registerGeneratePlugin
class HilbertCurvePlugin(LSystemPlugin):
    displayName = "希尔伯特曲线"
    options = LSystemPlugin.options + [{"name": "block", "text": "方块", "type": "blocktype", "value": "minecraft:stone"}]

    def createInitialSymbol(self, bounds):
        return HilbertCurveShell(bounds)

    def renderBlocks(self, symbol_list):
        points = [symbol.origin for symbol in symbol_list if isinstance(symbol, (Cell, _Point))]
        block = self.generateTool.options["block"]
        for first, second in zip(points, points[1:]):
            for point in bresenham(first, second):
                yield point + (block,)
        if points:
            yield tuple(points[-1]) + (block,)
