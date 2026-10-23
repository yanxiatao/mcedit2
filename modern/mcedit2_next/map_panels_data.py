"""Original Java filled-map metadata and pixel inspection for Qt6.

Color IDs 0..207 reuse the original MCEdit2 map color
lookup table; later and unsupported entries use an explicit diagnostic
placeholder, never an invented color. Saving is isolated in native_map_edit.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
import struct

from . import nbtwire as nbt
from .world_panels_data import read_original_gzip

MAP_NAME = re.compile(r"map_(0|[1-9][0-9]*)\.dat\Z")
MAX_MAPS = 1024
MAX_PIXELS = 1024 * 1024

# Copied from original src/mceditlib/anvil/adapter.py AnvilMapData._colorTable.
# Index slots with zero alpha are unsupported old values, NOT modern colors.
ORIGINAL_MCEDIT2_MAP_COLORS = (
    (0, 0, 0, 0),
    (0, 0, 0, 0),
    (0, 0, 0, 0),
    (0, 0, 0, 0),
    (88, 124, 39, 255),
    (108, 151, 47, 255),
    (125, 176, 55, 255),
    (66, 93, 29, 255),
    (172, 162, 114, 255),
    (210, 199, 138, 255),
    (244, 230, 161, 255),
    (128, 122, 85, 255),
    (138, 138, 138, 255),
    (169, 169, 169, 255),
    (197, 197, 197, 255),
    (104, 104, 104, 255),
    (178, 0, 0, 255),
    (217, 0, 0, 255),
    (252, 0, 0, 255),
    (133, 0, 0, 255),
    (111, 111, 178, 255),
    (136, 136, 217, 255),
    (158, 158, 252, 255),
    (83, 83, 133, 255),
    (116, 116, 116, 255),
    (142, 142, 142, 255),
    (165, 165, 165, 255),
    (87, 87, 87, 255),
    (0, 86, 0, 255),
    (0, 105, 0, 255),
    (0, 123, 0, 255),
    (0, 64, 0, 255),
    (178, 178, 178, 255),
    (217, 217, 217, 255),
    (252, 252, 252, 255),
    (133, 133, 133, 255),
    (114, 117, 127, 255),
    (139, 142, 156, 255),
    (162, 166, 182, 255),
    (85, 87, 96, 255),
    (105, 75, 53, 255),
    (128, 93, 65, 255),
    (149, 108, 76, 255),
    (78, 56, 39, 255),
    (78, 78, 78, 255),
    (95, 95, 95, 255),
    (111, 111, 111, 255),
    (58, 58, 58, 255),
    (44, 44, 178, 255),
    (54, 54, 217, 255),
    (63, 63, 252, 255),
    (33, 33, 133, 255),
    (99, 83, 49, 255),
    (122, 101, 61, 255),
    (141, 118, 71, 255),
    (74, 62, 38, 255),
    (178, 175, 170, 255),
    (217, 214, 208, 255),
    (252, 249, 242, 255),
    (133, 131, 127, 255),
    (150, 88, 36, 255),
    (184, 108, 43, 255),
    (213, 125, 50, 255),
    (113, 66, 27, 255),
    (124, 52, 150, 255),
    (151, 64, 184, 255),
    (176, 75, 213, 255),
    (93, 39, 113, 255),
    (71, 107, 150, 255),
    (87, 130, 184, 255),
    (101, 151, 213, 255),
    (53, 80, 113, 255),
    (159, 159, 36, 255),
    (195, 195, 43, 255),
    (226, 226, 50, 255),
    (120, 120, 27, 255),
    (88, 142, 17, 255),
    (108, 174, 21, 255),
    (125, 202, 25, 255),
    (66, 107, 13, 255),
    (168, 88, 115, 255),
    (206, 108, 140, 255),
    (239, 125, 163, 255),
    (126, 66, 86, 255),
    (52, 52, 52, 255),
    (64, 64, 64, 255),
    (75, 75, 75, 255),
    (39, 39, 39, 255),
    (107, 107, 107, 255),
    (130, 130, 130, 255),
    (151, 151, 151, 255),
    (80, 80, 80, 255),
    (52, 88, 107, 255),
    (64, 108, 130, 255),
    (75, 125, 151, 255),
    (39, 66, 80, 255),
    (88, 43, 124, 255),
    (108, 53, 151, 255),
    (125, 62, 176, 255),
    (66, 33, 93, 255),
    (36, 52, 124, 255),
    (43, 64, 151, 255),
    (50, 75, 176, 255),
    (27, 39, 93, 255),
    (71, 52, 36, 255),
    (87, 64, 43, 255),
    (101, 75, 50, 255),
    (53, 39, 27, 255),
    (71, 88, 36, 255),
    (87, 108, 43, 255),
    (101, 125, 50, 255),
    (53, 66, 27, 255),
    (107, 36, 36, 255),
    (130, 43, 43, 255),
    (151, 50, 50, 255),
    (80, 27, 27, 255),
    (17, 17, 17, 255),
    (21, 21, 21, 255),
    (25, 25, 25, 255),
    (13, 13, 13, 255),
    (174, 166, 53, 255),
    (212, 203, 65, 255),
    (247, 235, 76, 255),
    (130, 125, 39, 255),
    (63, 152, 148, 255),
    (78, 186, 181, 255),
    (91, 216, 210, 255),
    (47, 114, 111, 255),
    (51, 89, 178, 255),
    (62, 109, 217, 255),
    (73, 129, 252, 255),
    (39, 66, 133, 255),
    (0, 151, 39, 255),
    (0, 185, 49, 255),
    (0, 214, 57, 255),
    (0, 113, 30, 255),
    (90, 59, 34, 255),
    (110, 73, 41, 255),
    (127, 85, 48, 255),
    (67, 44, 25, 255),
    (78, 1, 0, 255),
    (95, 1, 0, 255),
    (111, 2, 0, 255),
    (58, 1, 0, 255),
    (147, 124, 113, 0),
    (180, 152, 138, 0),
    (209, 177, 161, 0),
    (110, 93, 85, 0),
    (112, 57, 25, 0),
    (137, 70, 31, 0),
    (159, 82, 36, 0),
    (84, 43, 19, 0),
    (105, 61, 76, 0),
    (128, 75, 93, 0),
    (149, 87, 108, 0),
    (78, 46, 57, 0),
    (79, 76, 97, 0),
    (96, 93, 119, 0),
    (112, 108, 138, 0),
    (59, 57, 73, 0),
    (131, 93, 25, 0),
    (160, 114, 31, 0),
    (186, 133, 36, 0),
    (98, 70, 19, 0),
    (72, 82, 37, 0),
    (88, 100, 45, 0),
    (103, 117, 53, 0),
    (54, 61, 28, 0),
    (112, 54, 55, 0),
    (138, 66, 67, 0),
    (160, 77, 78, 0),
    (84, 40, 41, 0),
    (40, 28, 24, 0),
    (49, 35, 30, 0),
    (57, 41, 35, 0),
    (30, 21, 18, 0),
    (95, 75, 69, 0),
    (116, 92, 84, 0),
    (135, 107, 98, 0),
    (71, 56, 51, 0),
    (61, 64, 64, 0),
    (75, 79, 79, 0),
    (87, 92, 92, 0),
    (46, 48, 48, 0),
    (86, 51, 62, 0),
    (105, 62, 75, 0),
    (122, 73, 88, 0),
    (64, 38, 46, 0),
    (53, 43, 64, 0),
    (65, 53, 79, 0),
    (76, 62, 92, 0),
    (40, 32, 48, 0),
    (53, 35, 24, 0),
    (65, 43, 30, 0),
    (76, 50, 35, 0),
    (40, 26, 18, 0),
    (53, 57, 29, 0),
    (65, 70, 36, 0),
    (76, 82, 42, 0),
    (40, 43, 22, 0),
    (100, 42, 32, 0),
    (122, 51, 39, 0),
    (142, 60, 46, 0),
    (75, 31, 24, 0),
    (26, 15, 11, 0),
    (31, 18, 13, 0),
    (37, 22, 16, 0),
    (19, 11, 8, 0),
)


@dataclass(frozen=True)
class MapInfo:
    map_id: int
    width: int
    height: int
    report: str
    colors: bytes
    nbt_payload: bytes


def list_original_maps(world):
    folder = world.path / "data"
    if not folder.is_dir() or folder.is_symlink():
        return ()
    entries = []
    for path in sorted(folder.iterdir(), key=lambda p: p.name):
        match = MAP_NAME.fullmatch(path.name)
        if match is None or path.is_symlink() or not path.is_file():
            continue
        entries.append(int(match.group(1)))
        if len(entries) > MAX_MAPS:
            raise ValueError("Map list exceeds the 1024-map UI cap")
    return tuple(sorted(entries))


def inspect_original_map(world, map_id: int):
    if type(map_id) is not int or not 0 <= map_id <= 2_147_483_647:
        raise ValueError("Invalid Java map ID")
    filename = world.path / "data" / f"map_{map_id}.dat"
    raw = read_original_gzip(filename)
    root = nbt.parse(raw)
    data = root.require("data", 10)
    width = data.child("width")
    height = data.child("height")
    w = int(width.value) if width is not None and width.kind in (2, 3) else 128
    h = int(height.value) if height is not None and height.kind in (2, 3) else 128
    if not 1 <= w <= 1024 or not 1 <= h <= 1024 or w * h > MAX_PIXELS:
        raise ValueError("Map resolution exceeds bounded preview")
    colors_tag = data.require("colors", 7)
    size = struct.unpack_from(">i", raw, colors_tag.payload_start)[0]
    if size != w * h:
        raise ValueError("Map byte colors array does not match width × height")
    pixels = raw[colors_tag.payload_start + 4:colors_tag.end]
    if len(pixels) != size:
        raise ValueError("Map colors data is truncated")
    unknown = sum(1 for index in pixels if index >= len(ORIGINAL_MCEDIT2_MAP_COLORS)
                  or (index >= 4 and ORIGINAL_MCEDIT2_MAP_COLORS[index][3] == 0))
    lines = [
        f"Map #{map_id} — original Java filled map (inspect; guarded pixel/scalar edits)",
        f"Size: {w} × {h}",
        f"Dimension: {data.child('dimension').value if data.child('dimension') is not None else '(missing)'}",
        f"Scale: {data.child('scale').value if data.child('scale') is not None else '(missing)'}",
        f"Center X: {data.child('xCenter').value if data.child('xCenter') is not None else '(missing)'}",
        f"Center Z: {data.child('zCenter').value if data.child('zCenter') is not None else '(missing)'}",
        f"Unsupported modern/unknown original palette pixels: {unknown}",
        "Known legacy colors use MCEdit2's original palette.",
        "Unsupported colors preview as diagnostic magenta. Staged edits change only chosen native byte indexes.",
    ]
    return MapInfo(map_id, w, h, "\n".join(lines), pixels,
                   raw[data.payload_start:data.end])


def color_for_map_byte(index: int):
    if not 0 <= index <= 255:
        raise ValueError("Native Minecraft map byte must be 0..255")
    if index < 4:
        return (0, 0, 0, 0)
    if index < len(ORIGINAL_MCEDIT2_MAP_COLORS):
        result = ORIGINAL_MCEDIT2_MAP_COLORS[index]
        if result[3]:
            return result
    return (255, 0, 255, 255)
