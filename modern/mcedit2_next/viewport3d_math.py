"""Pure Python 3D geometry and ray picking, independent of Qt and OpenGL."""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import math

Vec3 = tuple[float, float, float]
Position = tuple[int, int, int]
FACES = (
    ((1, 0, 0), ((1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)), .77),
    ((-1, 0, 0), ((0, 0, 1), (0, 1, 1), (0, 1, 0), (0, 0, 0)), .71),
    ((0, 1, 0), ((0, 1, 1), (1, 1, 1), (1, 1, 0), (0, 1, 0)), 1.0),
    ((0, -1, 0), ((0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)), .45),
    ((0, 0, 1), ((1, 0, 1), (1, 1, 1), (0, 1, 1), (0, 0, 1)), .86),
    ((0, 0, -1), ((0, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 0)), .63),
)


def add(a: Vec3, b: Vec3) -> Vec3:
    return a[0] + b[0], a[1] + b[1], a[2] + b[2]


def scale(a: Vec3, amount: float) -> Vec3:
    return a[0] * amount, a[1] * amount, a[2] * amount


def dot(a: Vec3, b: Vec3) -> float:
    return sum(a[i] * b[i] for i in range(3))


def cross(a: Vec3, b: Vec3) -> Vec3:
    return (a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0])


def unit(a: Vec3) -> Vec3:
    length = math.sqrt(dot(a, a))
    if length < 1e-12:
        raise ValueError("Zero-length direction vector")
    return scale(a, 1.0 / length)


@dataclass(frozen=True)
class Camera:
    target: Vec3 = (0.0, 64.0, 0.0)
    yaw: float = math.radians(45)
    pitch: float = math.radians(27)
    distance: float = 32.0
    fov_degrees: float = 65.0

    def frame(self):
        yaw, pitch = self.yaw, self.pitch
        offset = (self.distance * math.cos(pitch) * math.sin(yaw),
                  self.distance * math.sin(pitch),
                  self.distance * math.cos(pitch) * math.cos(yaw))
        eye = add(self.target, offset)
        forward = unit(scale(offset, -1.0))
        right = unit(cross(forward, (0., 1., 0.)))
        up = unit(cross(right, forward))
        return eye, forward, right, up

    def ray(self, x: float, y: float, width: int, height: int) -> tuple[Vec3, Vec3]:
        if width <= 0 or height <= 0:
            raise ValueError("Viewport dimensions must be positive")
        eye, forward, right, up = self.frame()
        spread = math.tan(math.radians(self.fov_degrees) * 0.5)
        nx = (2.0 * x / width - 1.0) * (width / height) * spread
        ny = (1.0 - 2.0 * y / height) * spread
        direction = unit(add(forward, add(scale(right, nx), scale(up, ny))))
        return eye, direction

    def matrices(self, width: int, height: int):
        eye, forward, right, up = self.frame()
        # Vertex coordinates are translated to the camera target in the mesh.
        relative_eye = tuple(eye[i] - self.target[i] for i in range(3))
        view = (
            right[0], up[0], -forward[0], 0.,
            right[1], up[1], -forward[1], 0.,
            right[2], up[2], -forward[2], 0.,
            -dot(right, relative_eye), -dot(up, relative_eye),
            dot(forward, relative_eye), 1.,
        )
        near, far = .25, 4096.  # streamed distant chunks; avoid hard 512-block clip
        factor = 1.0 / math.tan(math.radians(self.fov_degrees) / 2)
        proj = (
            factor / max(width / max(height, 1), .01), 0., 0., 0.,
            0., factor, 0., 0.,
            0., 0., (far + near) / (near - far), -1.,
            0., 0., (2 * far * near) / (near - far), 0.,
        )
        return view, proj


@dataclass(frozen=True)
class RayHit:
    position: Position
    normal: Position
    distance: float


