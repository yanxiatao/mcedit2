"""Atomic, BE-preserving pending structure imports across original Anvil chunks.

Independent structure exports are preflighted *together*, then staged in one
Undo transaction. No BE overwrite, cross-DataVersion remap, new chunk creation,
or implicit last-writer-wins on overlapping structure footprints.
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import hashlib

from .core import EditSession
from .structure_io import MAX_COMPRESSED_BYTES
from .structure_be import (
    BETransfer, BETransferError, MAX_BE_BYTES, MAX_BE_RECORDS,
    _plan_be_transfer,
)

MAX_PENDING_BE_IMPORTS = 16


@dataclass(frozen=True)
class PendingBEImport:
    name: str
    transfer: BETransfer
    origin: tuple[int, int, int]
    skip_air: bool = False
    only_replace_air: bool = False
    source_sha256: str | None = None
    world_token: int | None = None


@dataclass(frozen=True)
class BEQueuePreview:
    files: int
    input_blocks: int
    changed_blocks: int
    new_block_entities: int
    skipped_source_air: int
    skipped_target_nonair: int

    def summary(self):
        return (f"Structures: {self.files}; scanned blocks: {self.input_blocks:,}; "
                f"changes: {self.changed_blocks:,}; BE additions: "
                f"{self.new_block_entities:,}; skipped source air: "
                f"{self.skipped_source_air:,}; skipped target non-air: "
                f"{self.skipped_target_nonair:,}")


def _verify_queue_source(session: EditSession, entry: PendingBEImport):
    """Refuse stale expert queued files or entries from another world instance."""
    if entry.world_token is not None and entry.world_token != id(session.world):
        raise BETransferError("Queued BE file belongs to a different open world")
    if entry.source_sha256 is None:
        return  # Direct in-memory transfers have no external source file.
    digest = entry.source_sha256
    if (type(digest) is not str or len(digest) != 64
            or any(ch not in "0123456789abcdef" for ch in digest)):
        raise BETransferError("Invalid queued BE source fingerprint")
    source = Path(entry.name)
    try:
        if not source.is_file() or source.stat().st_size > MAX_COMPRESSED_BYTES:
            raise BETransferError("Queued BE source file missing or oversized")
        sha = hashlib.sha256()
        with source.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1048576), b""):
                sha.update(chunk)
    except OSError as exc:
        raise BETransferError("Queued BE source file is no longer readable") from exc
    if sha.hexdigest() != digest:
        raise BETransferError("Queued BE file changed since enqueue; remove and reload")


def plan_be_queue(session: EditSession, entries: tuple[PendingBEImport, ...]):
    """Read-only full native preflight; returns planned changes and summary.

    Even skipped blocks reserve their destination footprint, forbidding
    silent overwrite by another queued structure with different mask choices.
    """
    if not isinstance(session, EditSession) or not isinstance(entries, tuple):
        raise BETransferError("Expected native session and tuple of queued BE structures")
    if len(entries) > MAX_PENDING_BE_IMPORTS:
        raise BETransferError("BE queue exceeds 16 structures")
    if not entries:
        return [], BEQueuePreview(0, 0, 0, 0, 0, 0)
    staged = []
    occupied = set()
    total_blocks = total_be = total_bytes = 0
    skipped_source = skipped_target = 0
    for entry in entries:
        if not isinstance(entry, PendingBEImport) or not isinstance(entry.name, str):
            raise BETransferError("Invalid queued BE entry")
        _verify_queue_source(session, entry)
        transfer = entry.transfer
        total_blocks += len(transfer.structure.blocks)
        total_be += len(transfer.block_entities)
        total_bytes += sum(len(raw) for _, raw in transfer.block_entities)
        session._check_size(total_blocks)
        if total_be > MAX_BE_RECORDS or total_bytes > MAX_BE_BYTES:
            raise BETransferError("Combined queued BE budget exceeded")
        if type(entry.origin) is not tuple or len(entry.origin) != 3 or any(
                type(coord) is not int for coord in entry.origin):
            raise BETransferError("Queued origin must contain three integer coordinates")
        for relative, _ in transfer.structure.blocks:
            point = tuple(entry.origin[i] + relative[i] for i in range(3))
            if point in occupied:
                raise BETransferError(f"Queued structures overlap at {point}; edit origins")
            occupied.add(point)
        changes, report = _plan_be_transfer(
            session, transfer, entry.origin,
            skip_air=entry.skip_air, only_replace_air=entry.only_replace_air)
        staged.extend(changes)
        skipped_source += report.skipped_source_air
        skipped_target += report.skipped_target_nonair
    return staged, BEQueuePreview(
        files=len(entries), input_blocks=total_blocks,
        changed_blocks=len(staged),
        new_block_entities=sum(after.block_entity is not None
                               for _, _, after in staged),
        skipped_source_air=skipped_source,
        skipped_target_nonair=skipped_target,
    )


def preview_be_queue(session: EditSession, entries: tuple[PendingBEImport, ...]):
    return plan_be_queue(session, entries)[1]


def commit_be_queue(session: EditSession, entries: tuple[PendingBEImport, ...]):
    """Revalidate ALL pending structures, then commit once; no partial saves."""
    edits, _ = plan_be_queue(session, entries)
    if not edits:
        return 0
    session._apply(edits, reverse=False)
    session.undo_stack.append(edits)
    session.undo_stack = session.undo_stack[-100:]
    session.redo_stack.clear()
    session.dirty = True
    return len(edits)


def plan_mixed_queue(session: EditSession, entries: tuple):
    """Read-only preflight for original block-only and BE structures together.

    Reject all mixed-queue overlap, even for skipped cells. The existing
    separate pure block-only queue retains its historical last-wins behavior.
    """
    if not isinstance(entries, tuple) or len(entries) > MAX_PENDING_BE_IMPORTS:
        raise BETransferError("Mixed queue must contain at most 16 entries")
    if not any(isinstance(e, PendingBEImport) for e in entries):
        raise BETransferError("Use the original block-only queue for plain structures")
    occupied = set()
    planned = []
    blocks = records = raw_bytes = skipped_source = skipped_target = 0
    for entry in entries:
        if isinstance(entry, PendingBEImport):
            _verify_queue_source(session, entry)
            transfer = entry.transfer
            source = transfer.structure.blocks
            records += len(transfer.block_entities)
            raw_bytes += sum(len(payload) for _, payload in transfer.block_entities)
        else:
            if not hasattr(entry, "structure") or not hasattr(entry, "version_confirmed"):
                raise BETransferError("Unknown mixed queue entry")
            transfer = None
            if (getattr(entry, "world_token", None) is not None
                    and entry.world_token != id(session.world)):
                raise BETransferError("Queued blocks belong to a different open world")
            source = entry.structure.blocks
            version = entry.structure.data_version
            if (version and version != session.world.version
                    and not entry.version_confirmed):
                raise BETransferError("Unconfirmed queued block DataVersion mismatch")
        if not isinstance(entry.origin, tuple) or len(entry.origin) != 3 or any(
                type(v) is not int for v in entry.origin):
            raise BETransferError("Mixed queue origin must contain integer XYZ")
        sx, sy, sz = (entry.structure.size if transfer is None
                      else transfer.structure.size)
        if not sx or not sy or not sz or sx * sy * sz > session.max_blocks:
            raise BETransferError("Invalid queued structure bounds")
        blocks += len(source)
        session._check_size(blocks)
        if records > MAX_BE_RECORDS or raw_bytes > MAX_BE_BYTES:
            raise BETransferError("Mixed queue opaque BE budget exceeded")
        for relative, _ in source:
            if len(relative) != 3 or any(
                    not 0 <= relative[i] < (sx, sy, sz)[i] for i in range(3)):
                raise BETransferError("Malformed queued relative coordinate")
            point = tuple(entry.origin[i] + relative[i] for i in range(3))
            if point in occupied:
                raise BETransferError(f"Mixed queue overlap at {point}")
            occupied.add(point)
        if transfer is not None:
            changes, preview = _plan_be_transfer(
                session, transfer, entry.origin,
                skip_air=entry.skip_air, only_replace_air=entry.only_replace_air)
            planned.extend(changes)
            skipped_source += preview.skipped_source_air
            skipped_target += preview.skipped_target_nonair
        else:
            preflight = getattr(session.world, "validate_block_write", None)
            if not callable(preflight):
                raise BETransferError("Mixed queue requires native block write validation")
            for relative, after in source:
                point = tuple(entry.origin[i] + relative[i] for i in range(3))
                before = session.world.get_block(*point)
                if before != after:
                    preflight(*point, after)
                    planned.append((point, before, after))
    return planned, BEQueuePreview(
        len(entries), blocks, len(planned),
        sum(hasattr(after, "block_entity") and after.block_entity is not None
            for _, _, after in planned), skipped_source, skipped_target)


def preview_mixed_queue(session: EditSession, entries: tuple):
    return plan_mixed_queue(session, entries)[1]


def commit_mixed_queue(session: EditSession, entries: tuple):
    """All-or-nothing mixed BE+plain block transaction, one Undo step."""
    changes, _ = plan_mixed_queue(session, entries)
    if not changes:
        return 0
    session._apply(changes, reverse=False)
    session.undo_stack.append(changes)
    session.undo_stack = session.undo_stack[-100:]
    session.redo_stack.clear()
    session.dirty = True
    return len(changes)
