"""Shared, grow-only texture atlas for streamed chunk meshes.

The original atlas was rebuilt for the whole visible world on every camera
movement, stalling Qt6. This atlas has stable indices: adding new blockstates
updates only the atlas image, and existing Section vertex UVs remain valid.
"""
from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtGui import QColor, QImage, QPainter

from .textureatlas import GRID, TILE
from .viewport3d_math import block_color
from .block_models import resolve_block_model
from .fluid_geometry import fluid_info

MAX_TILES = GRID * GRID
CHECKER = 1


class StreamingAtlas:
    def __init__(self, resource_pack=None):
        self.resource_pack = resource_pack
        self.image = QImage(GRID * TILE, GRID * TILE, QImage.Format.Format_RGBA8888)
        self.image.fill(Qt.GlobalColor.white)
        self._texture_ids = {}
        self._state_tiles = {}
        self._model_cache = {}
        self._model_faces = {}
        self._partial_alpha_tiles = set()
        self._fluid_tiles = {}
        self._next = 2
        self.revision = 1
        self._paint_checker()

    def _paint_checker(self):
        # Tile 0 is pure white for known vanilla color fallback. Tile 1 is a
        # real magenta/black missing-texture marker, not discarded geometry.
        painter = QPainter(self.image)
        try:
            origin_x = CHECKER * TILE
            for y in range(TILE):
                for x in range(TILE):
                    magenta = (x // 8 + y // 8) % 2 == 0
                    painter.fillRect(origin_x + x, y, 1, 1,
                        QColor(255, 0, 255) if magenta else QColor(0, 0, 0))
        finally:
            painter.end()

    def _tile_for_name(self, name):
        if name is None:
            return CHECKER
        if name in self._texture_ids:
            return self._texture_ids[name]
        if self._next >= MAX_TILES or self.resource_pack is None:
            return CHECKER
        try:
            raw = self.resource_pack.png(name)
        except (OSError, ValueError):
            raw = None
        image = QImage.fromData(raw, "PNG") if raw else QImage()
        if image.isNull():
            return CHECKER
        index = self._next
        self._next += 1
        self._texture_ids[name] = index
        # The usual Java fluid animation PNG is a vertical frame strip.
        # Keep one complete frame rather than shrinking all frames together.
        # Animation playback still needs a separate time-varying atlas path.
        if (image.height() > image.width() and image.width() > 0
                and image.height() % image.width() == 0):
            image = image.copy(0, 0, image.width(), image.width())
        scaled = image.scaled(
            TILE, TILE, Qt.AspectRatioMode.IgnoreAspectRatio,
            Qt.TransformationMode.FastTransformation)
        # Qt's default SourceOver would flatten transparent pixels onto
        # the white atlas background. Preserve the PNG alpha instead.
        painter = QPainter(self.image)
        try:
            painter.setCompositionMode(QPainter.CompositionMode.CompositionMode_Source)
            painter.drawImage((index % GRID) * TILE,
                              (index // GRID) * TILE, scaled)
        finally:
            painter.end()
        if scaled.hasAlphaChannel() and any(
                0 < scaled.pixelColor(x, y).alpha() < 255
                for y in range(TILE) for x in range(TILE)):
            self._partial_alpha_tiles.add(index)
        self.revision += 1
        return index

    def models_for(self, states):
        """Return special-geometry meshes with resolved stable atlas tile IDs.

        Ordinary full cubes stay on the existing greedy rendering path.
        Model JSON is resolved on the Qt thread, never in meshworkers.
        """
        if self.resource_pack is None:
            return {}
        selected = {}
        for state in states:
            if state not in self._model_cache:
                try:
                    model = resolve_block_model(self.resource_pack, state)
                except (OSError, ValueError, TypeError, KeyError):
                    model = None
                self._model_cache[state] = model
            model = self._model_cache[state]
            if model is None or model.full_cube:
                continue
            if state not in self._model_faces:
                self._model_faces[state] = tuple(
                    (face.side, face.corners, face.uv,
                     self._tile_for_name(face.texture), face.cullface)
                    for face in model.faces)
            selected[state] = self._model_faces[state]
        return selected

    def tiles_for(self, states):
        values = {}
        for state in states:
            if state in self._state_tiles:
                values[state] = self._state_tiles[state]
                continue
            state_name = getattr(state, "name", str(state))
            fluid = fluid_info(state)
            if fluid is not None:
                # Fluid textures are selected by the renderer, not a JSON
                # cube model; share each pair among all level variants.
                if fluid[0] not in self._fluid_tiles:
                    basename = "water" if fluid[0] == "minecraft:water" else "lava"
                    fluid_tiles = {}
                    for suffix in ("flow", "still"):
                        name = f"minecraft:block/{basename}_{suffix}"
                        available = False
                        if self.resource_pack is not None:
                            try:
                                available = self.resource_pack.png(name) is not None
                            except (OSError, ValueError, KeyError):
                                pass
                        fluid_tiles[suffix] = self._tile_for_name(name) if available else 0
                    self._fluid_tiles[fluid[0]] = (
                        fluid_tiles["flow"], fluid_tiles["flow"],
                        fluid_tiles["still"], fluid_tiles["still"],
                        fluid_tiles["flow"], fluid_tiles["flow"])
                values[state] = self._state_tiles[state] = self._fluid_tiles[fluid[0]]
                continue
            missing = CHECKER if not state_name.startswith("minecraft:") else 0
            textures = ((None,) * 6 if self.resource_pack is None else
                        self.resource_pack.face_textures(state))
            faces = []
            for name in textures:
                if name is None:
                    faces.append(missing)
                    continue
                faces.append(self._tile_for_name(name))
            self._state_tiles[state] = tuple(faces)
            values[state] = tuple(faces)
        return values

    @property
    def partial_alpha_tiles(self):
        """Immutable snapshot for safe off-thread mesh-layer classification."""
        return frozenset(self._partial_alpha_tiles)

    def gpu_image(self):
        return self.image.mirrored(False, True)
