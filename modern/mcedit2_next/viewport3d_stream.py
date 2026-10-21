"""Streaming OpenGL viewer: indexed chunks, background Section IO and GPU mesh cache.

Unlike the original prototype this never calls world.get_block in a triple
nested loop on the paint event. UI camera movement changes only matrices and
visible draw calls; NBT decode/mesh construction happens incrementally.
"""
from __future__ import annotations

from array import array
from collections import OrderedDict
from dataclasses import dataclass
import ctypes
import math
from PySide6.QtCore import QTimer, Signal
from OpenGL.GL import (
    GL_ARRAY_BUFFER, GL_COLOR_BUFFER_BIT, GL_DEPTH_BUFFER_BIT,
    GL_DEPTH_TEST, GL_DYNAMIC_DRAW, GL_STATIC_DRAW, GL_FALSE, GL_TRUE,
    GL_FLOAT, GL_BLEND, GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA,
    GL_LINES, GL_TRIANGLES, GL_TEXTURE_2D, GL_TEXTURE0,
    GL_RGBA, GL_UNSIGNED_BYTE, GL_UNPACK_ALIGNMENT,
    glActiveTexture, glBindBuffer, glBindTexture, glBindVertexArray,
    glBufferData, glClear, glClearColor, glDeleteBuffers,
    glDrawArrays, glEnable, glDisable, glBlendFunc, glDepthMask,
    glGenBuffers, glGetUniformLocation,
    glPixelStorei, glTexImage2D, glUniform1i, glUniform1f, glUniform3f,
    glUniformMatrix4fv, glUseProgram, glVertexAttribPointer,
    glViewport,
)
from OpenGL.GL.shaders import compileProgram, compileShader
from OpenGL.GL import GL_VERTEX_SHADER, GL_FRAGMENT_SHADER

from .viewport3d import (Voxel3DView, VERTEX_SHADER, FRAGMENT_SHADER)
from .viewport3d_math import is_air, textured_mesh_blocks, trace_voxels
from .streaming_atlas import StreamingAtlas
from .worldstream import SectionStreamer
from .meshworker import MeshBuildPool
from .render_layers import classify_translucent_states, section_depth_order

VERTEX = """#version 330 core
layout (location=0) in vec3 position;
layout (location=1) in vec3 color;
layout (location=2) in vec2 repeatUV;
layout (location=3) in float atlasTile;
uniform mat4 viewMatrix;
uniform mat4 projectionMatrix;
uniform vec3 sectionOffset;
out vec3 fragmentColor;
out vec2 texCoord;
flat out int tileNumber;
void main() {
    fragmentColor = color;
    texCoord = repeatUV;
    tileNumber = int(atlasTile + 0.5);
    gl_Position = projectionMatrix * viewMatrix *
                  vec4(position + sectionOffset, 1.0);
}
"""

FRAGMENT_GREEDY = """#version 330 core
in vec3 fragmentColor;
in vec2 texCoord;
flat in int tileNumber;
uniform sampler2D textureAtlas;
uniform float layerOpacity;
out vec4 outColor;
void main() {
    // Repeat a 16x16 tile across the merged rectangle, not one stretched
    // texture over the whole Section. The small inset avoids tile bleed.
    vec2 localUV = fract(texCoord + vec2(0.00001));
    vec2 atlasBase = vec2(float(tileNumber % 32), float(tileNumber / 32));
    vec2 atlasUV = (atlasBase + vec2(0.02) + 0.96 * localUV) / 32.0;
    vec4 texel = texture(textureAtlas, atlasUV);
    // Cut-out model materials (crossed plants, panes, iron bars) must not
    // write fully transparent texels to the depth buffer. True translucent
    // water/glass still needs a separately sorted/blended render pass.
    if (texel.a < 0.10) discard;
    outColor = vec4(fragmentColor, layerOpacity) * texel;
}
"""

