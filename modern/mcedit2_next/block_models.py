"""Bounded Minecraft Java resource model geometry, separate from OpenGL.

Represents resource-pack blockstates variants/multipart and inherited model
elements. Face geometry is in native block-local units [0,1]. Unknown,
malformed and unsupported element-rotation cases fail closed to the legacy
diagnostic cube rather than silently fabricate an intricate model.
"""
from __future__ import annotations

from dataclasses import dataclass
import math

from .viewport3d_math import FACES

FACE_NAMES = ("east", "west", "up", "down", "south", "north")
FACE_INDEX = {face: i for i, face in enumerate(FACE_NAMES)}
MAX_MODEL_PARTS = 32
MAX_MODEL_ELEMENTS = 128
MAX_MODEL_FACES = 512


@dataclass(frozen=True)
class FaceModel:
    side: int
    corners: tuple[tuple[float, float, float], ...]
    uv: tuple[tuple[float, float], ...]
    texture: str
    cullface: bool
    tintindex: int | None = None


@dataclass(frozen=True)
class BlockModel:
    faces: tuple[FaceModel, ...]
    full_cube: bool = False


def _matching(condition, properties) -> bool:
    """Minecraft multipart WHEN conditions: AND and OR, with | alternatives."""
    if condition is None:
        return True
    if not isinstance(condition, dict):
        return False
    if "OR" in condition:
        parts = condition["OR"]
        if not isinstance(parts, list) or not parts:
            return False
        if not any(_matching(item, properties) for item in parts):
            return False
    if "AND" in condition:
        parts = condition["AND"]
        if not isinstance(parts, list) or not parts:
            return False
        if not all(_matching(item, properties) for item in parts):
            return False
    for key, value in condition.items():
        if key in ("OR", "AND"):
            continue
        if (not isinstance(value, str)
                or properties.get(key) not in value.split("|")):
            return False
    return True


def _variants(definition, properties):
    if not isinstance(definition, dict):
        return None
    parts = []
    if "variants" in definition:
        variants = definition["variants"]
        if not isinstance(variants, dict):
            return None
        matches = []
        for key, entry in variants.items():
            if not isinstance(key, str):
                continue
            pairs = key.split(",") if key else ()
            options = {}
            try:
                for pair in pairs:
                    prop, value = pair.split("=", 1)
                    if not prop or prop in options or not value:
                        raise ValueError("Bad state selector")
                    options[prop] = value
            except ValueError:
                continue
            if all(properties.get(k) == v for k, v in options.items()):
                matches.append((len(options), entry))
        if not matches:
            return None
        choice = max(enumerate(matches),
                     key=lambda entry: (entry[1][0], -entry[0]))[1][1]
        parts.append(choice)
    if "multipart" in definition:
        multipart = definition["multipart"]
        if not isinstance(multipart, list) or len(multipart) > MAX_MODEL_PARTS:
            return None
        for item in multipart:
            if not isinstance(item, dict) or "apply" not in item:
                return None
            if _matching(item.get("when"), properties):
                parts.append(item["apply"])
    if not parts or len(parts) > MAX_MODEL_PARTS:
        return None
    chosen = []
    for part in parts:
        # Weighted arrays need random seed/world-position logic; select
        # first deterministic entry until native weighted model parity exists.
        if isinstance(part, list):
            if not part:
                return None
            part = part[0]
        if (not isinstance(part, dict) or
                not isinstance(part.get("model"), str)):
            return None
        x, y = part.get("x", 0), part.get("y", 0)
        if (type(x) is not int or type(y) is not int
                or x % 90 or y % 90):
            return None
        chosen.append((part["model"], x % 360, y % 360))
    return tuple(chosen)


def _model_definition(library, model_name):
    chain = library._model_chain(model_name)
    if not chain:
        return None
    textures = {}
    elements = None
    for model in chain:
        if not isinstance(model, dict):
            return None
        entries = model.get("textures", {})
        if not isinstance(entries, dict):
            return None
        textures.update(entries)
        if "elements" in model:
            elements = model["elements"]
    if not isinstance(elements, list) or len(elements) > MAX_MODEL_ELEMENTS:
        return None
    return textures, elements


