"""原版参数化 L-system：符号规则留在脚本，直线栅格由共同 Rust 内核执行。"""


def _ls_work(symbols=0, commands=0, cells=0):
    if symbols * 512 + commands * 256 + cells * 128 > _available_work_budget():
        raise ValueError("L-system 符号、绘制命令及结构超过当前工作预算")


def _ls_symbols(values):
    result = []
    for index, symbol in enumerate(values):
        if index % 4096 == 0:
            _selection_cancel()
        if not isinstance(symbol, Symbol):
            raise TypeError("替换规则必须产生实际 Symbol")
        _ls_work(len(result) + 1)
        result.append(symbol)
    return result


def applyReplacements(symbol_list):
    original = _ls_symbols(symbol_list)
    replaced, result = False, []
    for symbol in original:
        _selection_cancel()
        if hasattr(symbol, "replace"):
            replaced = True
            values = symbol.replace()
        else:
            values = (symbol,)
        for index, value in enumerate(values):
            if index % 4096 == 0:
                _selection_cancel()
            if not isinstance(value, Symbol):
                raise TypeError("替换规则必须产生实际 Symbol")
            _ls_work(len(original) + len(result) + 1)
            result.append(value)
    return replaced, result


def applyReplacementsIterated(symbol_list, repeats):
    if type(repeats) is not int or repeats < 0:
        raise ValueError("L-system 迭代次数须为非负整数")
    symbols = _ls_symbols(symbol_list)
    for index in range(repeats):
        _selection_cancel()
        replaced, symbols = applyReplacements(symbols)
        if not replaced:
            break
        yield index, symbols


def renderBlocks(symbol_list):
    symbols = _ls_symbols(symbol_list)
    result = []
    for symbol in symbols:
        _selection_cancel()
        for index, command in enumerate(symbol.renderBlocks()):
            if index % 4096 == 0:
                _selection_cancel()
            x, y, z, block = command
            point = tuple(_grid_integer(value) for value in (x, y, z))
            _ls_work(len(symbols), len(result) + 1)
            result.append(point + (block,))
    return result


def renderSceneNodes(symbol_list):
    raise NotImplementedError("原生生成器使用实际方块结构候选预览，不生成旧 OpenGL SceneNode")


class Symbol:
    """参数化语法符号；不声明方块的非终结符本身不写入世界。"""
    def __init__(self, **parameters):
        self.parameters = parameters

    def __getattr__(self, key):
        parameters = object.__getattribute__(self, "parameters")
        if key in parameters:
            return parameters[key]
        raise AttributeError(key)

    def renderBlocks(self):
        return ()

    def renderSceneNodes(self):
        return renderSceneNodes((self,))

    def __repr__(self):
        return "%s(%r)" % (type(self).__name__, self.parameters)


class Geometric(Symbol, _BoundingBox):
    def __init__(self, box=None, **parameters):
        Symbol.__init__(self, **parameters)
        if box is None:
            origin = tuple(parameters[name] for name in ("minx", "miny", "minz"))
            size = tuple(parameters[name] for name in ("width", "height", "length"))
            _BoundingBox.__init__(self, origin, size)
        else:
            _BoundingBox.__init__(self, box.origin, box.size)


class Fill(Geometric):
    def __init__(self, box=None, blocktype=None, **parameters):
        if blocktype is not None:
            parameters["blocktype"] = blocktype
        super().__init__(box, **parameters)

    def renderBlocks(self):
        _ls_work(commands=self.volume)
        block = self.blocktype
        for index, point in enumerate(self.positions):
            if index % 4096 == 0:
                _selection_cancel()
            yield tuple(point) + (block,)


def bresenham(p1, p2):
    points = _mcedit2_plot_line({"start": list(p1), "end": list(p2)})
    for index, point in enumerate(points):
        if index % 4096 == 0:
            _selection_cancel()
        yield tuple(point)


class Line(Symbol):
    def __init__(self, p1, p2, **parameters):
        parameters.setdefault("glColor", (255, 64, 64, 128))
        super().__init__(**parameters)
        self.p1, self.p2 = _Vector(*p1), _Vector(*p2)

    def renderBlocks(self):
        block = self.blocktype
        for point in bresenham(self.p1, self.p2):
            yield point + (block,)


class LSystemPlugin(GeneratePlugin):
    recursive = True
    MAX_ITERATIONS = 50
    options = [{"name": "iterations", "text": "迭代次数", "type": "int", "value": 3,
                "min": 1, "max": 50, "increment": 1}]

    def __init__(self, generateTool):
        super().__init__(generateTool)
        self.optionsWidget = None
        self.iterationsSlider = _FormSpinSlider(minimum=1, maximum=self.MAX_ITERATIONS, value=3)
        self.iterationsSlider.setObjectName("iterations")
        self.iterationsSlider.valueChanged.connect(self.updatePreview)

    def createInitialSymbol(self, bounds):
        raise NotImplementedError("L-system 生成器必须提供 createInitialSymbol")

    def createSymbolList(self, bounds, indefinite=False):
        symbols = _ls_symbols((self.createInitialSymbol(bounds),))
        repeats = self.MAX_ITERATIONS if indefinite else self.generateTool.options["iterations"]
        for _, symbols in applyReplacementsIterated(symbols, repeats):
            _selection_cancel()
        return symbols

    def renderBlocks(self, symbol_list):
        return renderBlocks(symbol_list)

    def renderSceneNodes(self, symbol_list):
        return renderSceneNodes(symbol_list)

    def getPreviewNode(self, bounds):
        raise NotImplementedError("L-system 原生预览由实际生成结构提供，不返回 OpenGL 节点")

    def generateInSchematic(self, dimension, originalBounds):
        symbols = self.createSymbolList(originalBounds, indefinite=not self.recursive)
        commands = self.renderBlocks(symbols)
        pending = {}
        for index, (x, y, z, block) in enumerate(commands):
            if index % 4096 == 0:
                _selection_cancel()
            _ls_work(len(symbols), index + 1, len(dimension.blocks))
            point = tuple(_grid_integer(value) - origin for value, origin in zip((x, y, z), originalBounds.origin))
            at = dimension._index(*point)
            pending[at] = point, dimension.blocktypes[block]
        _selection_cancel()
        for at, (point, block) in pending.items():
            if dimension.blocks[at] != block:
                dimension.blocks[at] = block
                dimension._dirty_chunks.add((point[0] // 16, point[2] // 16))


_synth_module = _module("mcedit2.synth", True)
_lsystem_module = _module("mcedit2.synth.l_system")
for _name in ("Symbol", "Geometric", "Fill", "Line", "applyReplacements", "applyReplacementsIterated", "renderBlocks", "renderSceneNodes"):
    setattr(_lsystem_module, _name, globals()[_name])
_module("mcedit2.synth.l_system_plugin").LSystemPlugin = LSystemPlugin
_module("mcedit2.util.bresenham").bresenham = bresenham
