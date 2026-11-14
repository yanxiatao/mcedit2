"""RustPython 的 MCEdit2 数据接口；不加载 Qt、CPython 或原版世界写入代码。"""
import sys
import math as _math
import builtins
import logging as _logging
from importlib.util import find_spec as _find_module_spec
from _frozen_importlib import ModuleSpec

_request = __mcedit2_request__
_messages = []
_commands = []
_classes = []
_tool_state = {}
_python_print = builtins.print


def _message(level, text):
    text = str(text)
    _messages.append({"level": level, "text": text})
    if "_mcedit2_message_hook" in globals():
        _mcedit2_message_hook({"level": level, "text": text})


def _print(*values, sep=" ", end="\n", file=None, flush=False):
    destination = sys.stdout if file is None else file
    if isinstance(destination, _RuntimeStream):
        sep = " " if sep is None else sep
        end = "\n" if end is None else end
        if not isinstance(sep, str) or not isinstance(end, str):
            raise TypeError("print 的 sep 和 end 须为字符串或 None")
        destination.write(sep.join(str(v) for v in values) + end)
        if flush:
            destination.flush()
    else:
        _python_print(*values, sep=sep, end=end, file=file, flush=flush)


def _task_result(value):
    """完成进度任务并保留其返回值；进度只保留最终状态，不占满普通日志。"""
    if not hasattr(value, "__next__"):
        return value
    count = 0
    last = None
    while True:
        try:
            last = next(value)
        except StopIteration as stopped:
            if count:
                _message("info", "插件进度：%s（共 %d 步）" % (last, count))
            return stopped.value
        count += 1


def exhaust(task):
    """保留原 mceditlib.util.exhaust 的语义：返回最后一次 yield，而非 return。"""
    last = None
    for value in task:
        last = value
    return last


builtins.print = _print


class _RuntimeStream:
    """非缓冲文本日志流；写入立即交给宿主，不暴露插件传输管道。"""
    encoding = "utf-8"
    errors = "strict"

    def __init__(self, level):
        self.level, self.closed = level, False

    def write(self, text):
        if self.closed:
            raise ValueError("日志流已关闭")
        if not isinstance(text, str):
            raise TypeError("文本日志流只能写入字符串")
        if text:
            _message(self.level, text)
        return len(text)

    def writelines(self, lines):
        for line in lines:
            self.write(line)

    def flush(self):
        if self.closed:
            raise ValueError("日志流已关闭")

    def close(self):
        self.closed = True

    def writable(self):
        return not self.closed

    def isatty(self):
        return False

    def fileno(self):
        raise OSError("宿主日志流不提供操作系统文件描述符")


class _RuntimeInput:
    """图形脚本没有交互终端；需要输入的代码正常报告，不能阻塞协议管道。"""
    encoding = "utf-8"
    errors = "strict"

    def read(self, size=-1):
        if size == 0:
            return ""
        raise RuntimeError("脚本没有交互终端输入，请使用命令选项或显式文件")

    readline = read

    def readlines(self, hint=-1):
        raise RuntimeError("脚本没有交互终端输入，请使用命令选项或显式文件")

    def isatty(self):
        return False

    def fileno(self):
        raise OSError("脚本输入不提供插件协议管道的文件描述符")


sys.stdout = sys.__stdout__ = _RuntimeStream("info")
sys.stderr = sys.__stderr__ = _RuntimeStream("error")
sys.stdin = sys.__stdin__ = _RuntimeInput()


def _module(name, package=False):
    module = type(sys)(name)
    if package:
        module.__path__ = []
    sys.modules[name] = module
    if "." in name:
        parent, child = name.rsplit(".", 1)
        setattr(sys.modules[parent], child, module)
    return module


class _RuntimeLogHandler(_logging.Handler):
    def emit(self, record):
        level = ("error" if record.levelno >= _logging.ERROR else
                 "warning" if record.levelno >= _logging.WARNING else
                 "info" if record.levelno >= _logging.INFO else "debug")
        _message(level, self.format(record))


_runtime_log_handler = _RuntimeLogHandler()
_runtime_log_handler.setFormatter(_logging.Formatter("%(name)s: %(message)s"))
_logging.getLogger().addHandler(_runtime_log_handler)
_logging.getLogger().setLevel(_logging.DEBUG)


