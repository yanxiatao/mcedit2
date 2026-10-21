"""Interactive OpenGL 3.3 voxel viewport for the Python 3 editor.

This is a functional 3D editing preview: basic solid-color voxel cubes,
face culling, orbit/zoom/pan, depth testing and precise voxel ray picking.
It does not yet render resource-pack textures or non-cubic block models.
"""
from __future__ import annotations

from array import array
import ctypes
import math

from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QSurfaceFormat
from PySide6.QtOpenGLWidgets import QOpenGLWidget

from .viewport3d_math import Camera, is_air, textured_mesh_blocks, trace_voxels
from .textureatlas import make_atlas
from .resourcepack import ResourceLibrary
from .core import Box
from .clone_preview import ghost_wireframe_vertices
from .selection_gizmo import (
    axis_ray_position, horizontal_plane_point, move_selection, ray_handle,
    resize_selection, selection_handles,
)


VERTEX_SHADER = """#version 330 core
layout (location=0) in vec3 position;
layout (location=1) in vec3 color;
layout (location=2) in vec2 atlasUV;
uniform mat4 viewMatrix;
uniform mat4 projectionMatrix;
out vec3 fragmentColor;
out vec2 texCoord;
void main() {
    fragmentColor = color;
    texCoord = atlasUV;
    gl_Position = projectionMatrix * viewMatrix * vec4(position, 1.0);
}
"""

FRAGMENT_SHADER = """#version 330 core
in vec3 fragmentColor;
in vec2 texCoord;
uniform sampler2D textureAtlas;
out vec4 outColor;
void main() {
    outColor = vec4(fragmentColor, 1.0) * texture(textureAtlas, texCoord);
}
"""


def preferred_gl_format() -> QSurfaceFormat:
    result = QSurfaceFormat()
    result.setVersion(3, 3)
    result.setProfile(QSurfaceFormat.OpenGLContextProfile.CoreProfile)
    result.setDepthBufferSize(24)
    result.setSamples(0)
    return result


