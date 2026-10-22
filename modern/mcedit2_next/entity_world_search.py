"""Read-only indexed Java entity search across old chunk and separate entity regions.

Scans ONLY header-indexed, already generated entity/terrain chunks. Every
original Entity NBT compound remains opaque; only primitive ID and Pos are
read. No entity cloning, NBT normalization, DataFixer, block writes or save.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from threading import Event
from typing import Callable
import math
import struct

from . import nbtwire as nbt
from .entity_index import EntityRecord, read_entities
from .palette import IDENT
from .worldstream import RegionIndex

MAX_ENTITY_CHUNKS = 200_000
MAX_ENTITY_REGIONS = 8_192
MAX_ENTITY_RESULTS = 256
MAX_ENTITY_PER_CHUNK = 4_096
WORLD_LIMIT = 30_000_000


@dataclass(frozen=True)
class WorldEntityMatch:
    identifier: str
    position: tuple[float, float, float] | None
    source: str
    chunk: tuple[int, int]
    original_index: int
    status: str
    token: str = ""


@dataclass(frozen=True)
class WorldEntityFailure:
    source: str
    chunk: tuple[int, int]
    reason: str


@dataclass(frozen=True)
class WorldEntitySearchResult:
    query: str
    include_staged: bool
    indexed_chunks: int
    examined_chunks: int
    decoded_chunks: int
    matching_entities: int
    matches: tuple[WorldEntityMatch, ...]
    failures: tuple[WorldEntityFailure, ...]
    invalid_region_entries: int
    invalid_region_examples: tuple[str, ...]
    orphaned_staging: tuple[str, ...]
    cancelled: bool

    @property
    def truncated(self):
        return len(self.matches) < self.matching_entities

    @property
    def complete(self):
        return (not self.cancelled
                and self.examined_chunks == self.indexed_chunks
                and self.decoded_chunks == self.indexed_chunks
                and not self.failures and not self.orphaned_staging
                and not self.invalid_region_entries)

    def report(self):
        lines = [
            "INDEXED JAVA WORLD ENTITY FIND — READ ONLY",
            f"Exact entity ID: {self.query} (* = all)",
            f"Original and staged entity snapshot: {self.include_staged}",
            f"Storage chunks indexed: {self.indexed_chunks}",
            f"Storage chunks examined: {self.examined_chunks}",
            f"Storage chunks decoded: {self.decoded_chunks}",
            f"Matching entity records: {self.matching_entities}",
            f"Results listed: {len(self.matches)} / {MAX_ENTITY_RESULTS} cap",
            f"Cancelled: {self.cancelled}; complete: {self.complete}",
            "Sources marked 'chunk' and 'entities' denote distinct original files.",
            "Missing entity Pos is never guessed; original NBT is never changed.",
        ]
        if self.truncated:
            lines.append("MATCH LIST CAPPED; decoded-match count remains exact.")
        if not self.complete:
            lines.append(
                "INCOMPLETE: unreadable source, invalid region header, "
                "orphaned edit or cancellation. Do not infer absence.")
        if self.invalid_region_entries:
            lines.append(f"Invalid region index entries: {self.invalid_region_entries}")
            lines.extend("  [INDEX] " + line for line in self.invalid_region_examples)
        if self.orphaned_staging:
            lines.append(f"Staged entity references without indexed source: {len(self.orphaned_staging)}")
            lines.extend("  [STAGED] " + line for line in self.orphaned_staging[:64])
        for failure in self.failures[:128]:
            lines.append(
                f"  [UNREADABLE {failure.source} {failure.chunk}] {failure.reason}")
        for match in self.matches:
            lines.append(
                f"  {match.position} [{match.status}] {match.identifier} "
                f"({match.source} {match.chunk}, original-index={match.original_index})")
        return "\n".join(lines)


def _validate_entity_payload(payload, source):
    if not isinstance(payload, bytes):
        raise ValueError("Native staged entity payload must be opaque bytes")
    member = nbt.parse(nbt.make_tag(10, "", payload))
    entity_id = member.child("id")
    if entity_id is None or entity_id.kind != 8 or not entity_id.value:
        raise ValueError("Staged entity missing original String id")
    record = read_entities(nbt.make_tag(10, "", nbt.compound_payload([
        nbt.list_tag("Entities", 10, [payload])])), source)[0]
    if record.identifier != entity_id.value:
        raise ValueError("Entity ID mismatch in original staged NBT")
    return record


def _validate_storage_chunk(raw, source, cx, cz):
    """Refuse chunks whose embedded source coordinate identity conflicts.

    Terrain chunks require xPos/zPos (root in modern Java, Level in old
    versions). Separate entity regions may contain an integer-array Position
    of length 2; when present it must match the indexed chunk address.
    A missing Position in an old/modded split file is not invented.
    """
    root = nbt.parse(raw)
    if source == "chunk":
        level = root.child("Level")
        if level is not None and level.kind != 10:
            raise ValueError("Invalid original Level compound")
        where = level if level is not None else root
        x = where.child("xPos")
        z = where.child("zPos")
        if (x is None or z is None or x.kind != 3 or z.kind != 3
                or (x.value, z.value) != (cx, cz)):
            raise ValueError("Original terrain chunk xPos/zPos mismatch")
    elif source == "entities":
        position = root.child("Position")
        if position is not None:
            if position.kind != 11 or position.end - position.payload_start != 12:
                raise ValueError("Original entity region Position must be 2 Ints")
            n, px, pz = struct.unpack_from(">iii", raw, position.payload_start)
            if n != 2 or (px, pz) != (cx, cz):
                raise ValueError("Original entity region Position mismatch")
    else:
        raise ValueError("Unknown original entity storage source")
    return root


def _validated_position(record):
    pos = record.position
    if pos is None:
        return None
    if (len(pos) != 3 or any(not math.isfinite(value) for value in pos)
            or not -WORLD_LIMIT <= pos[0] <= WORLD_LIMIT
            or not -WORLD_LIMIT <= pos[2] <= WORLD_LIMIT
            or not -2048 <= pos[1] < 4096):
        return None
    return pos


def _storage_keys(world, max_regions, max_chunks):
    paths = [("chunk", world.region_dir, world.region_extension)]
    entity_folder = world.region_dir.parent / "entities"
    if not world.is_mcregion and entity_folder.is_dir():
        paths.append(("entities", entity_folder, ".mca"))
    from .worldstream import REGION_NAME
    files_count = 0
    for source, directory, extension in paths:
        files_count += sum(
            1 for path in directory.glob("r.*.*" + extension)
            if path.is_file() and REGION_NAME.fullmatch(path.name))
    if files_count > max_regions:
        raise ValueError(
            f"Original Java entity search has {files_count} region files; "
            f"limit {max_regions}")
    keys, invalid = [], []
    for source, directory, extension in paths:
        index = RegionIndex(directory, extension=extension)
        keys.extend((source, cx, cz) for cx, cz in index.chunks)
        invalid.extend((source, filename, slot)
                       for filename, slot in index.invalid)
        if len(keys) > max_chunks:
            raise ValueError(
                f"Original entity search exceeds {max_chunks} generated storage chunks")
    return sorted(keys, key=lambda x: (x[2], x[1], x[0])), invalid


def search_generated_entities(world, identifier="*", *, include_staged=True,
                              staged_snapshot=None,
                              cancel_event: Event | None = None,
                              progress: Callable[[int, int], None] | None = None,
                              max_chunks=MAX_ENTITY_CHUNKS,
                              max_regions=MAX_ENTITY_REGIONS,
                              max_results=MAX_ENTITY_RESULTS):
    """Cancelable bounded search of original distinct entity storage files.

    The staged dictionaries should be copied on the GUI thread *before*
    starting the scanner. The read-only worker owns a separate world reader.
    Every chunk is validated as a complete unit before matches are counted.
    """
    if (type(identifier) is not str or
            (identifier != "*" and
             (len(identifier.encode("utf-8")) > 512 or
              not IDENT.fullmatch(identifier)))):
        raise ValueError("Use a native namespaced entity ID or '*'")
    if type(include_staged) is not bool:
        raise ValueError("include_staged must be Boolean")
    for value, cap, name in (
            (max_chunks, MAX_ENTITY_CHUNKS, "chunks"),
            (max_regions, MAX_ENTITY_REGIONS, "regions"),
            (max_results, MAX_ENTITY_RESULTS, "results")):
        if type(value) is not int or not 1 <= value <= cap:
            raise ValueError(f"Entity search {name} exceeds safety budget")
    if (not isinstance(getattr(world, "region_dir", None), Path)
            or not callable(getattr(world, "_entity_chunk", None))):
        raise ValueError("Native Java entity region reader required")

    keys, invalid = _storage_keys(world, max_regions, max_chunks)
    keyset = set(keys)
    snapshot = staged_snapshot if staged_snapshot is not None else {
        "edits": dict(getattr(world, "_entity_edits", {})),
        "deletions": set(getattr(world, "_entity_deletions", ())),
        "additions": dict(getattr(world, "_entity_additions", {})),
        "transfers": dict(getattr(world, "_entity_transfers", {})),
    }
    if not include_staged:
        snapshot = {"edits": {}, "deletions": set(),
                    "additions": {}, "transfers": {}}
    if (not isinstance(snapshot, dict) or
            not isinstance(snapshot.get("edits"), dict) or
            not isinstance(snapshot.get("deletions"), set) or
            not isinstance(snapshot.get("additions"), dict) or
            not isinstance(snapshot.get("transfers"), dict)):
        raise ValueError("Invalid copied native entity edit snapshot")
    edits, deletions = snapshot["edits"], snapshot["deletions"]
    additions, transfers = snapshot["additions"], snapshot["transfers"]
    staged_index = {}
    orphaned = []
    for kind, values in (
            ("edit", edits), ("delete", deletions),
            ("add", additions), ("transfer", transfers)):
        for key in values:
            if (not isinstance(key, tuple) or len(key) != 4
                    or key[0] not in ("chunk", "entities")
                    or type(key[1]) is not int or type(key[2]) is not int):
                raise ValueError(f"Malformed original entity {kind} key")
            source, cx, cz, index = key
            item = (source, cx, cz)
            if item not in keyset:
                orphaned.append(f"{kind} {key}")
                continue
            staged_index.setdefault(item, {}).setdefault(kind, []).append(key)
            if kind == "transfer":
                target = transfers[key]
                if (not isinstance(target, tuple) or len(target) != 3
                        or type(target[0]) is not int
                        or type(target[1]) is not int):
                    raise ValueError("Malformed staged entity transfer target")
                destkey = (source, target[0], target[1])
                if destkey not in keyset:
                    orphaned.append(f"transfer destination {key} -> {destkey}")
                else:
                    staged_index.setdefault(destkey, {}).setdefault(
                        "incoming", []).append(key)

    stop = cancel_event if cancel_event is not None else Event()
    matches, failures = [], []
    examined = decoded = total = 0
    if progress is not None:
        progress(0, len(keys))
    for source, cx, cz in keys:
        if stop.is_set():
            break
        try:
            original_raw = world._entity_chunk(source, cx, cz)
            _validate_storage_chunk(original_raw, source, cx, cz)
            records = read_entities(original_raw, source)
            if len(records) > MAX_ENTITY_PER_CHUNK:
                raise ValueError("Original entity chunk exceeds safe record budget")
            if any(item.identifier == "<unidentified>" for item in records):
                raise ValueError(
                    "Original entity has no valid String id; exact-ID search "
                    "cannot claim full coverage for this chunk")
            records_by_index = {item.storage_index: item for item in records}
            changes = staged_index.get((source, cx, cz), {})
            # Validate staged references and construct a local complete snapshot.
            output = []
            for key in changes.get("edit", ()):
                if (type(key[3]) is not int or key[3] not in records_by_index
                        or not isinstance(edits[key], bytes)):
                    raise ValueError("Staged entity edit references invalid original index")
            for key in changes.get("delete", ()):
                if type(key[3]) is not int or key[3] not in records_by_index:
                    raise ValueError("Staged entity deletion references invalid index")
            for key in changes.get("transfer", ()):
                if type(key[3]) is not int or key[3] not in records_by_index:
                    raise ValueError("Staged entity transfer references invalid index")
            modified = {key[3]: edits[key] for key in changes.get("edit", ())}
            removed = {key[3] for key in changes.get("delete", ())}
            outgoing = {key[3] for key in changes.get("transfer", ())}
            if removed & outgoing:
                raise ValueError("Same original entity staged for both delete and transfer")
            for record in records:
                idx = record.storage_index
                payload = modified.get(idx)
                effective = (_validate_entity_payload(payload, source)
                             if payload is not None else record)
                state = ("deleted" if idx in removed else
                         "transferred_out" if idx in outgoing else
                         "modified" if payload is not None and
                         payload != record.payload else "original")
                candidate = record if state in ("deleted", "transferred_out") else effective
                output.append(WorldEntityMatch(
                    candidate.identifier, _validated_position(candidate),
                    source, (cx, cz), idx, state))
            for key in changes.get("add", ()):
                entity = _validate_entity_payload(additions[key], source)
                output.append(WorldEntityMatch(
                    entity.identifier, _validated_position(entity),
                    source, (cx, cz), -1, "created", str(key[3])))
            for key in changes.get("incoming", ()):
                target = transfers[key]
                entity = _validate_entity_payload(target[2], source)
                output.append(WorldEntityMatch(
                    entity.identifier, _validated_position(entity),
                    source, (cx, cz), -1, "transferred_in", str(key[3])))
            if len(output) > MAX_ENTITY_PER_CHUNK:
                raise ValueError("Staged entity chunk exceeds safe record budget")
            # Only a fully validated original+staged chunk contributes results.
            batch = [item for item in output
                     if identifier == "*" or item.identifier == identifier]
            decoded += 1
            total += len(batch)
            if len(matches) < max_results:
                matches.extend(batch[:max_results-len(matches)])
        except (ValueError, OSError, EOFError, IndexError, KeyError) as exc:
            failures.append(WorldEntityFailure(source, (cx, cz), str(exc)[:240]))
        examined += 1
        if progress is not None and (examined % 8 == 0 or examined == len(keys)):
            progress(examined, len(keys))
    if progress is not None:
        progress(examined, len(keys))
    return WorldEntitySearchResult(
        identifier, include_staged, len(keys), examined, decoded, total,
        tuple(matches), tuple(failures), len(invalid),
        tuple(f"{src} {path} (header {slot})"
              for src, path, slot in invalid[:32]),
        tuple(orphaned), stop.is_set())