@dataclass
class MeshGPU:
    buffer: int
    vertices: int
    blended_vertices: int = 0

    @property
    def opaque_vertices(self):
        return self.vertices - self.blended_vertices




class StreamingVoxel3DView(Voxel3DView):
    """Progressive 3D viewer with bounded CPU Section and GPU mesh caches."""
    progressChanged = Signal(str)

    def __init__(self, parent=None):
        self._stream = None
        self._editing_session = None
        super().__init__(parent)
        self.radius = 8  # CHUNKS, rather than the old 8 blocks.
        self.translucent_opacity = 0.72
        self.vertical_sections = 2  # 2 sections above/below current Y
        self.memory_sections = 1024
        self._atlas = StreamingAtlas()
        self._visible = ()
        self._view_signature = None
        self._gpu: OrderedDict[tuple, MeshGPU] = OrderedDict()
        self._retire = []
        self._queued: OrderedDict[tuple, bytes] = OrderedDict()
        self._remesh = set()
        self._empty_meshed = set()
        self._mesh_pool = MeshBuildPool(workers=2, max_pending=3)
        self._mesh_failures = set()
        self._atlas_revision = 0
        self._index_summary = ""
        self._preload_status = "idle"
        self._tick_index = 0
        self._timer = QTimer(self)
        self._timer.setInterval(45)
        self._timer.timeout.connect(self._tick)
        self._timer.start()

    @property
    def session(self):
        return self._editing_session

    @session.setter
    def session(self, value):
        if getattr(self, "_editing_session", None) is value:
            return
        previous = getattr(self, "_stream", None)
        if hasattr(self, "_mesh_pool"):
            self._mesh_pool.forget()
        if previous is not None:
            previous.close()
        self._stream = None
        self._editing_session = value
        if hasattr(self, "_gpu"):
            self._retire.extend(slot.buffer for slot in self._gpu.values())
            self._gpu.clear()
            self._queued.clear()
            self._remesh.clear()
            self._empty_meshed.clear()
            self._mesh_failures.clear()
            self._visible = ()
            self._view_signature = None
        if value is not None and hasattr(value.world, "section_blocks"):
            self._stream = SectionStreamer(
                value.world, capacity=self.memory_sections, workers=2,
                pending_limit=16)
            self._index_summary = (
                f"Indexed {len(self._stream.index.chunks):,} generated chunks "
                f"across {self._stream.index.regions:,} regions")
        if hasattr(self, "_timer"):
            self.update()

    def set_resources(self, files):
        super().set_resources(files)
        self._atlas = StreamingAtlas(self.resource_pack)
        self._mesh_pool.forget()
        self._mesh_failures.clear()
        self._atlas_revision = 0
        if hasattr(self, "_gpu"):
            self._retire.extend(slot.buffer for slot in self._gpu.values())
            self._gpu.clear()
            self._queued.clear()
            self._remesh.clear()
            self._empty_meshed.clear()
        super().update()

    def invalidate(self):
        """Camera/visibility changes do NOT invalidate decoded chunk geometry."""
        self._view_signature = None
        super(Voxel3DView, self).update()

    def update(self, *args):
        # Do not rebuild meshes for every mouse orbit, zoom or paint event.
        return super(Voxel3DView, self).update(*args)

    def invalidate_world_edits(self):
        if self._stream is not None:
            self._stream.forget()
        self._mesh_pool.forget()
        self._mesh_failures.clear()
        self._retire.extend(slot.buffer for slot in self._gpu.values())
        self._gpu.clear()
        self._queued.clear()
        self._remesh.clear()
        self._empty_meshed.clear()
        self.invalidate()

    def set_translucent_opacity(self, opacity):
        """Change only GL blending; no NBT reads or Section remeshing."""
        value = float(opacity)
        if not math.isfinite(value) or not 0.1 <= value <= 1.0:
            raise ValueError("Translucent opacity must be 0.1..1.0")
        self.translucent_opacity = value
        self.update()

    def set_cache_budget(self, sections):
        sections = max(32, min(int(sections), 8192))
        if sections == self.memory_sections:
            return
        self.memory_sections = sections
        if self._stream is not None:
            self._stream.capacity = sections
        self.invalidate()

    def initializeGL(self):
        super().initializeGL()
        if self._program is not None:
            try:
                self._program = compileProgram(
                    compileShader(VERTEX, GL_VERTEX_SHADER),
                    compileShader(FRAGMENT_GREEDY, GL_FRAGMENT_SHADER))
                from OpenGL.GL import glEnableVertexAttribArray
                glBindVertexArray(self._vao)
                glBindBuffer(GL_ARRAY_BUFFER, self._buffer)
                for i, n, offset in ((0, 3, 0), (1, 3, 12),
                                     (2, 2, 24), (3, 1, 32)):
                    glEnableVertexAttribArray(i)
                    glVertexAttribPointer(i, n, GL_FLOAT, GL_FALSE, 36,
                                          ctypes.c_void_p(offset))
                glBindVertexArray(0)
            except Exception as exc:
                self._program = None
                self._gl_error = str(exc)
                self.renderError.emit(self._gl_error)

    def _visible_keys(self):
        reader = self._stream
        if reader is None:
            return ()
        cx = self.center_x >> 4
        cz = self.center_z >> 4
        cy = self.y >> 4
        keys = []
        limit = min(self.memory_sections, 8192)
        layers = [cy] + [
            ly for dy in range(1, self.vertical_sections + 1)
            for ly in (cy + dy, cy - dy)]
        for chunk_x, chunk_z in reader.index.nearby(cx, cz, self.radius):
            for ly in layers:
                keys.append((chunk_x, ly, chunk_z))
                if len(keys) >= limit:
                    return tuple(keys)
        return tuple(keys)

    def _tick(self):
        reader = self._stream
        if reader is None or self._program is None or not self.isVisible():
            return
        sig = (self.center_x >> 4, self.center_z >> 4,
               self.y >> 4, self.radius,
               self.vertical_sections, self.memory_sections)
        if self._view_signature != sig:
            self._visible = self._visible_keys()
            self._view_signature = sig
            active = set(self._visible)
            for key, slot in list(self._gpu.items()):
                if key not in active:
                    self._retire.append(slot.buffer)
                    del self._gpu[key]
            for key in tuple(self._queued):
                if key not in active:
                    self._queued.pop(key, None)
            self._remesh.intersection_update(active)
            self._empty_meshed.intersection_update(active)
        self._tick_index += 1
        preload_status, done, total = reader.advance_full_preload()
        if preload_status != self._preload_status or (
                preload_status == "loading" and self._tick_index % 35 == 0):
            self._preload_status = preload_status
            if preload_status == "failed":
                self.progressChanged.emit(
                    "Full-world preload rejected: " + str(reader._preload_error))
            elif preload_status in ("scanning", "loading"):
                self.progressChanged.emit(
                    f"Whole-map preload {preload_status}: {done:,}/{total:,}")
            elif preload_status == "complete":
                self.progressChanged.emit(f"Loaded all {total:,} generated Sections")
        reader.request(self._visible)
        ready = reader.poll(limit=3)
        for mesh_key, data in self._mesh_pool.poll(limit=3):
            if mesh_key not in self._visible or mesh_key in self._remesh:
                # The Section moved outside the view or a neighbor has loaded
                # since the job started. Rebuild using current borders.
                continue
            if data is None:
                if mesh_key not in self._mesh_failures:
                    self._mesh_failures.add(mesh_key)
                    self.renderError.emit(
                        "Mesh build failed for Section " + str(mesh_key) +
                        ": " + self._mesh_pool.errors.get(mesh_key, "unknown error"))
                self._empty_meshed.add(mesh_key)
                continue
            if not data or (isinstance(data, tuple) and
                            not any(data)):
                self._empty_meshed.add(mesh_key)
            else:
                self._empty_meshed.discard(mesh_key)
            self._queued[mesh_key] = data
            self._queued.move_to_end(mesh_key)
        # Only mesh one Section per Qt tick: expensive work is rate-limited,
        # keeps window/input responsive even for very large loaded worlds.
        for key, result in ready:
            if key not in self._visible:
                continue
            if result is None:
                self._empty_meshed.add(key)
                continue
            self._remesh.add(key)
            cx, cy, cz = key
            for near in ((cx + 1, cy, cz), (cx - 1, cy, cz),
                         (cx, cy + 1, cz), (cx, cy - 1, cz),
                         (cx, cy, cz + 1), (cx, cy, cz - 1)):
                # Refresh borders, but don't remesh six adjacent sections
                # synchronously in the same UI tick.
                if near in self._gpu or near in self._empty_meshed:
                    self._remesh.add(near)

        # A Section can be cached on CPU but its GPU mesh may have been
        # evicted when moving away. Restore it without re-reading NBT.
        for key in self._visible:
            if (key in self._gpu or key in self._queued or
                    key in self._remesh or key in self._mesh_pool.pending or
                    key in self._mesh_failures):
                continue
            found, item = reader.get(key)
            if not found:
                continue
            if item is None:
                self._empty_meshed.add(key)
            elif key not in self._empty_meshed:
                self._remesh.add(key)

        if self._remesh:
            # The greedy mesh is computed on background worker threads;
            # Qt UI thread only resolves atlas tiles and schedules work.
            for key in self._visible:
                if key not in self._remesh or key in self._mesh_pool.pending:
                    continue
                found, item = reader.get(key)
                if found and item is not None:
                    if self._make_mesh(key, item.states):
                        self._remesh.discard(key)
                else:
                    self._remesh.discard(key)
                if len(self._mesh_pool.pending) >= self._mesh_pool.max_pending:
                    break
        if ready or self._queued:
            self.update()

    def _make_mesh(self, key, states):
        reader = self._stream
        cx, cy, cz = key
        neighbors = {}
        for offset in ((1, 0, 0), (-1, 0, 0),
                       (0, 1, 0), (0, -1, 0),
                       (0, 0, 1), (0, 0, -1)):
            neighbor_key = (cx + offset[0], cy + offset[1], cz + offset[2])
            found, node = reader.get(neighbor_key)
            neighbors[offset] = node.states if found and node else None
        # One stable atlas covers all visible chunks, so previous GPU meshes
        # never get invalid UVs as new chunks stream in.
        # Full cubes use the original merged-face path; multipart/rotated
        # JSON elements are decoded once per distinct BlockState. Neighbor
        # models also participate in face occlusion at Section boundaries.
        tiles = self._atlas.tiles_for(set(states))
        used_states = set(states)
        for adjacent in neighbors.values():
            if adjacent:
                used_states.update(adjacent)
        models = self._atlas.models_for(used_states)
        blended = classify_translucent_states(
            used_states, tiles, models, self._atlas.partial_alpha_tiles)
        return self._mesh_pool.submit(
            key, states, neighbors, tiles, models,
            translucent_states=blended, layered=True, fluids=True)

    def _release_buffers(self):
        if self._retire:
            glDeleteBuffers(len(self._retire), self._retire)
            self._retire.clear()

    def paintGL(self):
        glViewport(0, 0, max(1, self.width()), max(1, self.height()))
        glClearColor(.080, .104, .135, 1.)
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)
        if self._program is None:
            return
        self._release_buffers()
        while self._queued:
            key, data = self._queued.popitem(last=False)
            if key in self._gpu:
                self._retire.append(self._gpu.pop(key).buffer)
            if isinstance(data, tuple):
                opaque_bytes, blended_bytes = data
            else:
                # Compatible with any already queued legacy worker result.
                opaque_bytes, blended_bytes = data, b""
            if not opaque_bytes and not blended_bytes:
                continue
            payload = opaque_bytes + blended_bytes
            buffer = glGenBuffers(1)
            glBindBuffer(GL_ARRAY_BUFFER, buffer)
            glBufferData(GL_ARRAY_BUFFER, len(payload), payload, GL_STATIC_DRAW)
            self._gpu[key] = MeshGPU(
                buffer, len(payload) // 36, len(blended_bytes) // 36)
        self._vertex_count = sum(slot.vertices for slot in self._gpu.values())
        if self._atlas_revision != self._atlas.revision:
            image = self._atlas.gpu_image()
            glActiveTexture(GL_TEXTURE0)
            glBindTexture(GL_TEXTURE_2D, self._texture)
            glPixelStorei(GL_UNPACK_ALIGNMENT, 1)
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA,
                         image.width(), image.height(), 0, GL_RGBA,
                         GL_UNSIGNED_BYTE, image.bits().tobytes())
            self._atlas_revision = self._atlas.revision
        eye, forward, right, up = self.camera.frame()
        view, proj = self.camera.matrices(max(1, self.width()), max(1, self.height()))
        target = self.camera.target
        glEnable(GL_DEPTH_TEST)
        glUseProgram(self._program)
        glUniformMatrix4fv(glGetUniformLocation(self._program, "viewMatrix"),
                           1, GL_FALSE, (ctypes.c_float * 16)(*view))
        glUniformMatrix4fv(glGetUniformLocation(self._program, "projectionMatrix"),
                           1, GL_FALSE, (ctypes.c_float * 16)(*proj))
        offset_loc = glGetUniformLocation(self._program, "sectionOffset")
        glUniform1i(glGetUniformLocation(self._program, "textureAtlas"), 0)
        opacity_loc = glGetUniformLocation(self._program, "layerOpacity")
        glUniform1f(opacity_loc, 1.0)
        glDisable(GL_BLEND)
        glDepthMask(GL_TRUE)
        glActiveTexture(GL_TEXTURE0)
        glBindTexture(GL_TEXTURE_2D, self._texture)
        glBindVertexArray(self._vao)
        visible = set(self._visible)
        draw_items = {}
        for (cx, cy, cz), slot in self._gpu.items():
            if (cx, cy, cz) not in visible:
                continue
            center = (cx * 16 + 8, cy * 16 + 8, cz * 16 + 8)
            direction = tuple(center[i] - eye[i] for i in range(3))
            distance_sq = sum(value * value for value in direction)
            if distance_sq > 32 ** 2 and sum(
                    direction[i] * forward[i] for i in range(3)) < -16:
                continue
            draw_items[(cx, cy, cz)] = slot
            if not slot.opaque_vertices:
                continue
            glUniform3f(offset_loc, cx * 16 - target[0],
                        cy * 16 - target[1], cz * 16 - target[2])
            glBindBuffer(GL_ARRAY_BUFFER, slot.buffer)
            for i, n, offset in ((0, 3, 0), (1, 3, 12),
                                 (2, 2, 24), (3, 1, 32)):
                glVertexAttribPointer(i, n, GL_FLOAT, GL_FALSE, 36,
                                      ctypes.c_void_p(offset))
            glDrawArrays(GL_TRIANGLES, 0, slot.opaque_vertices)

        # Transparent faces must be drawn AFTER all depth-writing opaque
        # Sections. Sections are back-to-front; face-level sorting within
        # each Section is still outstanding for intersecting glass/water.
        translucent_keys = [key for key, slot in draw_items.items()
                            if slot.blended_vertices]
        if translucent_keys:
            glEnable(GL_BLEND)
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)
            glDepthMask(GL_FALSE)
            glUniform1f(opacity_loc, self.translucent_opacity)
            for cx, cy, cz in section_depth_order(translucent_keys, eye):
                slot = draw_items[(cx, cy, cz)]
                glUniform3f(offset_loc, cx * 16 - target[0],
                            cy * 16 - target[1], cz * 16 - target[2])
                glBindBuffer(GL_ARRAY_BUFFER, slot.buffer)
                for i, n, offset in ((0, 3, 0), (1, 3, 12),
                                     (2, 2, 24), (3, 1, 32)):
                    glVertexAttribPointer(i, n, GL_FLOAT, GL_FALSE, 36,
                                          ctypes.c_void_p(offset))
                glDrawArrays(GL_TRIANGLES, slot.opaque_vertices,
                             slot.blended_vertices)
            glDepthMask(GL_TRUE)
            glDisable(GL_BLEND)
            glUniform1f(opacity_loc, 1.0)
        if self.selection or self.clone_preview_boxes:
            vertices = self._selection_lines(target)
            # Both selection handles and clone ghost boxes use the same
            # read-only lines; no terrain mesh regeneration is required.
            # Parent wireframe uses 8-float vertices; new shader expects 9.
            vertices = [value for offset in range(0, len(vertices), 8)
                        for value in (*vertices[offset:offset + 6],
                                      0.0, 0.0, 0.0)]
            payload = array("f", vertices).tobytes()
            glUniform3f(offset_loc, 0., 0., 0.)
            glBindBuffer(GL_ARRAY_BUFFER, self._buffer)
            glBufferData(GL_ARRAY_BUFFER, len(payload), payload, GL_DYNAMIC_DRAW)
            for i, n, offset in ((0, 3, 0), (1, 3, 12),
                                 (2, 2, 24), (3, 1, 32)):
                glVertexAttribPointer(i, n, GL_FLOAT, GL_FALSE, 36,
                                      ctypes.c_void_p(offset))
            glDrawArrays(GL_LINES, 0, len(vertices) // 9)
        glBindVertexArray(0)
        glUseProgram(0)

    def _hit(self, position):
        reader = self._stream
        if reader is None:
            return None
        origin, direction = self.camera.ray(position.x(), position.y(),
                    max(1, self.width()), max(1, self.height()))
        def loaded_solid(point):
            x, y, z = point
            key = (x >> 4, y >> 4, z >> 4)
            present, snapshot = reader.get(key)
            if not present or snapshot is None:
                return False
            state = snapshot.states[
                ((y & 15) << 8) | ((z & 15) << 4) | (x & 15)]
            # Local staging takes precedence, no chunk IO on UI thread.
            if self.overlay is not None and point in self.overlay.changes:
                state = self.overlay.changes[point]
            elif point in getattr(self.session.world, "_edits", {}):
                state = self.session.world._edits[point]
            return not is_air(state)
        return trace_voxels(origin, direction, loaded_solid,
                            limit=max(128., self.distance + 64),
                            max_steps=3072)

    def preload_whole_world(self):
        """Attempt full preload only when all generated Sections fit RAM budget."""
        if self._stream is None:
            return False
        return self._stream.start_full_preload(self.memory_sections)

    def preload_visible(self):
        """Queue all visible candidate Sections; progress remains async."""
        if self._stream is not None:
            self._stream.request(self._visible)
        return len(self._visible)

    def world_index_summary(self):
        return self._index_summary

    def shutdown(self):
        self._timer.stop()
        if hasattr(self, "_mesh_pool"):
            self._mesh_pool.close()
        if self._stream is not None:
            self._stream.close()
            self._stream = None
        if self.resource_pack is not None:
            self.resource_pack.close()
            self.resource_pack = None

    def closeEvent(self, event):
        self.shutdown()
        if self.context() is not None:
            self.makeCurrent()
            self._retire.extend(slot.buffer for slot in self._gpu.values())
            self._gpu.clear()
            self._release_buffers()
            self.doneCurrent()
        super().closeEvent(event)