def trace_voxels(origin: Vec3, direction: Vec3, solid,
                 limit: float = 128., max_steps: int = 512) -> RayHit | None:
    """Amanatides-Woo voxel traversal, including negative world coordinates.

    A hit has the entered cube coordinate and its outward-facing face normal.
    The solid callback should return False for unloaded/missing world cells.
    """
    direction = unit(direction)
    current = [math.floor(n) for n in origin]
    step = [1 if n > 0 else (-1 if n < 0 else 0) for n in direction]
    infinity = float("inf")
    delta = [(abs(1. / n) if n else infinity) for n in direction]
    travel = [(((current[i] + (1 if step[i] > 0 else 0)) - origin[i]) /
               direction[i] if step[i] else infinity) for i in range(3)]
    distance = 0.
    normal = (0, 0, 0)
    for _ in range(max_steps):
        pos = tuple(current)
        if solid(pos):
            return RayHit(pos, normal, distance)
        axis = min(range(3), key=lambda k: travel[k])
        distance = travel[axis]
        if distance > limit or not math.isfinite(distance):
            break
        normal = tuple(-step[axis] if i == axis else 0 for i in range(3))
        current[axis] += step[axis]
        travel[axis] += delta[axis]
    return None


def is_air(state) -> bool:
    # Missing Section/neighbor is empty space, not an opaque unknown block.
    if state is None:
        return True
    # Preserve unknown modded block states without guessing transparency.
    name = getattr(state, "name", str(state)).split("[", 1)[0]
    return name in ("minecraft:air", "minecraft:cave_air", "minecraft:void_air", "air")


def block_color(state) -> tuple[float, float, float]:
    name = str(state).lower()
    if "grass" in name or "moss" in name or "leaves" in name:
        return .31, .61, .28
    if "water" in name:
        return .23, .38, .85
    if "lava" in name:
        return .92, .43, .13
    if "stone" in name or "deepslate" in name:
        return .52, .55, .59
    if "dirt" in name:
        return .50, .35, .23
    if "sand" in name:
        return .76, .70, .47
    if "snow" in name or "quartz" in name:
        return .88, .9, .93
    digest = hashlib.blake2b(name.encode("utf-8"), digest_size=3).digest()
    return tuple(.34 + digest[i] / 255 * .51 for i in range(3))


def mesh_blocks(blocks: dict[Position, object], target: Vec3) -> list[float]:
    """Return only exposed cube faces, six vertices × (x,y,z,r,g,b).

    No GPU or Minecraft I/O calls. Air and modded unknown blocks never get
    serialized or converted. All coordinates use a local floating origin.
    """
    vertices: list[float] = []
    active = {point: block for point, block in blocks.items() if not is_air(block)}
    for (x, y, z), state in active.items():
        rgb = block_color(state)
        for normal, corners, shade in FACES:
            neighbor = x + normal[0], y + normal[1], z + normal[2]
            if neighbor in active:
                continue
            color = tuple(min(1., component * shade) for component in rgb)
            for index in (0, 1, 2, 0, 2, 3):
                corner = corners[index]
                vertices.extend((
                    x + corner[0] - target[0],
                    y + corner[1] - target[1],
                    z + corner[2] - target[2],
                    *color,
                ))
    return vertices


def textured_mesh_blocks(blocks: dict[Position, object], target: Vec3,
                         face_tiles: dict[object, tuple[int, ...]],
                         grid_size: int = 32,
                         emit_bounds: tuple[int, int, int, int, int, int] | None = None) -> list[float]:
    """Mesh exposed full cube faces into 8-float position/color/UV vertices.

    Texture index 0 is a reserved white tile; a missing or unsupported texture
    uses the existing deterministic colored-cube fallback.
    """
    vertices: list[float] = []
    active = {point: block for point, block in blocks.items() if not is_air(block)}
    quv = ((0., 0.), (0., 1.), (1., 1.), (1., 0.))
    for (x, y, z), state in active.items():
        if emit_bounds is not None:
            a, b, c, d, e, f = emit_bounds
            if not (a <= x <= b and c <= y <= d and e <= z <= f):
                continue
        tint = block_color(state)
        textures = face_tiles.get(state, (0,) * 6)
        for side, (normal, corners, shade) in enumerate(FACES):
            if (x + normal[0], y + normal[1], z + normal[2]) in active:
                continue
            tile = textures[side] if side < len(textures) else 0
            if tile > 0:
                color = shade, shade, shade
            else:
                color = tuple(min(1., component * shade) for component in tint)
            col, row = tile % grid_size, tile // grid_size
            for index in (0, 1, 2, 0, 2, 3):
                corner = corners[index]
                u, v = quv[index]
                # Inset prevents neighbor tile bleeding if sampling changes.
                vertices.extend((
                    x + corner[0] - target[0],
                    y + corner[1] - target[1],
                    z + corner[2] - target[2],
                    *color,
                    (col + .02 + .96 * u) / grid_size,
                    (row + .02 + .96 * v) / grid_size,
                ))
    return vertices
