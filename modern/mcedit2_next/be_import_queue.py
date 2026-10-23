"""Atomic, BE-preserving pending structure imports across original Anvil chunks.

Independent structure exports are preflighted *together*, then staged in one
Undo transaction. No BE overwrite, cross-DataVersion remap, new chunk creation,
or implicit last-writer-wins on overlapping structure footprints.
"""
from __future__ import annotations

from dataclasses import dataclass

from .core import EditSession
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
