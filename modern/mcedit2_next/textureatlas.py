"""Qt6 atlas creation for basic full-cube Minecraft resource-pack textures."""
from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtGui import QImage, QPainter

GRID = 32
TILE = 16
LIMIT = GRID * GRID


def make_atlas(blocks: dict, library=None):
    """Return (RGBA QImage, per-blockstate face tile indices).

    Tile 0 stays solid white so unsupported shapes/unknown textures fall back
    to solid colors. Asset decoding uses Qt, not untrusted plugin Python code.
    """
    image = QImage(GRID * TILE, GRID * TILE, QImage.Format.Format_RGBA8888)
    image.fill(Qt.GlobalColor.white)
    tiles: dict[str, int] = {}
    state_tiles = {}
    if library is None or not blocks:
        return image, state_tiles

    painter = QPainter(image)
    try:
        for state in set(blocks.values()):
            names = library.face_textures(state)
            faces = []
            for identifier in names:
                if identifier is None:
                    faces.append(0)
                    continue
                if identifier in tiles:
                    faces.append(tiles[identifier])
                    continue
                if len(tiles) >= LIMIT - 1:
                    faces.append(0)
                    continue
                try:
                    raw = library.png(identifier)
                except (OSError, ValueError):
                    raw = None
                if not raw:
                    faces.append(0)
                    continue
                texture = QImage.fromData(raw, "PNG")
                if texture.isNull():
                    faces.append(0)
                    continue
                tile_id = len(tiles) + 1
                tiles[identifier] = tile_id
                texture = texture.scaled(TILE, TILE, Qt.AspectRatioMode.IgnoreAspectRatio,
                                         Qt.TransformationMode.FastTransformation)
                col, row = tile_id % GRID, tile_id // GRID
                painter.drawImage(col * TILE, row * TILE, texture)
                faces.append(tile_id)
            state_tiles[state] = tuple(faces)
    finally:
        painter.end()
    # glTexImage2D expects scanlines bottom-first; maintain matching UVs.
    return image.mirrored(False, True), state_tiles