class BlockType:
    def __init__(self, value):
        if isinstance(value, BlockType):
            self.name, self.properties = value.name, list(value.properties)
        elif isinstance(value, dict):
            self.name = value["name"]
            self.properties = [tuple(v) for v in value["properties"]]
        elif isinstance(value, str):
            name, bracket, rest = value.partition("[")
            if bracket:
                if not rest.endswith("]"):
                    raise ValueError("方块状态的属性缺少右括号")
                properties = [tuple(v.split("=", 1)) for v in rest[:-1].split(",")]
                if any(len(v) != 2 for v in properties):
                    raise ValueError("方块属性必须为 key=value")
            else:
                properties = []
            self.name, self.properties = name, sorted(properties)
        elif isinstance(value, int) and not isinstance(value, bool):
            self.name, self.properties = "numeric:" + str(value), [("data", "0")]
        else:
            raise TypeError("方块类型必须是状态字符串、数字 ID 或 BlockType")

    @property
    def internalName(self):
        return self.name

    @property
    def ID(self):
        if not self.name.startswith("numeric:"):
            raise ValueError("命名方块没有可推断的旧版数字 ID")
        return int(self.name[8:])

    @property
    def meta(self):
        if not self.name.startswith("numeric:"):
            raise ValueError("命名方块不能转换为旧版 metadata")
        return int(dict(self.properties)["data"])

    def __eq__(self, other):
        return isinstance(other, BlockType) and self.name == other.name and self.properties == other.properties

    def __hash__(self):
        return hash((self.name, tuple(self.properties)))

    def __str__(self):
        return self.name + ("[" + ",".join(k + "=" + v for k, v in self.properties) + "]" if self.properties else "")

    def state(self):
        return {"name": self.name, "properties": self.properties}


_legacy_tables = {}

def _legacy_block(value, release):
    block = BlockType(value)
    if block.name.startswith("numeric:"):
        return block
    if release is None:
        raise ValueError("数字来源缺少实际版本，无法解析原版方块注册表")
    if release not in _legacy_tables:
        registry = _mcedit2_legacy_registry(release)
        exact, defaults = {}, {}
        for entry in registry["entries"]:
            numeric_id, metadata, _resource, _variant, base, state = entry
            exact.setdefault(str(BlockType(state)), set()).add((numeric_id, metadata))
            if metadata == 0:
                defaults.setdefault(base, set()).add((numeric_id, metadata))
        _legacy_tables[release] = exact, defaults
    exact, defaults = _legacy_tables[release]
    key = str(block)
    matches = exact.get(key, set()) or (defaults.get(block.name, set()) if not block.properties else set())
    if len(matches) != 1:
        raise ValueError("旧版方块名称未知或对应多个数字状态：" + key)
    numeric_id, metadata = next(iter(matches))
    return BlockType("numeric:%d[data=%d]" % (numeric_id, metadata))


class _BlockTypes:
    def __init__(self, numeric=False, data_version=None, source_release=None):
        self.numeric, self.data_version = numeric, data_version
        self.source_release = source_release

    @property
    def itemTypes(self):
        return _item_types(self.data_version)

    @property
    def itemStackVersion(self):
        if self.data_version in (1343, 3465):
            return _VERSION_1_8
        raise ValueError("此版本不能表示为旧版 ItemStack 格式常量")

    def __getitem__(self, name):
        if isinstance(name, tuple):
            if not self.numeric or len(name) != 2 or any(not isinstance(v, int) or isinstance(v, bool) for v in name):
                raise TypeError("数字 ID/metadata 仅支持旧版数字世界")
            numeric_id, metadata = name
            if not 0 <= numeric_id <= 4095 or not 0 <= metadata <= 15:
                raise ValueError("数字 ID 或 metadata 超出范围")
            return BlockType("numeric:%d[data=%d]" % (numeric_id, metadata))
        block = _legacy_block(name, self.source_release) if self.numeric else BlockType(name)
        if not self.numeric and block.name.startswith("numeric:"):
            raise ValueError("命名方块来源没有可推断的旧版数字 ID 映射")
        return block


def _available_work_budget():
    return _request["work_budget_bytes"]


def _check_volume(size, retained=0):
    size = tuple(size)
    if len(size) != 3 or any(not isinstance(v, int) or isinstance(v, bool) or v <= 0 or v > 65535 for v in size):
        raise ValueError("结构尺寸必须是三个 1..65535 的整数")
    count = size[0] * size[1] * size[2]
    if (count + retained) * 2048 > _available_work_budget():
        raise ValueError("脚本密集选区或生成结构超过任务预算")
    return count