class Voxel3DView(QOpenGLWidget):
    """The same clicked/strokeStarted/strokeFinished signals as SliceView."""

    clicked = Signal(int, int, int)
    strokeStarted = Signal()
    strokeFinished = Signal()
    selectedY = Signal(int)
    selectedYEnd = Signal(int)
    selectionChanged = Signal()
    selectionEditRequested = Signal(object, object)
    renderError = Signal(str)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setFormat(preferred_gl_format())
        self.setMinimumSize(640, 480)
        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)
        self.session = None
        self.overlay = None
        self.mode = "Paint"
        self.selection = None  # (x1,z1,x2,z2), compatible with SliceView
        self.anchor = None
        self.anchor_y = None
        self.center_x = 0
        self.center_z = 0
        self._y = 64
        self.y_lo = 64
        self.y_hi = 64
        self.yaw = math.radians(45)
        self.pitch = math.radians(27)
        self.distance = 30.
        self.radius = 8  # A fixed, bounded sample region; not an infinite world render.
        self._dirty = True
        self._vertex_count = 0
        self._line_count = 0
        self._program = None
        self._buffer = None
        self._vao = None
        self._texture = None
        self.resource_paths = []
        self.resource_pack = None
        self._last_mouse = None
        self._dragged = False
        self._gizmo_drag = None
        self._gl_error = None
        self._skipped = 0
        self.clone_preview_boxes = ()  # Read-only original coordinate boxes.

    @property
    def y(self):
        return self._y

    @y.setter
    def y(self, value):
        self._y = int(value)
        self.invalidate()

    @property
    def camera(self):
        return Camera(
            (float(self.center_x) + 0.5, float(self.y) + 0.5,
             float(self.center_z) + 0.5),
            self.yaw, self.pitch, self.distance)

    def set_resources(self, files):
        candidate = ResourceLibrary(files) if files else None
        previous = self.resource_pack
        self.resource_pack = candidate
        self.resource_paths = list(files)
        if previous is not None:
            previous.close()
        self.invalidate()

    def invalidate(self):
        self._dirty = True
        super().update()

    def update(self, *args):
        # Calls made from editor actions mean block data may have changed.
        self._dirty = True
        return super().update(*args)

    def initializeGL(self):
        try:
            from OpenGL.GL import (
                GL_ARRAY_BUFFER, GL_DYNAMIC_DRAW, GL_FLOAT, GL_FALSE,
                glBindBuffer, glBindVertexArray, glBufferData, glEnableVertexAttribArray,
                glGenBuffers, glGenVertexArrays, glVertexAttribPointer,
                glGenTextures, glBindTexture, glTexParameteri, GL_TEXTURE_2D,
                GL_TEXTURE_MIN_FILTER, GL_TEXTURE_MAG_FILTER, GL_NEAREST,
                GL_TEXTURE_WRAP_S, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE,
            )
            from OpenGL.GL.shaders import compileProgram, compileShader
            from OpenGL.GL import GL_VERTEX_SHADER, GL_FRAGMENT_SHADER
            self._program = compileProgram(
                compileShader(VERTEX_SHADER, GL_VERTEX_SHADER),
                compileShader(FRAGMENT_SHADER, GL_FRAGMENT_SHADER))
            self._vao = glGenVertexArrays(1)
            self._buffer = glGenBuffers(1)
            glBindVertexArray(self._vao)
            glBindBuffer(GL_ARRAY_BUFFER, self._buffer)
            glBufferData(GL_ARRAY_BUFFER, 0, None, GL_DYNAMIC_DRAW)
            glEnableVertexAttribArray(0)
            glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, 32, ctypes.c_void_p(0))
            glEnableVertexAttribArray(1)
            glVertexAttribPointer(1, 3, GL_FLOAT, GL_FALSE, 32, ctypes.c_void_p(12))
            glEnableVertexAttribArray(2)
            glVertexAttribPointer(2, 2, GL_FLOAT, GL_FALSE, 32, ctypes.c_void_p(24))
            glBindVertexArray(0)
            self._texture = glGenTextures(1)
            glBindTexture(GL_TEXTURE_2D, self._texture)
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST)
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST)
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE)
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE)
        except Exception as exc:
            self._gl_error = str(exc)
            self._program = None
            self.renderError.emit(self._gl_error)

    def set_clone_preview_boxes(self, boxes):
        """Request a 3D target-volume overlay, WITHOUT modifying chunks."""
        self.clone_preview_boxes = tuple(boxes)
        # Streaming subclass invalidate() does not rebuild decoded chunks.
        self.invalidate()

    def _selection_lines(self, target):
        """Show ordinary selection and any read-only target clone bounds."""
        ghost = ghost_wireframe_vertices(self.clone_preview_boxes, target)
        if self.selection is None:
            return ghost
        x1, z1, x2, z2 = self.selection
        a = (min(x1, x2), min(self.y_lo, self.y_hi), min(z1, z2))
        b = (max(x1, x2) + 1, max(self.y_lo, self.y_hi) + 1, max(z1, z2) + 1)
        p = [(a[0], a[1], a[2]), (b[0], a[1], a[2]),
             (b[0], a[1], b[2]), (a[0], a[1], b[2]),
             (a[0], b[1], a[2]), (b[0], b[1], a[2]),
             (b[0], b[1], b[2]), (a[0], b[1], b[2])]
        edges = [(0, 1), (1, 2), (2, 3), (3, 0),
                 (4, 5), (5, 6), (6, 7), (7, 4),
                 (0, 4), (1, 5), (2, 6), (3, 7)]
        result = []
        for i, j in edges:
            for corner in (p[i], p[j]):
                result.extend((corner[0] - target[0], corner[1] - target[1],
                               corner[2] - target[2], 1., .78, .14,
                               .5 / 32, .5 / 32))
        # Face-resize diamonds plus a cyan central translate cross. All
        # vertices feed the existing native 3D/streaming GL_LINES pipeline.
        if self.mode == "Select":
            box = Box.from_corners(
                (x1, self.y_lo, z1), (x2, self.y_hi, z2))
            for handle in selection_handles(box):
                center = handle.center
                is_move = handle.name == "move"
                color = (0.14, 1.0, 0.86) if is_move else (1.0, 0.45, 0.17)
                if is_move:
                    segments = [
                        (tuple(center[i] - (0.45 if i == axis else 0.)
                               for i in range(3)),
                         tuple(center[i] + (0.45 if i == axis else 0.)
                               for i in range(3))) for axis in range(3)]
                else:
                    axis = "xyz".index(handle.name[0])
                    plane = [i for i in range(3) if i != axis]
                    corners = []
                    for i, scale in ((plane[0], 0.38),
                                     (plane[1], 0.38),
                                     (plane[0], -0.38),
                                     (plane[1], -0.38)):
                        corner = list(center)
                        corner[i] += scale
                        corners.append(tuple(corner))
                    segments = [(corners[i], corners[(i + 1) % 4])
                                for i in range(4)]
                for start, end in segments:
                    for point in (start, end):
                        result.extend((
                            point[0] - target[0],
                            point[1] - target[1],
                            point[2] - target[2],
                            *color, .5 / 32, .5 / 32))
        result.extend(ghost)
        return result

    def _gizmo_ray(self, position):
        return self.camera.ray(position.x(), position.y(),
                               max(1, self.width()), max(1, self.height()))

    def _begin_gizmo(self, position, modifiers):
        """Begin manipulating the visual selection without touching blocks."""
        if self.selection is None or self.mode != "Select":
            return False
        x1, z1, x2, z2 = self.selection
        box = Box.from_corners((x1, self.y_lo, z1),
                               (x2, self.y_hi, z2))
        origin, direction = self._gizmo_ray(position)
        handle = ray_handle(origin, direction, selection_handles(box))
        if handle is None:
            return False
        vertical = (handle.name == "move" and
                    bool(modifiers & Qt.KeyboardModifier.ShiftModifier))
        axis = ("xyz".index(handle.name[0])
                if handle.name != "move" else 1 if vertical else None)
        if axis is not None:
            start = axis_ray_position(origin, direction,
                                      handle.center, axis)
        else:
            start = horizontal_plane_point(
                origin, direction, handle.center[1])
        if start is None:
            return False
        self._gizmo_drag = (box, handle, start, axis, vertical)
        return True

    def _update_gizmo(self, position):
        if self._gizmo_drag is None:
            return
        box, handle, start, axis, vertical = self._gizmo_drag
        origin, direction = self._gizmo_ray(position)
        if axis is None:
            current = horizontal_plane_point(origin, direction, handle.center[1])
            if current is None:
                return
            delta = (round(current[0] - start[0]), 0,
                     round(current[2] - start[2]))
            edited = move_selection(box, delta)
        else:
            current = axis_ray_position(origin, direction,
                                        handle.center, axis)
            if current is None:
                return
            displacement = round(current - start)
            if handle.name == "move":
                delta = [0, 0, 0]
                delta[axis] = displacement
                edited = move_selection(box, tuple(delta))
            else:
                edited = resize_selection(box, handle.name, displacement)
        if edited != box:
            self.selectionEditRequested.emit(edited.minimum, edited.maximum)

    def _build_mesh(self):
        if self.session is None:
            self._atlas_image, _ = make_atlas({})
            return [], 0, 0
        r = max(4, min(24, int(self.radius)))
        # Sample bounded by 48^3 positions. Never generate new chunks.
        cx, cy, cz = self.center_x, self.y, self.center_z
        blocks = {}
        skipped = 0
        getter = (self.overlay.get_block if self.overlay is not None
                  else lambda point: self.session.world.get_block(*point))
        for x in range(cx - r, cx + r):
            for z in range(cz - r, cz + r):
                for y in range(cy - r, cy + r):
                    try:
                        state = getter((x, y, z))
                    except (OSError, ValueError, KeyError, IndexError):
                        skipped += 1
                        continue
                    if not is_air(state):
                        blocks[(x, y, z)] = state
        self._skipped = skipped
        target = self.camera.target
        self._atlas_image, tiles = make_atlas(blocks, self.resource_pack)
        triangles = textured_mesh_blocks(blocks, target, tiles)
        selection = self._selection_lines(target)
        return triangles + selection, len(triangles) // 8, len(selection) // 8

    def paintGL(self):
        from OpenGL.GL import (
            GL_ARRAY_BUFFER, GL_COLOR_BUFFER_BIT, GL_DEPTH_BUFFER_BIT,
            GL_DEPTH_TEST, GL_DYNAMIC_DRAW, GL_FALSE, GL_FLOAT, GL_LINES,
            GL_TRIANGLES, GL_TEXTURE_2D, GL_TEXTURE0, GL_RGBA, GL_UNSIGNED_BYTE,
            glActiveTexture, glBindTexture, glTexImage2D, glPixelStorei,
            GL_UNPACK_ALIGNMENT, glUniform1i,
            glBindBuffer, glBindVertexArray, glBufferData,
            glClear, glClearColor, glDrawArrays, glEnable, glGetUniformLocation,
            glUniformMatrix4fv, glUseProgram, glViewport,
        )
        glViewport(0, 0, max(1, self.width()), max(1, self.height()))
        glClearColor(.080, .104, .135, 1.)
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
        if self._program is None:
            return
        if self._dirty:
            data, triangles, lines = self._build_mesh()
            self._vertex_count, self._line_count = triangles, lines
            raw = array("f", data).tobytes()
            glBindBuffer(GL_ARRAY_BUFFER, self._buffer)
            glBufferData(GL_ARRAY_BUFFER, len(raw), raw, GL_DYNAMIC_DRAW)
            image = self._atlas_image.convertToFormat(self._atlas_image.Format.Format_RGBA8888)
            glActiveTexture(GL_TEXTURE0)
            glBindTexture(GL_TEXTURE_2D, self._texture)
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1)
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, image.width(), image.height(),
                         0, GL_RGBA, GL_UNSIGNED_BYTE, image.bits().tobytes())
            self._dirty = False
        view, projection = self.camera.matrices(max(1, self.width()), max(1, self.height()))
        glEnable(GL_DEPTH_TEST)
        glUseProgram(self._program)
        glUniformMatrix4fv(glGetUniformLocation(self._program, "viewMatrix"),
                           1, GL_FALSE, (ctypes.c_float * 16)(*view))
        glUniformMatrix4fv(glGetUniformLocation(self._program, "projectionMatrix"),
                           1, GL_FALSE, (ctypes.c_float * 16)(*projection))
        glUniform1i(glGetUniformLocation(self._program, "textureAtlas"), 0)
        glActiveTexture(GL_TEXTURE0)
        glBindTexture(GL_TEXTURE_2D, self._texture)
        glBindVertexArray(self._vao)
        if self._vertex_count:
            glDrawArrays(GL_TRIANGLES, 0, self._vertex_count)
        if self._line_count:
            # Slight z fighting at the boundary is acceptable in this preview.
            glDrawArrays(GL_LINES, self._vertex_count, self._line_count)
        glBindVertexArray(0)
        glUseProgram(0)

    def _hit(self, pos):
        if self.session is None:
            return None
        # QWidget event coordinates and .width/.height are logical pixels.
        origin, direction = self.camera.ray(pos.x(), pos.y(),
                                           max(1, self.width()), max(1, self.height()))
        def solid(point):
            x, y, z = point
            if (abs(x - self.center_x) >= self.radius or
                    abs(y - self.y) >= self.radius or
                    abs(z - self.center_z) >= self.radius):
                return False
            try:
                state = (self.overlay.get_block(point) if self.overlay else
                         self.session.world.get_block(*point))
                return not is_air(state)
            except (OSError, ValueError, KeyError, IndexError):
                return False
        return trace_voxels(origin, direction, solid, limit=min(128., self.distance + 40))

    def mousePressEvent(self, event):
        self.setFocus()
        self._last_mouse = event.position()
        self._dragged = False
        if event.button() == Qt.MouseButton.LeftButton:
            if self.mode == "Select" and (
                    event.modifiers() & Qt.KeyboardModifier.AltModifier):
                # Alt+left mouse handles use only the view ray, so they also
                # work above empty void where voxel picking returns None.
                self._begin_gizmo(event.position(), event.modifiers())
                return
            hit = self._hit(event.position())
            if hit is None:
                return
            x, y, z = hit.position
            if self.mode == "Select":
                self.selection = (x, z, x, z)
                self.anchor = (x, z)
                self.anchor_y = y
                self.selectedY.emit(y)
                self.invalidate()
            else:
                self.strokeStarted.emit()
                # Ctrl+click places against the clicked face instead of replacing.
                if event.modifiers() & Qt.KeyboardModifier.ControlModifier:
                    x += hit.normal[0]
                    y += hit.normal[1]
                    z += hit.normal[2]
                self.clicked.emit(x, y, z)

    def mouseMoveEvent(self, event):
        if self._gizmo_drag is not None:
            if event.buttons() & Qt.MouseButton.LeftButton:
                self._update_gizmo(event.position())
            return
        if self._last_mouse is None:
            self._last_mouse = event.position()
        delta = event.position() - self._last_mouse
        self._last_mouse = event.position()
        if event.buttons() & Qt.MouseButton.RightButton:
            self.yaw -= float(delta.x()) * .008
            self.pitch = max(-1.43, min(1.43, self.pitch + float(delta.y()) * .008))
            super().update()
        elif event.buttons() & Qt.MouseButton.MiddleButton:
            self.center_x += int(-delta.x() * .06)
            self.center_z += int(delta.y() * .06)
            self.invalidate()
        elif event.buttons() & Qt.MouseButton.LeftButton:
            hit = self._hit(event.position())
            if hit is None:
                return
            x, y, z = hit.position
            if self.mode == "Select" and self.anchor is not None:
                self.selection = (*self.anchor, x, z)
                self.selectedYEnd.emit(y)
                self.invalidate()
            elif self.mode == "Paint":
                if event.modifiers() & Qt.KeyboardModifier.ControlModifier:
                    x += hit.normal[0]
                    y += hit.normal[1]
                    z += hit.normal[2]
                self.clicked.emit(x, y, z)

    def mouseReleaseEvent(self, event):
        if self._gizmo_drag is not None:
            if event.button() == Qt.MouseButton.LeftButton:
                self._gizmo_drag = None
                self._last_mouse = None
                self.selectionChanged.emit()
            return
        if event.button() == Qt.MouseButton.LeftButton:
            if self.mode == "Paint":
                self.strokeFinished.emit()
            elif self.mode == "Select" and self.anchor is not None:
                # Selection coordinates are shared with the 2D editor. Notify
                # the tool dock after a finished 3D drag, even for one block.
                self.selectionChanged.emit()
            self.anchor = None
            self.anchor_y = None
        self._last_mouse = None

    def wheelEvent(self, event):
        self.distance = min(200., max(3., self.distance * (0.86 ** (
            event.angleDelta().y() / 120.))))
        super().update()

    def keyPressEvent(self, event):
        delta = {
            Qt.Key.Key_W: (0, -1), Qt.Key.Key_S: (0, 1),
            Qt.Key.Key_A: (-1, 0), Qt.Key.Key_D: (1, 0),
            Qt.Key.Key_Up: (0, -1), Qt.Key.Key_Down: (0, 1),
            Qt.Key.Key_Left: (-1, 0), Qt.Key.Key_Right: (1, 0),
        }.get(event.key())
        if delta is not None:
            factor = 16 if event.modifiers() & Qt.KeyboardModifier.ShiftModifier else 3
            self.center_x += delta[0] * factor
            self.center_z += delta[1] * factor
            self.invalidate()
        elif event.key() in (Qt.Key.Key_PageUp, Qt.Key.Key_PageDown):
            self.y += 4 if event.key() == Qt.Key.Key_PageUp else -4
        else:
            super().keyPressEvent(event)
