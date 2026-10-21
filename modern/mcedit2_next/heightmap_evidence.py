"""Explicit version-bound, per-state heightmap rule snapshots.

A JSON evidence snapshot is DATA, not executable code. Its author must
collect exact effective heightmap/fluid predicate facts from the running game for the exact
DataVersion and runtime/modpack. A file claiming to be a vanilla snapshot
does not authenticate itself. Unknown states are rejected, never guessed.
"""
from __future__ import annotations

from dataclasses import dataclass
from hashlib import sha256
from pathlib import Path
from types import MappingProxyType
from typing import Mapping
import json

from .heightmap_codec import HeightmapError
from .palette import BlockState, make_block

MAX_EVIDENCE_BYTES = 32 * 1024 * 1024
MAX_EVIDENCE_STATES = 200000
_REQUIRED = frozenset(("blocks_motion_in_heightmap", "has_fluid"))


@dataclass(frozen=True)
class EvidenceFacts:
    # Historical JSON field name; semantics = runtime OCEAN_FLOOR predicate,
    # not membership in an assumed Minecraft block tag.
    blocks_motion_in_heightmap: bool
    has_fluid: bool


@dataclass(frozen=True)
class HeightmapEvidence:
    data_version: int
    source: str
    digest: str
    states: Mapping[str, EvidenceFacts]

    @property
    def total_states(self) -> int:
        return len(self.states)

    def classify(self, block: BlockState, name: str, data_version: int) -> bool:
        if name not in ("MOTION_BLOCKING", "OCEAN_FLOOR"):
            raise HeightmapError("Unsupported heightmap predicate")
        if type(data_version) is not int or data_version != self.data_version:
            raise HeightmapError(
                "Imported heightmap rule DataVersion does not match the chunk")
        if not isinstance(block, BlockState):
            raise HeightmapError("Expected native BlockState")
        key = str(block)
        facts = self.states.get(key)
        if facts is None:
            raise HeightmapError(
                f"No explicit registered heightmap facts for {key}; "
                "unsafe state is not inferred")
        return (facts.blocks_motion_in_heightmap or facts.has_fluid
                if name == "MOTION_BLOCKING"
                else facts.blocks_motion_in_heightmap)


def _reject_duplicates(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise HeightmapError(f"Duplicate JSON object field {str(k)[:90]}")
        out[k] = v
    return out


def parse_heightmap_evidence(raw: bytes) -> HeightmapEvidence:
    """Validate a strict, per-state immutable JSON snapshot in memory."""
    if not isinstance(raw, bytes) or not 1 <= len(raw) <= MAX_EVIDENCE_BYTES:
        raise HeightmapError("Evidence JSON must be 1 byte to 32 MiB")
    try:
        obj = json.loads(raw.decode("utf-8"),
                         object_pairs_hook=_reject_duplicates,
                         parse_constant=lambda x: (_ for _ in ()).throw(
                             HeightmapError("Nonfinite JSON number")))
    except (ValueError, UnicodeDecodeError) as exc:
        raise HeightmapError(f"Invalid UTF-8 JSON evidence: {exc}") from exc
    if type(obj) is not dict or set(obj) != {
            "schema", "data_version", "source", "states"}:
        raise HeightmapError("Evidence schema requires exactly schema/data_version/source/states")
    if obj["schema"] != 1 or type(obj["schema"]) is not int:
        raise HeightmapError("Only heightmap evidence schema 1 is supported")
    dv = obj["data_version"]
    if type(dv) is not int or not 2865 <= dv <= 100000:
        raise HeightmapError("Evidence DataVersion must be modern Java, 2865+")
    source = obj["source"]
    if (type(source) is not str or
            not 1 <= len(source.strip()) <= 256 or
            "\n" in source or "\r" in source):
        raise HeightmapError("Evidence source must identify the original registry")
    data = obj["states"]
    if type(data) is not dict or not 1 <= len(data) <= MAX_EVIDENCE_STATES:
        raise HeightmapError("Evidence requires 1..200000 exact native block states")
    checked = {}
    for state, entry in data.items():
        if not isinstance(state, str) or len(state) > 512:
            raise HeightmapError("Evidence contains invalid state name")
        try:
            parsed = make_block(state)
        except (TypeError, ValueError) as exc:
            raise HeightmapError(f"Malformed evidence state {state!r}") from exc
        if str(parsed) != state:
            # All namespaces are permitted ONLY with exact per-state facts.
            # This is still read-only: third-party registry claims cannot
            # authorize saving or generation of world data.
            raise HeightmapError(
                f"Noncanonical state {state!r} is not accepted")
        if type(entry) is not dict or set(entry) != _REQUIRED:
            raise HeightmapError(
                f"State {state} must specify heightmap and fluid predicate facts")
        if any(type(entry[k]) is not bool for k in _REQUIRED):
            raise HeightmapError(f"State {state} must use boolean heightmap/fluids")
        checked[state] = EvidenceFacts(
            entry["blocks_motion_in_heightmap"], entry["has_fluid"])
    return HeightmapEvidence(
        dv, source.strip(), sha256(raw).hexdigest(),
        MappingProxyType(checked))


def read_heightmap_evidence(filename: str | Path) -> HeightmapEvidence:
    """Read an explicitly selected local JSON file, not the world/datapack.

    Reject symlinks and oversized inputs. Never import Python plugins,
    resolve tag references automatically or execute supplied expressions.
    """
    path = Path(filename)
    if path.is_symlink() or not path.is_file():
        raise HeightmapError("Select a regular non-symlink evidence JSON file")
    if path.stat().st_size > MAX_EVIDENCE_BYTES:
        raise HeightmapError("Evidence JSON exceeds the 32 MiB limit")
    try:
        return parse_heightmap_evidence(path.read_bytes())
    except OSError as exc:
        raise HeightmapError("Cannot read evidence file") from exc