class SelectionBox:
    def __init__(self, origin=(0, 0, 0), size=(0, 0, 0), mask=None):
        self._origin, self._size = _Vector(*origin), _Vector(*size)
        if any(type(value) is not int for value in tuple(self.origin) + tuple(self.size)):
            raise TypeError("方块选区的起点和尺寸需要整数坐标")
        if any(value < 0 for value in self.size):
            raise ValueError("选区尺寸不能为负数")
        self._set_declared_mask(mask)

    def _set_declared_mask(self, mask):
        if mask is not None:
            _mask_count((self.volume,))
        if mask is not None and (len(mask) != self.volume or any(type(v) is not bool for v in mask)):
            raise ValueError("插件选区掩码无效")
        self._declared_mask = tuple(mask) if mask is not None else None

    origin = property(lambda self: self._origin)
    size = property(lambda self: self._size)
    minx = property(lambda self: self.origin[0])
    miny = property(lambda self: self.origin[1])
    minz = property(lambda self: self.origin[2])
    maxx = property(lambda self: self.origin[0] + self.size[0])
    maxy = property(lambda self: self.origin[1] + self.size[1])
    maxz = property(lambda self: self.origin[2] + self.size[2])
    width = property(lambda self: self.size[0])
    height = property(lambda self: self.size[1])
    length = property(lambda self: self.size[2])
    volume = property(lambda self: self.width * self.height * self.length)
    maximum = property(lambda self: self.origin + self.size)
    center = property(lambda self: self.origin + self.size * 0.5)

    @property
    def _mask(self):
        if type(self).__contains__ is SelectionBox.__contains__:
            return self._declared_mask
        return _SelectionMask(self)

    @property
    def positions(self):
        return ((x, y, z) for y in range(self.miny, self.maxy)
                for z in range(self.minz, self.maxz) for x in range(self.minx, self.maxx)
                if (x, y, z) in self)

    def __contains__(self, position):
        if not all(self.origin[i] <= position[i] < self.origin[i] + self.size[i] for i in range(3)):
            return False
        index = ((int(position[1] - self.miny) * self.length) + int(position[2] - self.minz)) * self.width + int(position[0] - self.minx)
        return self._declared_mask is None or self._declared_mask[index]


