"""Read-only resource-pack assets for the experimental OpenGL cube renderer.

Reads vanilla client JAR files, mod JARs and resource pack ZIP archives.
This resolver intentionally handles full cube models only. It never reads
world NBT and it never rewrites resource archives.
"""
from __future__ import annotations

from functools import lru_cache
import json
from pathlib import Path
import re
from zipfile import ZipFile, BadZipFile

VALID_ASSET = re.compile(r"^[a-z0-9_.-]+:(block|item)/[a-z0-9_./-]+$")
NAMESPACE = re.compile(r"^[a-z0-9_.-]+:[a-z0-9_./-]+$")
MAX_JSON_BYTES = 512 * 1024
MAX_TEXTURE_BYTES = 4 * 1024 * 1024
MAX_ZIP_ENTRIES = 150000
FACE_NAMES = ("east", "west", "up", "down", "south", "north")


class ResourcePackError(ValueError):
    pass


def asset_path(resource: str, category: str, extension: str) -> str:
    if ":" not in resource:
        resource = "minecraft:" + resource
    if not NAMESPACE.fullmatch(resource):
        raise ResourcePackError("Invalid resource location")
    namespace, name = resource.split(":", 1)
    if name.startswith("/") or any(part in ("", ".", "..") for part in name.split("/")):
        raise ResourcePackError("Unsafe resource path")
    return f"assets/{namespace}/{category}/{name}{extension}"


class ResourceLibrary:
    def __init__(self, filenames=()):
        self.archives: list[ZipFile] = []
        try:
            for filename in filenames:
                file = ZipFile(Path(filename).expanduser())
                if len(file.infolist()) > MAX_ZIP_ENTRIES:
                    file.close()
                    raise ResourcePackError("Asset archive has too many entries")
                self.archives.append(file)
        except (BadZipFile, OSError, ResourcePackError) as exc:
            self.close()
            raise ResourcePackError("Could not open Minecraft asset ZIP/JAR") from exc

    def close(self):
        for archive in self.archives:
            archive.close()
        self.archives.clear()
        self.model.cache_clear()
        self.blockstate.cache_clear()
        self.face_textures.cache_clear()

    def _read(self, key: str, size_limit: int) -> bytes | None:
        for archive in reversed(self.archives):
            try:
                item = archive.getinfo(key)
            except KeyError:
                continue
            if item.file_size > size_limit:
                raise ResourcePackError("Resource asset exceeds size limit")
            with archive.open(item) as stream:
                data = stream.read(size_limit + 1)
            if len(data) > size_limit:
                raise ResourcePackError("Resource asset exceeds size limit")
            return data
        return None

    def png(self, texture_name: str) -> bytes | None:
        return self._read(asset_path(texture_name, "textures", ".png"),
                          MAX_TEXTURE_BYTES)

    def _json(self, identifier: str, category: str):
        payload = self._read(asset_path(identifier, category, ".json"),
                             MAX_JSON_BYTES)
        if payload is None:
            return None
        try:
            result = json.loads(payload)
        except (json.JSONDecodeError, UnicodeDecodeError) as exc:
            raise ResourcePackError("Invalid Minecraft model JSON") from exc
        if not isinstance(result, dict):
            raise ResourcePackError("Expected JSON object")
        return result

    @lru_cache(maxsize=8192)
    def model(self, identifier: str):
        return self._json(identifier, "models")

    @lru_cache(maxsize=8192)
    def blockstate(self, identifier: str):
        return self._json(identifier, "blockstates")

    def _select_variant(self, state):
        definition = self.blockstate(state.name)
        if definition is None:
            return None
        variants = definition.get("variants")
        if not isinstance(variants, dict):
            # Multipart models are not a single unit cube.
            return None
        properties = dict(state.properties)
        candidates = []
        for variant, entry in variants.items():
            constraints = {}
            if variant:
                try:
                    constraints = dict(item.split("=", 1) for item in variant.split(","))
                except ValueError:
                    continue
            if not all(properties.get(k) == v for k, v in constraints.items()):
                continue
            candidates.append((len(constraints), entry))
        if not candidates:
            return None
        chosen = max(enumerate(candidates), key=lambda item: (item[1][0], -item[0]))[1][1]
        if isinstance(chosen, list):
            if not chosen:
                return None
            chosen = chosen[0]
        if not isinstance(chosen, dict) or not isinstance(chosen.get("model"), str):
            return None
        if chosen.get("x", 0) or chosen.get("y", 0):
            # Rotated block geometry/UV is not yet supported; avoid a wrong model.
            return None
        return chosen["model"]

    def _model_chain(self, identifier: str):
        seen = set()
        chain = []
        for _ in range(20):
            if identifier in seen:
                raise ResourcePackError("Cyclic block model inheritance")
            seen.add(identifier)
            model = self.model(identifier)
            if model is None:
                break
            chain.append(model)
            parent = model.get("parent")
            if not isinstance(parent, str):
                break
            identifier = parent
        else:
            raise ResourcePackError("Block model inheritance is too deep")
        return list(reversed(chain))

    @staticmethod
    def _resolve_texture(value: str, textures: dict) -> str | None:
        for _ in range(20):
            if not isinstance(value, str):
                return None
            if not value.startswith("#"):
                if ":" not in value:
                    value = "minecraft:" + value
                return value if NAMESPACE.fullmatch(value) else None
            value = textures.get(value[1:])
        return None

    @lru_cache(maxsize=8192)
    def face_textures(self, state) -> tuple[str | None, ...]:
        """Return +X, -X, +Y, -Y, +Z, -Z texture references.

        Complex shapes, multipart variants, model rotations, explicit element
        transforms and unknown resource paths use a plain color fallback.
        """
        model_name = self._select_variant(state)
        if not model_name:
            return (None,) * 6
        chain = self._model_chain(model_name)
        if not chain:
            return (None,) * 6
        textures = {}
        for item in chain:
            entries = item.get("textures", {})
            if isinstance(entries, dict):
                textures.update(entries)
        elements = None
        for item in chain:
            if "elements" in item:
                elements = item["elements"]
        faces = None
        if elements is not None:
            if (not isinstance(elements, list) or len(elements) != 1 or
                not isinstance(elements[0], dict) or
                elements[0].get("from") != [0, 0, 0] or
                elements[0].get("to") != [16, 16, 16] or
                "rotation" in elements[0]):
                return (None,) * 6
            faces = elements[0].get("faces")
            if not isinstance(faces, dict):
                return (None,) * 6
        mapped = []
        for face in FACE_NAMES:
            explicit = (faces or {}).get(face)
            token = explicit.get("texture") if isinstance(explicit, dict) else None
            if token is None:
                names = ((face, "top", "end", "all") if face == "up" else
                         (face, "bottom", "end", "all") if face == "down" else
                         (face, "side", "all"))
                token = next((textures[name] for name in names if name in textures),
                             None)
            mapped.append(self._resolve_texture(token, textures))
        return tuple(mapped)
