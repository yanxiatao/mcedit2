"""NBT 文件、二进制缓冲区和树辅助接口；实际编解码统一交给 Rust。"""
import os as _nbt_os


class LevelFormatError(Exception):
    """世界数据不符合实际保存格式。"""


class PlayerNotFound(KeyError):
    """实际存档中没有请求的玩家。"""


class ChunkNotPresent(KeyError):
    """实际存档中没有请求的区块，不用于表示快照未捕获。"""


class ChunkSizeError(ValueError):
    """区块超过所属保存格式的容量。"""


class RegionFormatError(LevelFormatError):
    """区域索引、扇区或压缩格式无效。"""


class ChunkFormatError(LevelFormatError):
    """区块载荷没有保存格式要求的字段。"""


class NBTFormatError(LevelFormatError):
    """已识别 NBT / 压缩流的格式或编码预算错误。"""


def _nbt_bytes(data):
    if not isinstance(data, (bytes, bytearray, memoryview)):
        raise TypeError("NBT 缓冲区需要 bytes、bytearray 或 memoryview")
    size = data.nbytes if isinstance(data, memoryview) else len(data)
    if size > min(64 * 1024 * 1024, _available_work_budget() // 4):
        raise ValueError("NBT 输入超过编码或当前工作预算")
    return bytes(data)


def _nbt_load(filename="", buf=None):
    limit = min(64 * 1024 * 1024, _available_work_budget() // 4)
    if filename:
        with open(_nbt_os.fspath(filename), "rb") as source:
            data = _nbt_bytes(source.read(limit + 1))
    elif hasattr(buf, "read"):
        data = _nbt_bytes(buf.read(limit + 1))
    else:
        data = _nbt_bytes(buf)
    try:
        result = _mcedit2_nbt_io("load", data, False)
    except ValueError as error:
        raise NBTFormatError(str(error)) from error
    tag = _tag_from_wire(result["value"])
    tag.name = result["name"]
    return tag


def _nbt_check_tree(tag):
    remaining = [(tag, 0)]
    work = nodes = 0
    budget = _available_work_budget()
    while remaining:
        node, depth = remaining.pop()
        nodes += 1
        if nodes % 4096 == 0:
            _selection_cancel()
        if depth > 64:
            raise ValueError("NBT 树超过 Rust 编解码器的递归深度")
        if not isinstance(node, _Tag):
            raise TypeError("NBT 树只接受类型化标签")
        work += 512 + len(node.name) * 8
        if isinstance(node, _Compound):
            remaining.extend((child, depth + 1) for child in node.values())
        elif isinstance(node, _List):
            remaining.extend((child, depth + 1) for child in node)
        elif isinstance(node, _Array):
            work += len(node) * 128
        elif node.tagID == 8:
            work += len(node.value) * 8
        if work > budget:
            raise ValueError("NBT 树超过当前工作预算")
    _selection_cancel()


def _nbt_save(self, filename_or_buf=None, compressed=True):
    if type(compressed) is not bool:
        raise TypeError("compressed 需要布尔值")
    _nbt_check_tree(self)
    data = _mcedit2_nbt_io("save", {"name": self.name, "value": self._wire()}, compressed)
    if filename_or_buf is None:
        return data
    _selection_cancel()
    if isinstance(filename_or_buf, (str, bytes)) or hasattr(filename_or_buf, "__fspath__"):
        with open(_nbt_os.fspath(filename_or_buf), "wb") as target:
            written = target.write(data)
    else:
        written = filename_or_buf.write(data)
    if written != len(data):
        raise OSError("NBT 输出流没有完整写入数据")


def _nbt_gunzip(data):
    return _mcedit2_nbt_io("gunzip", _nbt_bytes(data), True)


def _nbt_try_gunzip(data):
    return _mcedit2_nbt_io("gunzip", _nbt_bytes(data), False)


def _nbt_children(tag):
    if isinstance(tag, _Compound):
        return iter(tag.items())
    if isinstance(tag, _List):
        return enumerate(tag)
    return iter(())


def _nbt_walk(tag, path=None):
    if not isinstance(tag, _Tag):
        raise TypeError("walk 需要实际 NBT 标签")
    path = [] if path is None else path
    active = {id(tag)}
    pending = [(_nbt_children(tag), path, id(tag))]
    nodes = 0
    while pending:
        iterator, parent, owner = pending[-1]
        try:
            name, child = next(iterator)
        except StopIteration:
            pending.pop()
            active.remove(owner)
            continue
        nodes += 1
        if nodes % 4096 == 0:
            _selection_cancel()
        yield name, child, parent
        if isinstance(child, (_Compound, _List)):
            identity = id(child)
            if identity in active:
                raise ValueError("NBT 标签树存在循环引用")
            active.add(identity)
            pending.append((_nbt_children(child), parent + [name], identity))


def _nbt_nested_string(tag, indent_string="  ", indent=0):
    _nbt_check_tree(tag)
    if not isinstance(indent_string, str) or type(indent) is not int or indent < 0:
        raise ValueError("NBT 缩进需要字符串和非负整数")
    parts = []
    size = 0
    budget = _available_work_budget()

    def append(value):
        nonlocal size
        size += len(value) * 4
        if size > budget:
            raise ValueError("NBT 诊断文本超过当前工作预算")
        parts.append(value)

    def render(node, level):
        _selection_cancel()
        if isinstance(node, (_Compound, _List)):
            compound = isinstance(node, _Compound)
            append("TAG_Compound({\n" if compound else "TAG_List([\n")
            for name, child in _nbt_children(node):
                append(indent_string * (level + 1))
                if compound:
                    append("%r: " % name)
                render(child, level + 1)
                append(",\n")
            append(indent_string * level)
            append("})" if compound else "])")
        else:
            append("%s(%r)" % (type(node).__name__, node.value))

    if len(indent_string) * (indent + 65) * 4 > budget:
        raise ValueError("NBT 缩进超过当前工作预算")
    render(tag, indent)
    return "".join(parts)


def _nbt_hexdump(src, length=8):
    data = _nbt_bytes(src)
    if type(length) is not int or length <= 0:
        raise ValueError("hexdump 行宽需要正整数")
    lines = (len(data) + length - 1) // length
    if lines * (length * 4 + 24) * 4 > _available_work_budget():
        raise ValueError("hexdump 文本超过当前工作预算")
    result = []
    for offset in range(0, len(data), length):
        if offset % 4096 == 0:
            _selection_cancel()
        part = data[offset:offset + length]
        hexadecimal = " ".join("%02X" % value for value in part)
        printable = "".join(chr(value) if 32 <= value < 127 else "." for value in part)
        result.append("%04X   %-*s   %s\n" % (offset, length * 3, hexadecimal, printable))
    return "".join(result)


_nbt_module = sys.modules["mceditlib.nbt"]
NBTFormatError.__module__ = "mceditlib.nbt"
_nbt_module.NBTFormatError = NBTFormatError
_nbt_module.load = _nbt_load
_nbt_module.gunzip = _nbt_gunzip
_nbt_module.try_gunzip = _nbt_try_gunzip
_nbt_module.walk = _nbt_walk
_nbt_module.nested_string = _nbt_nested_string
_nbt_module.hexdump = _nbt_hexdump
_nbt_module.tag_classes = {0: None, **{cls.tagID: cls for cls in _tag_classes.values()}}
_exceptions_module = _module("mceditlib.exceptions")
for _exception in (LevelFormatError, PlayerNotFound, ChunkNotPresent, ChunkSizeError, RegionFormatError, ChunkFormatError):
    _exception.__module__ = "mceditlib.exceptions"
    setattr(_exceptions_module, _exception.__name__, _exception)
_Compound.save = _nbt_save
_Tag.__str__ = _nbt_nested_string
_List.tagClass = property(lambda self: _nbt_module.tag_classes[self.list_type])
