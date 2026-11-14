"""科赫雪花：保留三段公理和四段替换，使用实际选区内的原生方块预览。"""
from math import cos, sin, pi
from mcedit2.plugins import registerGeneratePlugin
from mcedit2.synth.l_system import Geometric, Line, Fill
from mcedit2.synth.l_system_plugin import LSystemPlugin
from mceditlib.geometry import Vector
from mceditlib.selection import BoundingBox

_COS, _SIN = cos(2 * pi / 3), sin(2 * pi / 3)


def rotatePoint(point):
    x, y, z = point
    return Vector(_COS * x - _SIN * z, y, _SIN * x + _COS * z)


class Side(Line):
    def replace(self):
        delta = (self.p2 - self.p1) / 3
        first, second = self.p1 + delta, self.p2 - delta
        spike = first + rotatePoint(-delta)
        for start, end in ((self.p1, first), (first, spike), (spike, second), (second, self.p2)):
            yield Side(start, end, **self.parameters)


class Snowflake(Geometric):
    def replace(self):
        center = Vector(self.center.x, self.miny + 0.5, self.center.z)
        radius = (min(self.width, self.length) - 1) / 2
        if radius == 0:
            yield Fill(BoundingBox(center.intfloor(), (1, 1, 1)), blocktype=self.blocktype)
            return
        point, points = Vector(radius, 0, 0), []
        for _ in range(3):
            points.append(center + point)
            point = rotatePoint(point)
        for first, second in zip(points, points[1:] + points[:1]):
            yield Side(first, second, blocktype=self.blocktype)


@registerGeneratePlugin
class KochSnowflakePlugin(LSystemPlugin):
    displayName = "科赫雪花"
    options = LSystemPlugin.options + [{"name": "block", "text": "方块", "type": "blocktype", "value": "minecraft:stone"}]

    def createInitialSymbol(self, bounds):
        return Snowflake(bounds, blocktype=self.generateTool.options["block"])
