"""GFX 城市的原生表单迁移。

原始城市规则来自 Adrian Brightmoore，L-system 转换来自 David Vierra。
沿用原版实际可达的矩形废墟、楼板、房间隔断、玻璃及条带规则。
"""
from math import cos, sin, pi
from random import Random
from mcedit2.plugins import registerGeneratePlugin
from mcedit2.synth.l_system import Geometric, Fill, Line
from mcedit2.synth.l_system_plugin import LSystemPlugin
from mceditlib.geometry import Vector
from mceditlib.selection import BoundingBox


def Factorise(number):
    """返回正整数的全部整数因子，固定排序使种子行为不依赖集合迭代顺序。"""
    if type(number) is not int or number <= 0:
        raise ValueError("建筑分格需要正整数边长")
    factors, divisor = set(), 1
    while divisor * divisor <= number:
        if number % divisor == 0:
            factors.add(divisor)
            factors.add(number // divisor)
        divisor += 1
    return sorted(factors)


def polarToCartesian(angleHoriz, angleVert, distance):
    return Vector(cos(angleHoriz) * cos(angleVert) * distance,
                  sin(angleVert) * distance,
                  sin(angleHoriz) * cos(angleVert) * distance)


def _half_size(random, extent):
    upper = extent // 4
    if upper < 4:
        upper = 5
    size = max(4, random.randint(4, upper) / 2)
    if size > 8:
        size = 8 + random.randint(4, 8) if random.randint(1, 10) < 2 else 8
    return size


class City(Geometric):
    def replace(self):
        if self.width < 8 or self.length < 8 or self.height < 4 or self.width + self.length < 48:
            raise ValueError("城市选区宽、长至少为 8，高至少为 4，宽长之和至少为 48")
        random = self.random
        building_count = (self.width + self.length) // 24
        max_radius = self.width * 7 // 16
        for _ in range(1, building_count):
            radius = random.randint(0, max_radius)
            theta = random.random() * 2 * pi
            fraction = radius / max_radius
            height_limit = self.height if random.randint(1, 20) < 2 else max(4, int(self.height * (1 - fraction)))
            height = random.randint(4, height_limit)
            width = int(2 * _half_size(random, self.width))
            length = int(2 * _half_size(random, self.length))
            # 按实际可容纳建筑的椭圆域分布中心，不裁掉墙体或在来源之外补数据。
            direction = polarToCartesian(theta, 0, fraction)
            x = self.minx + int((self.width - width) * (1 + direction.x) / 2)
            z = self.minz + int((self.length - length) * (1 + direction.z) / 2)
            box = BoundingBox((x, self.miny, z), (width, height, length))
            yield RuinedBuilding(box, **self.parameters)


class RuinedBuilding(Geometric):
    def replace(self):
        random = self.random
        edge, fill, light = self.edgeBlockType, self.fillBlockType, self.lightBlockType
        # 原版也从高度因子中做一次选择；保留这次随机消耗及后续规则的调用顺序。
        width_divisor, _height_divisor, length_divisor = (
            random.choice(Factorise(extent - 1)) for extent in self.size)
        glass = random.randint(1, 20) > 1
        banded = random.randint(1, 20) < 10
        if banded:
            first_band = random.randint(2, 8)
            second_band = random.randint(1, first_band)
        band_type = 2 if random.randint(1, 20) < 5 else 1
        wall_height = self.height - 1

        for y in range(wall_height):
            if y == 0 or (y % 4 == 0 and random.randint(1, 10) > 1):
                yield Fill(BoundingBox((self.minx + 1, self.miny + y, self.minz + 1),
                                       (self.width - 1, 1, self.length - 1)), blocktype=fill)

        room_size = random.randint(6, 12)
        for x in range(self.width):
            if x % width_divisor == 0 or x % room_size == 0:
                yield Fill(BoundingBox((self.minx + x, self.miny, self.minz),
                                       (1, wall_height, self.length)), blocktype=edge)
        for z in range(self.length):
            if z % length_divisor == 0 or z % room_size == 0:
                yield Fill(BoundingBox((self.minx, self.miny, self.minz + z),
                                       (self.width, wall_height, 1)), blocktype=edge)

        if glass:
            if not banded:
                yield Walls(BoundingBox(self.origin, (self.width, wall_height, self.length)), blocktype=light)
            else:
                y = 0
                while y < wall_height:
                    first_height = min(first_band, wall_height - y)
                    material = edge if band_type == 1 else fill
                    yield Walls(BoundingBox((self.minx, self.miny + y, self.minz),
                                            (self.width, first_height, self.length)), blocktype=material)
                    y += first_height
                    if y < wall_height:
                        second_height = min(second_band, wall_height - y)
                        yield Walls(BoundingBox((self.minx, self.miny + y, self.minz),
                                                (self.width, second_height, self.length)), blocktype=light)
                        y += second_height

        for y in (self.miny, self.maxy - 1):
            corners = ((self.minx, y, self.minz), (self.maxx - 1, y, self.minz),
                       (self.maxx - 1, y, self.maxz - 1), (self.minx, y, self.maxz - 1))
            for start, end in zip(corners, corners[1:] + corners[:1]):
                yield Line(start, end, blocktype=edge)


class Walls(Geometric):
    def replace(self):
        # 高度由调用方确定；条带不再二次减去一层而丢失整条单层墙。
        for x in (self.minx, self.maxx - 1):
            yield Fill(BoundingBox((x, self.miny, self.minz), (1, self.height, self.length)), blocktype=self.blocktype)
        for z in (self.minz, self.maxz - 1):
            yield Fill(BoundingBox((self.minx, self.miny, z), (self.width, self.height, 1)), blocktype=self.blocktype)


@registerGeneratePlugin
class CityGeneratePlugin(LSystemPlugin):
    displayName = "GFX 城市"
    options = LSystemPlugin.options + [
        {"name": "seed", "text": "种子", "type": "int", "value": 0, "min": -(1 << 30), "max": 1 << 30, "increment": 1},
        {"name": "fill", "text": "楼板", "type": "blocktype", "value": "minecraft:stone"},
        {"name": "edge", "text": "框架", "type": "blocktype", "value": "minecraft:quartz_block"},
        {"name": "light", "text": "玻璃墙带", "type": "blocktype", "value": "minecraft:glass"},
    ]

    def createInitialSymbol(self, bounds):
        options = self.generateTool.options
        return City(bounds, random=Random(options["seed"]), fillBlockType=options["fill"],
                    edgeBlockType=options["edge"], lightBlockType=options["light"])