class Dimension:
    def __init__(self, snapshot):
        self._copy_context = 0
        count = _check_volume(snapshot["size"])
        if count != len(snapshot["blocks"]):
            raise ValueError("插件选区状态数量与尺寸不一致")
        self.selection = _BoundingBox(snapshot["source_min"], snapshot["size"], mask=snapshot.get("mask"))
        self._blocks = [BlockType(v) for v in snapshot["blocks"]]
        self.numeric = snapshot["numeric"]
        self.data_version = snapshot["data_version"]
        self.source_release = snapshot.get("source_release")
        self.blocktypes = _BlockTypes(self.numeric, self.data_version, self.source_release)
        self.dimName = snapshot["dimension"]
        self.entity_dimension = snapshot.get("entity_dimension")

    def _index(self, x, y, z):
        if any(not isinstance(v, int) or isinstance(v, bool) for v in (x, y, z)):
            raise TypeError("方块坐标必须为整数")
        if (x, y, z) not in self.selection:
            raise ValueError("插件只能访问本次传入的源选区")
        box = self.selection
        return ((y - box.miny) * box.length + z - box.minz) * box.width + x - box.minx

    def getBlock(self, x, y, z):
        return BlockType(self._blocks[self._index(x, y, z)])

    def setBlock(self, x, y, z, block):
        at = self._index(x, y, z)
        value = self.blocktypes[block]
        if self._blocks[at] != value:
            self._blocks[at] = value
            self._dirty_chunks.add((x // 16, z // 16))

    def getBlockID(self, x, y, z):
        return self.getBlock(x, y, z).ID

    def getBlockData(self, x, y, z):
        return self.getBlock(x, y, z).meta

    blockAt = getBlockID
    blockDataAt = getBlockData

    def setBlockID(self, x, y, z, value):
        current = self.getBlock(x, y, z)
        self.setBlock(x, y, z, "numeric:%d[data=%d]" % (value, current.meta))

    def setBlockData(self, x, y, z, value):
        current = self.getBlock(x, y, z)
        self.setBlock(x, y, z, "numeric:%d[data=%d]" % (current.ID, value))

    setBlockAt = setBlockID
    setBlockDataAt = setBlockData

class _FrozenRevision:
    """插件请求内的编辑范围；正式历史由完整差异候选一次发布。"""
    def __init__(self, label):
        if not isinstance(label, str) or not label:
            raise ValueError("编辑命令需要名称")
        self.label = label

    def __enter__(self):
        _selection_cancel()
        return self

    def __exit__(self, error_type, error, traceback):
        if error_type is None:
            _selection_cancel()
        return False


def _expose_application(application, application_type):
    application_type.app = application
    _module("mcedit2.editorapp").MCEditApp = application_type
    _module("PySide.QtGui").qApp = application
    _module("PySide.QtWidgets").qApp = application


class _SessionActions:
    def gotoDimension(self, name):
        dimension = self.worldEditor.getDimension(name)
        _native_tool_dimension(dimension)._call("use_dimension")
        self.worldEditor._dimension = name
        self._currentDimension = dimension
        self._currentSelection = _native_tool_dimension(dimension).selection

    def zoomToPoint(self, point, distance=15):
        _native_tool_dimension(self.currentDimension)._call("focus", position=list(point), distance=distance)

    def save(self):
        _native_tool_dimension(self.currentDimension)._call("save")

    def deleteSelection(self):
        return self._delete("all")

    def deleteBlocks(self):
        return self._delete("blocks")

    def deleteEntities(self):
        return self._delete("entities_without_items", ["TConstruct.FancyItem"])

    def deleteEntitiesAndItems(self):
        return self._delete("entities")

    def _delete(self, mode, keep_ids=()):
        if self.currentSelection is None:
            raise ValueError("请先设置实际选区")
        return _native_tool_dimension(self.currentDimension)._call("delete", region=_scope(self.currentSelection),
                                                                 mode=mode, keep_ids=list(keep_ids))

    def deselect(self):
        self.currentSelection = None

    def selectAll(self):
        self.currentSelection = _native_tool_dimension(self.currentDimension).bounds

    def _set_selection(self, value):
        if isinstance(value, SelectionBox) and value.volume == 0:
            value = None
        region = None if value is None else _scope(value)
        native = _native_tool_dimension(self.currentDimension)
        region = native._call("selection", region=region)
        self.worldEditor._selections[native.dimName] = region
        self._currentSelection = None if region is None else _ToolSelection(region)


class _EditorSession(_SessionActions):
    def __init__(self, dimension):
        self._currentDimension = dimension
        source = _request.get("native_source")
        self.filename = (source["document"]["root"] if source is not None else
                         _request["snapshot"]["source_root"] if _request["snapshot"] is not None else None)
        self._currentSelection = dimension.selection if dimension is not None else (
            _ContextSelection(_request["selection"]) if _request.get("selection") is not None else None)
        if _request.get("gesture") is not None or _request.get("tool_phase") is not None:
            self._currentSelection = _ContextSelection(_request["selection"]) if _request.get("selection") is not None else None
        self.worldEditor = (dimension._tool_native._world
                            if dimension is not None and hasattr(dimension, "_tool_native") else dimension)
        if isinstance(dimension, _ToolNativeDimension):
            self.worldEditor = dimension._world
        self.readonly = _request.get("read_only", False)
        if source is not None:
            self._currentDimension = self.worldEditor.getDimension()
            self._currentSelection = _native_tool_dimension(self._currentDimension).selection

    @property
    def currentDimension(self):
        return self._currentDimension

    @property
    def currentSelection(self):
        return self._currentSelection

    @currentSelection.setter
    def currentSelection(self, value):
        self._set_selection(value)

    def beginSimpleCommand(self, text):
        return _FrozenRevision(text)


def _add_command(identifier, name, options, callback):
    if any(c["id"] == identifier for c in _commands):
        raise ValueError("插件命令标识重复")
    _commands.append({"id": identifier, "name": str(name), "options": list(options), "callback": callback})


class _MenuAction:
    def __init__(self, command):
        self.command = command
        self.enabled = True

    def setEnabled(self, enabled):
        self.enabled = bool(enabled)

    def setText(self, text):
        self.command["name"] = str(text)


class CommandPlugin:
    def __init__(self, editorSession):
        self.editorSession = editorSession

    def tr(self, text):
        return text

    def addMenuItem(self, text, func, submenu=None):
        identifier = type(self).__name__ + ":" + str(len(_commands))
        name = str(text) if submenu is None else str(submenu) + "/" + str(text)
        _add_command(identifier, name, [], lambda dimension, selection, options: func())
        action = _MenuAction(_commands[-1])
        _commands[-1]["action"] = action
        return action


class SimpleCommandPlugin(CommandPlugin):
    options = []

    def __init__(self, editorSession):
        super().__init__(editorSession)
        _add_command(type(self).__name__, self.displayName, self.options, self.perform)


def registerPluginCommand(cls):
    if not issubclass(cls, CommandPlugin):
        raise TypeError("命令插件必须继承 CommandPlugin")
    _classes.append(cls)
    return cls


_generator_classes = []


class _GeneratedDimension(Dimension):
    @property
    def bounds(self):
        return self.selection


def _copy_generated_grid(grid):
    if grid is None:
        return None
    return dict(grid, size=list(grid["size"]), cells=list(grid["cells"]))


class _Schematic:
    def __init__(self, size, source, blocktypes, context, *, biomes=None, numeric_biomes=None, metadata=None, mask=None):
        self.size = tuple(size)
        count = _check_volume(self.size, len(source["blocks"]))
        snapshot = dict(source)
        snapshot["source_min"] = [0, 0, 0]
        snapshot["size"] = list(self.size)
        snapshot["mask"] = mask
        air = "numeric:0[data=0]" if source["numeric"] else "minecraft:air"
        snapshot["blocks"] = [BlockType(air).state() for _ in range(count)]
        self.dimension = _GeneratedDimension(snapshot)
        self._context = self.dimension._copy_context = context
        self.dimension.blocktypes = blocktypes
        _initialize_snapshot_objects(self.dimension, {"entities": [], "block_entities": [],
            "biomes": _copy_generated_grid(biomes), "numeric_biomes": _copy_generated_grid(numeric_biomes), "scheduled_ticks": []})
        self.offset = (0, 0, 0)
        self.metadata = None if metadata is None else metadata.copy()
        self.blocktypes = self.dimension.blocktypes
        self.data_version = source["data_version"]
        self.source_release = source.get("source_release")

    def createEntity(self, identifier):
        return self.dimension.createEntity(identifier)

    def getDimension(self, name=None):
        if name is not None and name != self.dimension.dimName:
            raise ValueError("生成结构只包含一个维度")
        return self.dimension

    def _clipboard(self):
        metadata = None if self.metadata is None else self.metadata._wire()
        return _mcedit2_generate_structure(self._context, {"size": list(self.size),
            "blocks": [block.state() for block in self.dimension.blocks],
            "content": _snapshot_content(self.dimension), "offset": list(self.offset),
            "mask": None if self.dimension.selection._mask is None else list(self.dimension.selection._mask),
            "metadata": metadata})


def _schematic_source(blocktypes):
    if _request["snapshot"] is None:
        raise ValueError("createSchematic 需要当前实际文档快照")
    if blocktypes is not _session.currentDimension.blocktypes:
        raise ValueError("createSchematic 必须使用当前文档实际的 blocktypes，不接受未核实的注册表名称")
    return _request["snapshot"], 0


def createSchematic(shape, blocktypes, *, biomes=None, numeric_biomes=None, metadata=None, mask=None):
    """使用实际注册表及冻结来源创建独立局部结构，不写文件或推测版本转换。"""
    source, context = _schematic_source(blocktypes)
    return _Schematic(shape, source, blocktypes, context, biomes=biomes, numeric_biomes=numeric_biomes,
                      metadata=metadata, mask=mask)


class GeneratePlugin:
    options = []

    def __init__(self, generateTool):
        self.generateTool = generateTool

    @property
    def editorSession(self):
        return self.generateTool.editorSession

    def tr(self, text):
        return text

    def boundsChanged(self, bounds):
        return None

    def generate(self, bounds, blocktypes):
        result = createSchematic(bounds.size, blocktypes)
        returned = _task_result(self.generateInSchematic(result.getDimension(), bounds))
        if returned is not None:
            raise TypeError("generateInSchematic 应修改传入维度并返回 None")
        return result

    def generateInSchematic(self, dimension, originalBounds):
        raise NotImplementedError("生成器必须实现 generate 或 generateInSchematic")

    def generateInDimension(self, bounds, dimension):
        raise NotImplementedError("generate 返回 None 时必须实现直接维度生成回调")

    def updatePreview(self, *signal_args):
        self.generateTool.updatePreview()


def registerGeneratePlugin(cls):
    if not issubclass(cls, GeneratePlugin):
        raise TypeError("生成器必须继承 GeneratePlugin")
    _generator_classes.append(cls)
    return cls


class _NativeGenerateTool:
    def __init__(self, session, options):
        self.editorSession = session
        self.bounds = session.currentSelection
        self.options = options
        self.preview_requested = False

    def updatePreview(self, *signal_args):
        # 当前请求在应用参数和 boundsChanged 后生成一次完整候选；不递归进入生成器。
        self.preview_requested = True


def _install_generator(cls):
    generator = cls(_NativeGenerateTool(_session, {}))
    custom_options = any("getOptionsWidget" in parent.__dict__ for parent in cls.__mro__
                         if parent is not GeneratePlugin and parent is not object)
    widget = generator.getOptionsWidget() if custom_options else None
    if _session.currentSelection is not None:
        generator.boundsChanged(_session.currentSelection)
    declared = _normal_options(getattr(cls, "options", []))
    bindings = {}
    if widget is not None:
        fields, bindings = _native_form(widget, generator)
        declared = [spec for spec in declared if spec.get("name") not in bindings] + fields
    direct_methods = [name for name in ("generateInWorld", "generateInDimension")
                      if any(name in parent.__dict__ for parent in cls.__mro__ if parent is not GeneratePlugin and parent is not object)]
    # 原版调用方使用 generateInWorld，基类文档声明 generateInDimension；前者明确声明时沿用原调用顺序。
    # getPreviewNode 的轻量显示由宿主完整内容候选取代，不执行 Qt SceneNode。
    def invoke(dimension, bounds, options):
        generator.generateTool.options = options
        _native_form_apply(bindings, options)
        _native_form_action(bindings)
        if generator.generateTool.preview_requested:
            entry["preview_on_change"] = True
            generator.generateTool.preview_requested = False
        if widget is not None:
            fields, current = _native_form(widget, generator)
            entry["options"] = _normal_options([spec for spec in declared if spec.get("name") not in current] + fields)
            generator.generateTool.options = _option_values(entry["options"], {})
        result = _task_result(generator.generate(bounds, dimension.blocktypes))
        if result is None:
            if not direct_methods:
                raise TypeError("generate 返回 None，但没有声明直接维度生成回调")
            returned = _task_result(getattr(generator, direct_methods[0])(bounds, dimension))
            if returned is not None:
                raise TypeError("直接维度生成应修改传入快照并返回 None 或进度迭代器")
            return None
        if not isinstance(result, _Schematic):
            raise TypeError("generate 应返回 createSchematic 结构，或返回 None 并实现直接维度生成")
        return result._clipboard()
    _add_command("generator:" + cls.__name__, getattr(cls, "displayName", cls.__name__),
                 declared, invoke)
    entry = _commands[-1]
    entry["preview_on_change"] = widget is not None or generator.generateTool.preview_requested


_mcedit2 = _module("mcedit2", True)
_module("mcedit2.util", True)
_plugins = _module("mcedit2.plugins", True)
_command = _module("mcedit2.plugins.command")
for _target in (_plugins, _command):
    _target.CommandPlugin = CommandPlugin
    _target.SimpleCommandPlugin = SimpleCommandPlugin
_plugins.registerPluginCommand = registerPluginCommand
_plugins.registerGeneratePlugin = registerGeneratePlugin
_editortools = _module("mcedit2.editortools", True)
_generate = _module("mcedit2.editortools.generate")
_generate.GeneratePlugin = GeneratePlugin
_mceditlib = _module("mceditlib", True)
_blocktypes = _module("mceditlib.blocktypes", True)
_blocktypes.BlockType = BlockType
_selection_module = _module("mceditlib.selection")
_selection_module.SelectionBox = SelectionBox
_selection_module.BoundingBox = SelectionBox
_schematic_module = _module("mceditlib.schematic")
_schematic_module.createSchematic = createSchematic
_util_module = _module("mceditlib.util", True)
_util_module.exhaust = exhaust


class _MessageBox:
    @staticmethod
    def information(parent, title, text):
        _message("info", str(title) + ": " + str(text))
        return 0

    @staticmethod
    def warning(parent, title, text):
        _message("warning", str(title) + ": " + str(text))
        return 0

    @staticmethod
    def critical(parent, title, text):
        _message("error", str(title) + ": " + str(text))
        return 0


_pyside = _module("PySide", True)
_qtgui = _module("PySide.QtGui")
_qtgui.QMessageBox = _MessageBox


def _option_block_value(value):
    if isinstance(value, (tuple, list)):
        # 明确的数字 ID / metadata 仅编码为数字状态，不映射或猜测命名版本。
        return str(_BlockTypes(numeric=True)[tuple(value)])
    return str(BlockType(value))


def dictFromFilterTuple(item):
    """按原版命令选项格式转换元组，不把未知输入变成占位标签。"""
    name, value = item
    spec = {"name": name, "text": name}
    if isinstance(value, tuple):
        if not value:
            raise ValueError("旧式选项元组不能为空")
        if value[0] == "block":
            if len(value) != 2:
                raise ValueError("block 选项应为 ('block', 方块类型)")
            spec.update(type="blocktype", value=_option_block_value(value[1]))
            return spec
        if value[0] == "string":
            spec.update(type="text", value="")
            for argument in value[1:]:
                key, setting = argument.split("=", 1)
                if key not in ("width", "value"):
                    raise ValueError("未知 string 选项属性：" + key)
                spec[key] = setting
            return spec
        if all(isinstance(v, str) for v in value):
            spec.update(type="choice", choices=[(v, v) for v in value])
            return spec
        if len(value) not in (2, 3, 4) or not all(isinstance(v, (int, float)) and not isinstance(v, bool) for v in value):
            raise ValueError("数值选项应为 (最小值, 最大值)、(默认值, 最小值, 最大值) 或再加步长")
        default, low, high = (value[0], value[0], value[1]) if len(value) == 2 else value[:3]
        spec.update(type="int" if isinstance(default, int) else "float", value=default, min=low, max=high)
        if len(value) == 4:
            spec["increment"] = value[3]
        return spec
    if value == "label":
        spec["type"] = "label"
    elif value == "blocktype" or isinstance(value, BlockType):
        spec.update(type="blocktype", value="minecraft:stone" if value == "blocktype" else str(value))
    elif isinstance(value, bool):
        spec.update(type="bool", value=value)
    elif isinstance(value, int):
        spec.update(type="int", value=value)
    elif isinstance(value, float):
        spec.update(type="float", value=value)
    elif isinstance(value, str):
        spec.update(type="text", value="" if value == "string" else value)
    else:
        raise TypeError("未知旧式选项值：" + name)
    return spec


def optionsFromFilterInputs(inputs):
    return [dictFromFilterTuple(item) for item in inputs]


_command.dictFromFilterTuple = dictFromFilterTuple
_command.optionsFromFilterInputs = optionsFromFilterInputs


def _normal_options(items):
    result = []
    for item in items:
        if isinstance(item, tuple):
            item = dictFromFilterTuple(item)
        if not isinstance(item, dict) or item.get("type") not in ("label", "action", "int", "float", "bool", "text", "choice", "blocktype"):
            raise ValueError("插件选项类型不受支持")
        if item["type"] != "label" and (not isinstance(item.get("name"), str) or not item["name"]):
            raise ValueError("插件选项缺少名称")
        spec = dict(item)
        for key in ("enabled", "visible", "slider"):
            if key in spec and not isinstance(spec[key], bool):
                raise ValueError("插件选项的 " + key + " 必须为布尔值")
        if spec["type"] == "label" and (not isinstance(spec.get("text"), str) or not spec["text"]):
            raise ValueError("标签选项必须提供 text")
        if spec["type"] == "blocktype" and "value" in spec:
            spec["value"] = _option_block_value(spec["value"])
        result.append(spec)
    names = [v["name"] for v in result if v["type"] != "label"]
    if len(set(names)) != len(names):
        raise ValueError("插件选项名称重复")
    return result


def _option_values(specs, supplied):
    if set(supplied) - {v["name"] for v in specs if v["type"] not in ("label", "action")}:
        raise ValueError("插件收到未声明的选项")
    result = {}
    for spec in specs:
        kind = spec["type"]
        if kind in ("label", "action"):
            continue
        name = spec["name"]
        defaults = {"int": 0, "float": 0.0, "bool": False, "text": "", "blocktype": "minecraft:stone"}
        if kind == "choice":
            choices = spec.get("choices", [])
            if not choices:
                raise ValueError("选择选项必须提供 choices")
            values = [v[1] if isinstance(v, (tuple, list)) else v for v in choices]
            value = supplied.get(name, spec.get("value", values[0]))
            if value not in values:
                raise ValueError("无效的选项值：" + name)
        else:
            value = supplied.get(name, spec.get("value", defaults[kind]))
        if kind in ("int", "float"):
            if isinstance(value, bool) or not isinstance(value, (int, float)) or (kind == "int" and not isinstance(value, int)):
                raise ValueError("数值选项类型错误：" + name)
            if (isinstance(value, float) and not _math.isfinite(value)) or ("min" in spec and value < spec["min"]) or ("max" in spec and value > spec["max"]):
                raise ValueError("数值选项超出范围：" + name)
        if kind == "bool" and not isinstance(value, bool):
            raise ValueError("布尔选项类型错误：" + name)
        if kind == "text" and not isinstance(value, str):
            raise ValueError("文本选项无效：" + name)
        result[name] = _session.currentDimension.blocktypes[value] if kind == "blocktype" else value
    return result


class _Context:
    def __init__(self, dimension, options):
        self.dimension, self.selection, self.options = dimension, dimension.selection, options
        self.snapshot_selection = dimension.selection
        self.tool_event = _request.get("tool_event")
        self.gesture = _request.get("gesture")
        self.state = _tool_state
        if self.gesture is not None:
            self.selection = _session.currentSelection
        self.inspector = _request.get("inspector")
        self.form_action = _request.get("form_action")

    def log(self, text):
        _message("info", text)

    def structure(self, size, blocks, offset=(0, 0, 0), block_entities=(), entities=(), biomes=None, metadata=None, mask=None, numeric_biomes=None):
        size = tuple(size)
        if len(size) != 3 or any(not isinstance(v, int) or isinstance(v, bool) or v <= 0 or v > 65535 for v in size):
            raise ValueError("结构尺寸必须是三个 1..65535 的整数")
        offset = tuple(offset)
        if len(offset) != 3 or any(not isinstance(v, int) or isinstance(v, bool) or v < -2147483648 or v > 2147483647 for v in offset):
            raise ValueError("结构偏移必须是三个 i32 整数")
        count = _check_volume(size, len(self.dimension.blocks))
        values = []
        for value in blocks:
            if len(values) == count:
                raise ValueError("结构方块数量超过声明尺寸")
            values.append(self.dimension.blocktypes[value].state())
        if len(values) != size[0] * size[1] * size[2]:
            raise ValueError("结构方块数量与尺寸不一致")
        source = _request["snapshot"]
        return {"format": "MCEDIT2_RUST_STRUCTURE_CLIPBOARD", "version": 2, "source_root": source["source_root"],
                "dimension": source["dimension"], "data_version": source["data_version"],
                "entity_dimension": source.get("entity_dimension"), "painting_variants": source.get("painting_variants", {}),
                "source_release": source.get("source_release"), "source_bound": False, "source_file": None,
                "numeric": source["numeric"], "source_min": source["source_min"], "size": list(size),
                "blocks": values, "offset": list(offset), "block_entities": list(block_entities),
                "entities": list(entities), "biomes": biomes, "numeric_biomes": numeric_biomes, "metadata": metadata, "mask": mask}


def register_command(name=None, options=()):
    def register(function):
        _add_command(function.__name__, name or function.__name__, options,
                     lambda dimension, selection, values: function(_Context(dimension, values)))
        return function
    return register


def register_tool(name=None, options=(), *, gesture=False, persistent=False):
    """注册单点工具，或通过 gesture=True 接收完整真实指针手势的工具。"""
    if not isinstance(gesture, bool):
        raise TypeError("gesture 必须为布尔值")
    if not isinstance(persistent, bool):
        raise TypeError("persistent 必须为布尔值")
    def register(function):
        def invoke(dimension, selection, values):
            context = _Context(dimension, values)
            if persistent and not __mcedit2_tool_session__:
                raise ValueError("持久工具必须在激活期间的独立工具宿主中执行")
            if gesture:
                if context.gesture is None:
                    raise ValueError("手势工具需要完整的按下、拖动与松开输入")
            elif context.tool_event is None:
                raise ValueError("工具需要已激活并接收到源方块点选")
            return function(context)
        _add_command("tool:" + function.__name__, name or function.__name__, options, invoke)
        _commands[-1]["gesture_input"] = gesture
        _commands[-1]["persistent_tool"] = persistent
        return function
    return register


def register_inspector(name=None, options=()):
    """注册真实方块实体 Inspector；返回 NBT 操作或操作列表，只读查看返回 None。"""
    def register(function):
        def invoke(dimension, selection, values):
            context = _Context(dimension, values)
            if context.inspector is None:
                raise ValueError("Inspector 需要选中的真实方块实体")
            return function(context)
        _add_command("inspector:" + function.__name__, name or function.__name__, options, invoke)
        return function
    return register


def register_generator(name=None, options=(), bounds_changed=None):
    """注册原生结构生成器；每次预览以选区触发 bounds_changed。"""
    if bounds_changed is not None and not callable(bounds_changed):
        raise TypeError("bounds_changed 必须可调用")
    def register(function):
        def run(dimension, selection, values):
            context = _Context(dimension, values)
            if bounds_changed is not None:
                bounds_changed(context, selection)
            return function(context)
        _add_command("generator:" + function.__name__, name or function.__name__, options, run)
        return function
    return register


def register_form(name=None, options=()):
    """注册使用宿主原生类型选项的可执行表单。"""
    def register(function):
        identifier = "form:" + function.__name__
        _add_command(identifier, name or function.__name__, options,
                     lambda dimension, selection, values: function(_Context(dimension, values)))
        return function
    return register


_api = _module("mcedit2.api")
_api.register_command = register_command
_api.register_tool = register_tool
_api.register_inspector = register_inspector
_api.register_nbt_editor = register_inspector
_api.register_generator = register_generator
_api.register_form = register_form
_api.create_structure = createSchematic


class _SnapshotImporter:
    """只导入宿主传入的源码快照，遵循 Python 模块缓存及包导入协议。"""
    def __init__(self, request):
        self.root = "mcedit2_user_plugin"
        if request["package"]:
            filename = request["filename"].replace("\\", "/")
            parts = filename.rsplit("/", 2)
            if len(parts) < 2 or parts[-1] != "__init__.py":
                raise ValueError("Python 包入口必须是实际包目录中的 __init__.py")
            self.root = parts[-2]
            if not self.root.isidentifier():
                raise ValueError("Python 插件包名不是有效模块标识：" + self.root)
        # 禁止把插件包装成宿主接口或已存在标准库；也不能把已有模块当作插件执行成功。
        if self.root in sys.modules or _find_module_spec(self.root) is not None:
            raise ValueError("插件模块名与运行时模块冲突，请重命名插件：" + self.root)
        self.sources = {self.root: {
            "source": request["source"], "filename": request["filename"],
            "is_package": request["package"],
        }}
        for name, source in request["modules"].items():
            if not all(part.isidentifier() for part in name.split(".")):
                raise ValueError("Python 模块标识无效：" + name)
            self.sources[self.root + "." + name] = source

    def find_spec(self, fullname, path=None, target=None):
        source = self.sources.get(fullname)
        if source is None:
            return None
        spec = ModuleSpec(fullname, self, origin=source["filename"],
                          is_package=source["is_package"])
        # 内存快照没有磁盘加载/字节码缓存位置；origin 仅用于错误定位。
        return spec

    def create_module(self, spec):
        # None 委托解释器按 Python 标准模块协议创建对象。
        return None

    def exec_module(self, module):
        source = self.sources[module.__name__]
        module.__file__ = source["filename"]
        exec(compile(source["source"], source["filename"], "exec"), module.__dict__)


_install_snapshot_objects()
