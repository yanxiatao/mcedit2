"""把旧工具的节点树、选框和变换转换成原生覆盖层，不执行 OpenGL 或二进制渲染回调。"""
import math as _scene_math
import weakref as _scene_weakref

_tool_visual = None


def _scene_property(storage, default, convert=None):
    def read(self):
        return getattr(self, storage, default)
    def write(self, value):
        setattr(self, storage, value if convert is None else convert(value))
        self.dirty = True
    return property(read, write)


class _SceneNode:
    visible = _scene_property("_visible", True, bool)
    def __init__(self, name=None):
        self.name = name
        self._children = []
        self._parents = []
        self.states = []
        self._dirty = True
        self.visible = True
        self.childrenChanged = False
        self.descendentNeedsUpdate = False

    @property
    def children(self):
        return iter(self._children)

    def addParent(self, parent):
        if not self.hasParent(parent):
            self._parents.append(_scene_weakref.ref(parent))

    def removeParent(self, parent):
        self._parents[:] = [ref for ref in self._parents if ref() is not None and ref() is not parent]

    def hasParent(self, parent):
        return any(ref() is parent for ref in self._parents)

    def notifyParents(self):
        for ref in self._parents:
            parent = ref()
            if parent is not None:
                parent.descendentNeedsUpdate = True
                parent.notifyParents()

    def childrenDidChange(self):
        self.childrenChanged = True
        self.notifyParents()

    def _check_child(self, node):
        if not isinstance(node, _SceneNode):
            raise TypeError("场景子节点必须为 Node")
        pending, visited = [node], set()
        while pending:
            child = pending.pop()
            if child is self:
                raise ValueError("场景节点不能形成循环")
            if id(child) not in visited:
                visited.add(id(child))
                pending.extend(child.children)

    def addChild(self, node):
        self.insertChild(len(self._children), node)

    def insertChild(self, index, node):
        self._check_child(node)
        self._children.insert(index, node)
        node.addParent(self)
        self.childrenDidChange()

    def removeChild(self, node):
        self._children.remove(node)
        if node not in self._children:
            node.removeParent(self)
        self.childrenDidChange()

    def clear(self):
        for child in self.children:
            child.removeParent(self)
        self._children.clear()
        self.childrenDidChange()

    def childCount(self):
        return len(self._children)

    def addState(self, state):
        if not isinstance(state, _SceneState):
            raise TypeError("节点状态必须为 SceneNodeState")
        self.states.append(state)
        state.addParent(self)
        self.dirty = True

    def removeState(self, state):
        self.states.remove(state)
        if state not in self.states:
            state.removeParent(self)
        self.dirty = True

    @property
    def dirty(self):
        return self._dirty

    @dirty.setter
    def dirty(self, value):
        self._dirty = bool(value)
        if value:
            self.notifyParents()

    def _nested_repr(self, indent=0):
        yield " " * indent + repr(self)
        for child in self.children:
            yield from child._nested_repr(indent + 2)

    def nested_repr(self):
        return "\n".join(self._nested_repr())

    def __repr__(self):
        return "%s(%r, visible=%r, children=%d)" % (type(self).__name__, self.name, self.visible, self.childCount())


class _NamedSceneNode(_SceneNode):
    def __init__(self):
        super().__init__()
        self._children = {}

    @property
    def children(self):
        return iter(self._children.values())

    def addChild(self, name, node):
        self._check_child(node)
        old = self._children.get(name)
        self._children[name] = node
        if old is not None and old not in self._children.values():
            old.removeParent(self)
        node.addParent(self)
        self.childrenDidChange()

    def insertChild(self, index, node):
        raise TypeError("命名节点通过名称添加子节点，不提供顺序插入")

    def removeChild(self, name):
        node = self._children.pop(name)
        if node not in self._children.values():
            node.removeParent(self)
        self.childrenDidChange()

    def getChild(self, name):
        return self._children.get(name)


class _SceneState:
    def __init__(self):
        self._parents = []
        self._dirty = False

    def addParent(self, parent):
        self._parents.append(_scene_weakref.ref(parent))

    def removeParent(self, parent):
        self._parents[:] = [ref for ref in self._parents if ref() is not None and ref() is not parent]

    @property
    def dirty(self):
        return self._dirty

    @dirty.setter
    def dirty(self, value):
        self._dirty = bool(value)
        if value:
            for ref in self._parents:
                parent = ref()
                if parent is not None:
                    parent.dirty = True


