"""Amulet back end for version-neutral world access (experimental)."""
from __future__ import annotations
from pathlib import Path
import re

BLOCK_NAME = re.compile(r"^[a-z0-9_.-]+:[a-z0-9_./-]+$")

class AmuletWorld:
    def __init__(self, path: str, dimension: str = "minecraft:overworld"):
        world_path = Path(path).expanduser().resolve()
        if not world_path.is_dir() or not (world_path / "level.dat").is_file():
            raise ValueError("Select a Minecraft world folder containing level.dat")
        try:
            import amulet
            from amulet.api.block import Block
        except (ImportError, AttributeError) as exc:
            raise RuntimeError("A compatible Amulet-Core backend is not installed") from exc
        self.Block = Block
        self.level = amulet.load_level(str(world_path))
        self.path = world_path
        self.dimension = dimension
        if not all(callable(getattr(self.level, method, None))
                   for method in ("get_block", "set_block", "save", "close")):
            self.level.close()
            raise RuntimeError("Installed Amulet-Core uses an incompatible level API")

    def get_block(self, x: int, y: int, z: int):
        return self.level.get_block(x, y, z, self.dimension)

    def set_block(self, x: int, y: int, z: int, block):
        # Never turn an absent/corrupt chunk into a new chunk without permission.
        self.get_block(x, y, z)
        self.level.set_block(x, y, z, self.dimension, block)

    def make_block(self, name: str):
        name = name.strip().lower()
        if ":" not in name:
            name = "minecraft:" + name
        if "[" in name or "]" in name:
            # Amulet's Block parser understands Java block states, e.g.
            # minecraft:oak_log[axis=x]. Parse rather than discard the state.
            try:
                return self.Block.from_string_blockstate(name)
            except (AttributeError, ValueError, TypeError) as exc:
                raise ValueError("Invalid block state: " + name) from exc
        if not BLOCK_NAME.fullmatch(name):
            raise ValueError("Use a block ID such as minecraft:stone or modid:block")
        namespace, base_name = name.split(":", 1)
        return self.Block(namespace, base_name)

    def save(self):
        self.level.save()

    def close(self):
        self.level.close()