def _rotate_point(point, xrot, yrot):
    """Java model blockstate rotation about (8,8,8), quarter turns."""
    x, y, z = [v - 0.5 for v in point]
    for _ in range((xrot // 90) % 4):
        y, z = -z, y
    for _ in range((yrot // 90) % 4):
        x, z = z, -x
    return (x + 0.5, y + 0.5, z + 0.5)


def _element_rotation_point(point, rotation):
    """Minecraft JSON element rotations around an explicit origin."""
    if rotation is None:
        return point
    if not isinstance(rotation, dict):
        raise ValueError("Invalid Minecraft element rotation")
    axis = rotation.get("axis")
    angle = rotation.get("angle")
    origin = rotation.get("origin", [8, 8, 8])
    rescale = rotation.get("rescale", False)
    if (axis not in ("x", "y", "z")
            or type(angle) not in (int, float)
            or not math.isfinite(angle)
            or angle not in (-45, -22.5, 0, 22.5, 45)
            or type(rescale) is not bool
            or not isinstance(origin, list)
            or len(origin) != 3
            or any(type(v) not in (int, float) or not math.isfinite(v)
                   or not -16 <= v <= 32 for v in origin)):
        raise ValueError("Unsupported Minecraft element rotation")
    center = tuple(v / 16.0 for v in origin)
    x, y, z = (point[i] - center[i] for i in range(3))
    radians = math.radians(angle)
    cosine, sine = math.cos(radians), math.sin(radians)
    if axis == "x":
        y, z = y * cosine - z * sine, y * sine + z * cosine
        if rescale:
            y, z = y / cosine, z / cosine
    elif axis == "y":
        x, z = x * cosine + z * sine, -x * sine + z * cosine
        if rescale:
            x, z = x / cosine, z / cosine
    else:
        x, y = x * cosine - y * sine, x * sine + y * cosine
        if rescale:
            x, y = x / cosine, y / cosine
    return x + center[0], y + center[1], z + center[2]


def _rotated_side(side, xrot, yrot):
    """Quarter-rotate a normal as a vector, NOT a coordinate around 0.5."""
    x, y, z = FACES[side][0]
    for _ in range((xrot // 90) % 4):
        y, z = -z, y
    for _ in range((yrot // 90) % 4):
        x, z = z, -x
    return next((i for i, (normal, _, _) in enumerate(FACES)
                 if normal == (x, y, z)), None)


def _face_corners(low, high, side):
    return tuple(tuple(low[i] if corner[i] == 0 else high[i]
                       for i in range(3)) for corner in FACES[side][1])


def _default_uv(low, high, side):
    x0, y0, z0 = low
    x1, y1, z1 = high
    # Resource pack V runs downwards; UV must agree with the retained
    # face corner order from viewport3d_math.FACES.
    if side in (0, 1):
        return (z0, 1 - y1, z1, 1 - y0)
    if side in (2, 3):
        return (x0, z0, x1, z1)
    return (x0, 1 - y1, x1, 1 - y0)


def _face_uv(uv, rotation):
    if (not isinstance(uv, (tuple, list)) or len(uv) != 4
            or any(type(v) not in (int, float) or not math.isfinite(v)
                   for v in uv) or rotation not in (0, 90, 180, 270)):
        return None
    u0, v0, u1, v1 = (float(v) for v in uv)
    coordinates = [(u0, v0), (u0, v1), (u1, v1), (u1, v0)]
    turns = rotation // 90
    return tuple(tuple(q / 16.0 for q in coordinates[(i + turns) % 4])
                 for i in range(4))


def resolve_block_model(library, state) -> BlockModel | None:
    definition = library.blockstate(state.name)
    choices = _variants(definition, dict(state.properties))
    if choices is None:
        return None
    output = []
    full_cube = (len(choices) == 1 and choices[0][1:] == (0, 0))
    for model_name, xrot, yrot in choices:
        model = _model_definition(library, model_name)
        if model is None:
            return None
        textures, elements = model
        if len(elements) != 1:
            full_cube = False
        for element in elements:
            if not isinstance(element, dict):
                return None
            element_rotation = element.get("rotation")
            if element_rotation is not None:
                full_cube = False
            low, high = element.get("from"), element.get("to")
            if (not isinstance(low, list) or not isinstance(high, list)
                    or len(low) != 3 or len(high) != 3
                    or any(type(v) not in (int, float)
                           or not math.isfinite(v) or v < -16 or v > 32
                           for v in (*low, *high))
                    or any(lo > hi for lo, hi in zip(low, high))
                    or sum(lo < hi for lo, hi in zip(low, high)) < 2):
                return None
            low = tuple(float(v) / 16 for v in low)
            high = tuple(float(v) / 16 for v in high)
            if low != (0., 0., 0.) or high != (1., 1., 1.):
                full_cube = False
            faces = element.get("faces")
            if not isinstance(faces, dict):
                return None
            if set(faces) != set(FACE_NAMES):
                full_cube = False
            for name, face in faces.items():
                if name not in FACE_INDEX or not isinstance(face, dict):
                    return None
                token = face.get("texture")
                texture = library._resolve_texture(token, textures)
                if texture is None:
                    return None
                side = FACE_INDEX[name]
                uv = face.get("uv")
                if uv is None:
                    uv = tuple(v * 16 for v in _default_uv(low, high, side))
                else:
                    # Greedy mesher assumes basic non-rotated repeating UV.
                    # Respect explicitly authored per-face UV rectangles.
                    full_cube = False
                if face.get("rotation", 0):
                    full_cube = False
                uvs = _face_uv(uv, face.get("rotation", 0))
                if uvs is None:
                    return None
                try:
                    corners = tuple(_rotate_point(
                        _element_rotation_point(pt, element_rotation),
                        xrot, yrot)
                        for pt in _face_corners(low, high, side))
                except ValueError:
                    return None
                rotated = _rotated_side(side, xrot, yrot)
                if rotated is None:
                    return None
                cullface = face.get("cullface")
                if cullface is not None and cullface not in FACE_INDEX:
                    return None
                # cullface applies only to a face on the external boundary;
                # rotated faces use their resulting normal axis.
                normal = FACES[rotated][0]
                axis = next(i for i, value in enumerate(normal) if value)
                target = 1.0 if normal[axis] > 0 else 0.0
                boundary = all(abs(pt[axis] - target) < 1e-7
                               for pt in corners)
                output.append(FaceModel(rotated, corners, uvs, texture,
                                        bool(cullface) and boundary
                                        and element_rotation is None,
                                        face.get("tintindex") if
                                        type(face.get("tintindex")) is int
                                        else None))
                if len(output) > MAX_MODEL_FACES:
                    return None
    if not output:
        return None
    return BlockModel(tuple(output), full_cube)