class _SceneTranslate(_SceneState):
    translateOffset = _scene_property("_translateOffset", (0., 0., 0.), lambda value: _Vector(*_scene_vector(value, 3)))
    def __init__(self, translateOffset=(0., 0., 0.)):
        super().__init__()
        self.translateOffset = translateOffset


class _SceneScale(_SceneState):
    scale = _scene_property("_scale", (1., 1., 1.), lambda value: _scene_vector(value, 3))
    def __init__(self, scale=(1., 1., 1.)):
        super().__init__()
        self.scale = scale


class _SceneRotate(_SceneState):
    degrees = _scene_property("_degrees", 0., lambda value: _scene_vector((value,), 1)[0])
    axis = _scene_property("_axis", (0., 1., 0.), lambda value: _scene_vector(value, 3))
    def __init__(self, degrees=0., axis=(0., 1., 0.)):
        super().__init__()
        self.degrees, self.axis = degrees, axis


class _SceneIdentity(_SceneState):
    def __init__(self):
        super().__init__()


class _SceneLineWidth(_SceneState):
    lineWidth = _scene_property("_lineWidth", 2., lambda value: _scene_vector((value,), 1)[0])
    def __init__(self, lineWidth):
        super().__init__()
        self.lineWidth = lineWidth


class _SceneBox(_SceneNode):
    selectionBox = _scene_property("_selectionBox", None)
    depth = 0
    wire = _scene_property("_wire", True, bool)
    filled = _scene_property("_filled", True, bool)
    color = _scene_property("_color", (.3, .3, 1., .3), lambda value: _scene_vector(value, 4))
    wireColor = _scene_property("_wireColor", (.8, .8, .8, .6), lambda value: _scene_vector(value, 4))


class _SceneFace(_SceneNode):
    selectionBox = _scene_property("_selectionBox", None)
    depth = 0
    face = _scene_property("_face", 2)
    color = _scene_property("_color", (.3, .3, 1., .15), lambda value: _scene_vector(value, 4))
    wireColor = _scene_property("_wireColor", (.8, .8, .8, .6), lambda value: _scene_vector(value, 4))


def _scene_vector(value, count):
    result = tuple(value)
    if len(result) != count or any(type(v) not in (int, float) or not _scene_math.isfinite(v) for v in result):
        raise ValueError("场景坐标、颜色与变换需要指定维数的有限数值")
    return result


def _scene_identity():
    return tuple(tuple(float(row == column) for column in range(4)) for row in range(4))


def _scene_multiply(left, right):
    return tuple(tuple(sum(left[row][k] * right[k][column] for k in range(4)) for column in range(4)) for row in range(4))


def _scene_transform(state):
    matrix = [list(row) for row in _scene_identity()]
    if isinstance(state, _SceneTranslate):
        for axis, value in enumerate(_scene_vector(state.translateOffset, 3)):
            matrix[axis][3] = value
    elif isinstance(state, _SceneScale):
        for axis, value in enumerate(_scene_vector(state.scale, 3)):
            matrix[axis][axis] = value
    elif isinstance(state, _SceneRotate):
        x, y, z = _scene_vector(state.axis, 3)
        length = (x*x + y*y + z*z) ** .5
        if length == 0:
            raise ValueError("场景旋转轴不能为零")
        x, y, z = x/length, y/length, z/length
        degrees = _scene_vector((state.degrees,), 1)[0]
        c, s = _scene_math.cos(_scene_math.radians(degrees)), _scene_math.sin(_scene_math.radians(degrees))
        t = 1-c
        matrix[:3] = [[t*x*x+c, t*x*y-s*z, t*x*z+s*y, 0.],
                      [t*x*y+s*z, t*y*y+c, t*y*z-s*x, 0.],
                      [t*x*z-s*y, t*y*z+s*x, t*z*z+c, 0.]]
    else:
        raise TypeError("尚未转换的旧场景状态：" + type(state).__name__)
    return matrix


def _scene_primitives(root):
    output, active = [], set()
    def emit(points, closed, fill, stroke, width, matrix):
        transformed = [[sum(matrix[row][axis] * point[axis] for axis in range(3)) + matrix[row][3]
                        for row in range(3)] for point in points]
        primitive = {"points": transformed, "closed": closed, "fill": fill, "stroke": stroke, "width": width}
        _scene_vector((width,), 1)
        if width <= 0:
            raise ValueError("场景线宽必须大于零")
        for color in (fill, stroke):
            if color is not None and any(not 0 <= v <= 1 for v in _scene_vector(color, 4)):
                raise ValueError("场景颜色通道必须在 0..1 内")
        output.append(primitive)
        if len(output) * 4096 > _available_work_budget():
            raise ValueError("工具场景超过工作预算")

    def visit(node, matrix, width):
        _selection_cancel()
        if not isinstance(node, _SceneNode):
            raise TypeError("光标和覆盖层必须为原生可转换的场景节点")
        if id(node) in active:
            raise ValueError("场景节点存在循环")
        if not node.visible:
            return
        if hasattr(node, "RenderNodeClass"):
            raise TypeError("自定义 OpenGL RenderNode 需要转换为原生场景几何")
        active.add(id(node))
        for state in node.states:
            if isinstance(state, _SceneIdentity):
                matrix = _scene_identity()
            elif isinstance(state, _SceneLineWidth):
                width = state.lineWidth
            else:
                matrix = _scene_multiply(matrix, _scene_transform(state))
        if isinstance(node, (_SceneBox, _SceneFace)) and node.selectionBox is not None:
            box = node.selectionBox
            origin, size = _scene_vector(box.origin, 3), _scene_vector(box.size, 3)
            if any(value < 0 for value in size):
                raise ValueError("场景选框尺寸不能为负")
            corners = [tuple(origin[axis] + (size[axis] if mask & (1 << axis) else 0.) for axis in range(3)) for mask in range(8)]
            if isinstance(node, _SceneFace):
                if not isinstance(node.face, int) or isinstance(node.face, bool) or not 0 <= node.face < 6:
                    raise ValueError("场景面方向无效")
                face = int(node.face)
                axis = face // 2
                tangents = [value for value in range(3) if value != axis]
                masks = [((1 << axis) if face % 2 == 0 else 0)
                         | (a << tangents[0]) | (b << tangents[1]) for a, b in ((0,0), (1,0), (1,1), (0,1))]
                emit([corners[mask] for mask in masks], True, node.color, node.wireColor, width, matrix)
            else:
                if node.filled:
                    for axis in range(3):
                        tangents = [value for value in range(3) if value != axis]
                        for high in (0, 1):
                            masks = [(high << axis) | (a << tangents[0]) | (b << tangents[1]) for a,b in ((0,0), (1,0), (1,1), (0,1))]
                            emit([corners[mask] for mask in masks], True, node.color, None, width, matrix)
                if node.wire:
                    for mask in range(8):
                        for axis in range(3):
                            if not mask & (1 << axis):
                                emit([corners[mask], corners[mask | (1 << axis)]], False, None, node.wireColor, width, matrix)
        for child in node.children:
            visit(child, matrix, width)
        active.remove(id(node))
    if root is not None:
        visit(root, _scene_identity(), 2.)
    return output


_rendering = _module("mcedit2.rendering", True)
_scenegraph = _module("mcedit2.rendering.scenegraph", True)
_nodes = _module("mcedit2.rendering.scenegraph.scenenode")
_nodes.Node, _nodes.NamedChildrenNode = _SceneNode, _NamedSceneNode
_module("mcedit2.rendering.scenegraph.states").SceneNodeState = _SceneState
_transforms = _module("mcedit2.rendering.scenegraph.matrix")
_transforms.Translate, _transforms.Scale, _transforms.Rotate, _transforms.Identity = _SceneTranslate, _SceneScale, _SceneRotate, _SceneIdentity
_module("mcedit2.rendering.scenegraph.misc").LineWidth = _SceneLineWidth
_selection_nodes = _module("mcedit2.rendering.selection")
_selection_nodes.SelectionBoxNode, _selection_nodes.SelectionFaceNode = _SceneBox, _SceneFace
