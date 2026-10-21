"""Version-neutral block transactions for the Python 3 UI.

World serialization and block entities belong to the backend, never to this core.
"""
from __future__ import annotations
from collections import deque
from dataclasses import dataclass
from typing import Any, Protocol

class World(Protocol):
    def get_block(self, x: int, y: int, z: int) -> Any: ...
    def set_block(self, x: int, y: int, z: int, block: Any) -> None: ...
    def save(self) -> None: ...

Position = tuple[int, int, int]
Change = tuple[Position, Any, Any]


@dataclass(frozen=True)
class BEPayloadEdit:
    """Complete original NBT bytes before/after one explicit structural edit."""
    point: Position
    before: bytes
    after: bytes


@dataclass(frozen=True)
class BEScalarEdit:
    """Type-preserving change to one existing BE NBT scalar."""
    point: Position
    path: tuple[str | int, ...]
    before: Any
    after: Any


@dataclass(frozen=True)
class EntityMove:
    """One original-format same-chunk entity position edit."""
    source: str
    cx: int
    cz: int
    index: int
    before: tuple[float, float, float]
    after: tuple[float, float, float]


@dataclass(frozen=True)
class EntityRotation:
    """Undoable, original-format entity yaw/pitch mutation in its same chunk."""
    source: str
    cx: int
    cz: int
    index: int
    before: tuple[float, float]
    after: tuple[float, float]


@dataclass(frozen=True)
class EntityScalarEdit:
    """One strictly whitelisted original entity NBT scalar edit."""
    source: str
    cx: int
    cz: int
    index: int
    path: tuple[str, ...]
    before: Any
    after: Any


@dataclass(frozen=True)
class EntityRemoval:
    """Staged original-list deletion; valid for undo/redo BEFORE disk save."""
    source: str
    cx: int
    cz: int
    index: int
    before: bool = False
    after: bool = True


@dataclass(frozen=True)
class EntityAddition:
    """An unsaved native Entity Compound with a new stable UUID identity."""
    source: str
    cx: int
    cz: int
    token: str
    payload: bytes
    before: bool = False
    after: bool = True


@dataclass(frozen=True)
class EntityTransfer:
    """Move an existing original UUID between two existing Java chunks."""
    source: str
    cx: int
    cz: int
    index: int
    dcx: int
    dcz: int
    payload: bytes
    before: bool = False
    after: bool = True


MAX_OPERATION_BLOCKS = 32768

@dataclass(frozen=True)
class Box:
    minimum: Position
    maximum: Position

    @classmethod
    def from_corners(cls, a: Position, b: Position) -> Box:
        return cls(tuple(min(a[i], b[i]) for i in range(3)),
                   tuple(max(a[i], b[i]) for i in range(3)))

    @property
    def size(self) -> Position:
        return tuple(self.maximum[i] - self.minimum[i] + 1 for i in range(3))

    @property
    def volume(self) -> int:
        x, y, z = self.size
        return x * y * z

    def points(self):
        for x in range(self.minimum[0], self.maximum[0] + 1):
            for y in range(self.minimum[1], self.maximum[1] + 1):
                for z in range(self.minimum[2], self.maximum[2] + 1):
                    yield x, y, z

@dataclass(frozen=True)
class CellSnapshot:
    """A named block state plus optional opaque BE compound payload."""
    block: Any
    block_entity: bytes | None = None


@dataclass(frozen=True)
class Clipboard:
    size: Position
    blocks: tuple[tuple[Position, Any], ...]
    with_nbt: bool = False

@dataclass(frozen=True)
class SelectionStatistics:
    """Read-only counts in original native block-state format."""
    total_blocks: int
    block_entities: int | None
    counts: tuple[tuple[str, int], ...]


@dataclass(frozen=True)
class ReplacementPreview:
    """Read-only summary for a native block filter (BE counts may be unknown)."""
    total_blocks: int
    matches: int
    changes: int
    protected_be: int | None


class EditSession:
    def __init__(self, world: World, max_blocks: int = MAX_OPERATION_BLOCKS):
        self.world = world
        self.max_blocks = max_blocks
        self.undo_stack: list[list[Change]] = []
        self.redo_stack: list[list[Change]] = []
        self.clipboard: Clipboard | None = None
        self.dirty = False

    def _check_size(self, size: int):
        if size > self.max_blocks:
            raise ValueError(f"Operation would affect {size:,} blocks; limit is {self.max_blocks:,}")

    def _apply(self, changes: list[Change | EntityMove], reverse: bool):
        def apply_one(change, undo):
            if isinstance(change, EntityTransfer):
                moved = change.before if undo else change.after
                self.world.set_entity_transferred(
                    change.source, change.cx, change.cz, change.index,
                    change.dcx, change.dcz, change.payload, moved)
                return
            if isinstance(change, EntityAddition):
                added = change.before if undo else change.after
                self.world.set_entity_added(
                    change.source, change.cx, change.cz, change.token,
                    change.payload, added)
                return
            if isinstance(change, EntityRemoval):
                removed = change.before if undo else change.after
                self.world.set_entity_deleted(
                    change.source, change.cx, change.cz,
                    change.index, removed)
                return
            if isinstance(change, EntityMove):
                value = change.before if undo else change.after
                self.world.set_entity_position(
                    change.source, change.cx, change.cz, change.index, value)
                return
            if isinstance(change, EntityRotation):
                value = change.before if undo else change.after
                self.world.set_entity_rotation(
                    change.source, change.cx, change.cz, change.index, value)
                return
            if isinstance(change, EntityScalarEdit):
                value = change.before if undo else change.after
                self.world.set_entity_scalar(
                    change.source, change.cx, change.cz, change.index,
                    change.path, value)
                return
            if isinstance(change, BEScalarEdit):
                value = change.before if undo else change.after
                self.world.set_block_entity_scalar(
                    change.point, change.path, value)
                return
            if isinstance(change, BEPayloadEdit):
                value = change.before if undo else change.after
                self.world.set_block_entity_payload(change.point, value)
                return
            point, before, after = change
            value = before if undo else after
            if isinstance(value, CellSnapshot):
                self.world.set_cell(*point, value, allow_existing_be=True)
            else:
                self.world.set_block(*point, value)

        completed = []
        try:
            for change in (reversed(changes) if reverse else changes):
                apply_one(change, reverse)
                completed.append(change)
        except Exception:
            for change in reversed(completed):
                try:
                    apply_one(change, not reverse)
                except Exception:
                    pass
            raise

    def _commit(self, updates: list[tuple[Position, Any]]) -> int:
        self._check_size(len(updates))
        prepared = []
        for point, after in updates:
            before = self.world.get_block(*point)
            if before != after:
                prepared.append((point, before, after))
        if not prepared:
            return 0
        # Backends may verify native section existence, version-specific block
        # IDs and protected BE positions without staging any mutations.
        # A rejected destination must not leave a partial move behind.
        preflight = getattr(self.world, "validate_block_write", None)
        if callable(preflight):
            for point, before, after in prepared:
                if isinstance(after, CellSnapshot):
                    continue  # Explicit BE-aware transaction has its own checks.
                preflight(*point, after)
        self._apply(prepared, reverse=False)
        self.undo_stack.append(prepared)
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return len(prepared)

    def paint(self, point: Position, block: Any) -> int:
        return self._commit([(point, block)])

    def fill(self, box: Box, block: Any) -> int:
        self._check_size(box.volume)
        return self._commit([(point, block) for point in box.points()])

    def hollow_box(self, box: Box, block: Any, thickness: int = 1) -> int:
        """Fill only the shell of a selection, preserving its interior.

        Uses a single undo record. Bounds are validated before enumerating.
        """
        if not 1 <= thickness <= 64:
            raise ValueError("Shell thickness must be in 1..64")
        self._check_size(box.volume)
        lo, hi = box.minimum, box.maximum
        updates = (
            (p, block) for p in box.points()
            if any((p[i] - lo[i] < thickness or
                    hi[i] - p[i] < thickness) for i in range(3))
        )
        return self._commit(list(updates))

    def sphere(self, center: Position, radius: int, block: Any,
               hollow: bool = False) -> int:
        """Draw a solid or hollow 3D sphere centered at an integer coordinate."""
        if not 1 <= radius <= 128:
            raise ValueError("Sphere radius must be between 1 and 128")
        if (2 * radius + 1) ** 3 > self.max_blocks * 12:
            raise ValueError("Sphere bounding volume exceeds safe operation limit")
        updates = []
        outer = radius * radius
        inner = (radius - 1) * (radius - 1)
        for y in range(-radius, radius + 1):
            for z in range(-radius, radius + 1):
                for x in range(-radius, radius + 1):
                    d2 = x*x + y*y + z*z
                    if d2 > outer or (hollow and d2 < inner):
                        continue
                    updates.append(((center[0] + x, center[1] + y,
                                     center[2] + z), block))
                    self._check_size(len(updates))
        return self._commit(updates)

    def line(self, start: Position, end: Position, block: Any) -> int:
        """Draw a continuous integer 3D line inclusive of both endpoints."""
        difference = tuple(end[i] - start[i] for i in range(3))
        steps = max(abs(value) for value in difference)
        self._check_size(steps + 1)
        if steps == 0:
            return self.paint(start, block)
        points = []
        for index in range(steps + 1):
            point = tuple(round(start[i] + difference[i] * index / steps)
                          for i in range(3))
            if not points or points[-1][0] != point:
                points.append((point, block))
        return self._commit(points)

    def find_blocks(self, box: Box, target: Any, *,
                    exact_state: bool = False, max_results: int = 512):
        """Read-only search of a bounded selection in its original block format.

        Returns (positions, total_matches). A non-exact search matches the
        native block identifier and ignores block-state properties/metadata.
        Only the first max_results positions are retained for the UI; the
        total count is exact within the selected volume. Never writes chunks
        or creates undo records. BEs are read through get_block only.
        """
        self._check_size(box.volume)
        if type(max_results) is not int or not 1 <= max_results <= 4096:
            raise ValueError("Search display limit must be between 1 and 4096")
        if not exact_state:
            identifier = (target.name if hasattr(target, "name")
                          else str(target).split("[", 1)[0])
        result = []
        count = 0
        for point in box.points():
            state = self.world.get_block(*point)
            matches = (state == target if exact_state else
                       (state.name if hasattr(state, "name")
                        else str(state).split("[", 1)[0]) == identifier)
            if matches:
                count += 1
                if len(result) < max_results:
                    result.append(point)
        return tuple(result), count

    def selection_statistics(self, box: Box, *,
                             exact_state: bool = False) -> SelectionStatistics:
        """Scan bounded native states and BE presence without modifying chunks.

        Lack of block-entity support is reported as unknown, never as zero.
        Any unreadable Section aborts the scan before a result is produced.
        """
        self._check_size(box.volume)
        if type(exact_state) is not bool:
            raise ValueError("exact_state must be a bool")
        checker = getattr(self.world, "has_block_entity", None)
        be_count = 0 if callable(checker) else None
        counts: dict[str, int] = {}
        for point in box.points():
            state = self.world.get_block(*point)
            key = (str(state) if exact_state else
                   (state.name if hasattr(state, "name")
                    else str(state).split("[", 1)[0]))
            counts[key] = counts.get(key, 0) + 1
            if be_count is not None and checker(*point):
                be_count += 1
        return SelectionStatistics(
            box.volume, be_count,
            tuple(sorted(counts.items(), key=lambda item: (-item[1], item[0]))))

    def _scan_replace_matching(self, box: Box, source: Any, target: Any, *,
                               exact_state: bool):
        """Gather native-only changes before any write or undo mutation."""
        self._check_size(box.volume)
        if type(exact_state) is not bool:
            raise ValueError("exact_state must be a bool")
        checker = getattr(self.world, "has_block_entity", None)
        be_count = 0 if callable(checker) else None
        source_id = (source.name if hasattr(source, "name")
                     else str(source).split("[", 1)[0])
        matches = 0
        updates = []
        for point in box.points():
            before = self.world.get_block(*point)
            identifier = (before.name if hasattr(before, "name")
                          else str(before).split("[", 1)[0])
            if not (before == source if exact_state else identifier == source_id):
                continue
            matches += 1
            if before == target:
                continue
            updates.append((point, target))
            if be_count is not None and checker(*point):
                be_count += 1
        return (ReplacementPreview(box.volume, matches, len(updates), be_count),
                updates)

    def preview_replace_matching(self, box: Box, source: Any, target: Any, *,
                                 exact_state: bool = False) -> ReplacementPreview:
        """Native state/BE-only preview; no block, region, NBT or undo edits."""
        preview, _ = self._scan_replace_matching(
            box, source, target, exact_state=exact_state)
        return preview

    def replace_matching(self, box: Box, source: Any, target: Any, *,
                         exact_state: bool = False) -> int:
        """Commit one undoable filtered replace, refusing any BE conflict.

        Matching by ID ignores properties, including legacy numeric metadata.
        Native backend preflight still validates every proposed new state.
        """
        preview, updates = self._scan_replace_matching(
            box, source, target, exact_state=exact_state)
        if not updates:
            return 0
        if preview.protected_be is None:
            raise ValueError("Backend must expose BE presence for safe filtered replacement")
        if preview.protected_be:
            raise ValueError(
                f"{preview.protected_be} matching Block Entity locations; "
                "refusing the entire filtered replacement")
        return self._commit(updates)

    def replace(self, box: Box, source: Any, target: Any) -> int:
        self._check_size(box.volume)
        return self._commit([(p, target) for p in box.points()
                             if self.world.get_block(*p) == source])

    def flood_fill(self, start: Position, replacement: Any,
                   bounds: Box | None = None) -> int:
        """Replace six-connected blocks without leaving optional selection bounds.

        Bounding a fill to a selection also prevents accidental traversal of
        entire generated worlds through contiguous air.
        """
        if bounds is not None:
            if any(not bounds.minimum[i] <= start[i] <= bounds.maximum[i]
                   for i in range(3)):
                raise ValueError("Flood fill seed must be inside selection")
            self._check_size(bounds.volume)
        original = self.world.get_block(*start)
        if original == replacement:
            return 0
        queue = deque([start])
        seen = {start}
        updates = []
        while queue:
            point = queue.popleft()
            if self.world.get_block(*point) != original:
                continue
            updates.append((point, replacement))
            self._check_size(len(updates))
            x, y, z = point
            for neighbor in ((x + 1, y, z), (x - 1, y, z),
                             (x, y + 1, z), (x, y - 1, z),
                             (x, y, z + 1), (x, y, z - 1)):
                if neighbor in seen:
                    continue
                if bounds is not None and any(
                        not bounds.minimum[i] <= neighbor[i] <= bounds.maximum[i]
                        for i in range(3)):
                    continue
                seen.add(neighbor)
                queue.append(neighbor)
        return self._commit(updates)

    @staticmethod
    def _offset3(offset: Position) -> Position:
        if (len(offset) != 3 or
                any(type(n) is not int for n in offset)):
            raise ValueError("Offset must be three integer block coordinates")
        return offset

    def _ensure_block_only(self, points):
        checker = getattr(self.world, "has_block_entity", None)
        if callable(checker):
            for point in points:
                if checker(*point):
                    raise ValueError(
                        f"Block entity at {point}; clone/move are block-only. "
                        "Use an explicit BE-aware tool after checking mod links.")

    def clone_box(self, box: Box, offset: Position) -> int:
        """Clone a selection by a fixed offset without changing blockstates.

        The source is fully captured before any modification, including for
        overlapping selections. Entity and BE data are never copied here.
        """
        dx, dy, dz = self._offset3(offset)
        self._check_size(box.volume)
        if not (dx or dy or dz):
            return 0
        source = tuple(box.points())
        targets = tuple((x + dx, y + dy, z + dz) for x, y, z in source)
        self._ensure_block_only((*source, *targets))
        states = tuple(self.world.get_block(*point) for point in source)
        return self._commit(list(zip(targets, states)))

    def clone_box_repeated(self, box: Box, offset: Position,
                           repetitions: int) -> int:
        """Tile an original block-only snapshot at consecutive step offsets.

        Corresponds to MCEdit2 CloneTool's repeat/tiling workflow. Each
        stamp uses the SAME original snapshot, including when a destination
        overlaps the source. Later stamps win if destinations overlap.
        Deduplicating the targets before _commit is essential: duplicate
        history entries for one cell would corrupt the Undo baseline.

        No entities or Block Entities are transferred. All source and
        target BE positions, native IDs and existing destination sections
        are checked before staging a single world edit.
        """
        dx, dy, dz = self._offset3(offset)
        if type(repetitions) is not int or not 1 <= repetitions <= 128:
            raise ValueError("Clone repeat count must be an integer in 1..128")
        if not (dx or dy or dz):
            raise ValueError("Clone tiling requires a nonzero XYZ step")
        # Cap *planned* work before materializing any source/target blocks,
        # even when many stamps overlap and the unique destination is small.
        self._check_size(box.volume * repetitions)
        source = tuple(box.points())
        # Read the complete original source before any staged world mutation.
        self._ensure_block_only(source)
        original_states = tuple(self.world.get_block(*point)
                                for point in source)
        destinations: dict[Position, Any] = {}
        for repeat in range(1, repetitions + 1):
            ox, oy, oz = dx * repeat, dy * repeat, dz * repeat
            for (x, y, z), state in zip(source, original_states):
                # Keys are unique; later stamps take precedence. A target
                # may overlap the source, but the snapshot cannot cascade.
                destinations[(x + ox, y + oy, z + oz)] = state
        self._ensure_block_only(destinations)
        return self._commit(list(destinations.items()))

    def move_box(self, box: Box, offset: Position, air: Any) -> int:
        """Shift a selection without converting its block states or metadata.

        Preflight all coordinates, including overlap. Snapshot the source first,
        then clear source-only cells and populate destination cells in one
        undoable transaction. Entities and BEs are intentionally excluded.
        """
        dx, dy, dz = self._offset3(offset)
        self._check_size(box.volume)
        if not (dx or dy or dz):
            return 0
        source = tuple(box.points())
        targets = tuple((x + dx, y + dy, z + dz) for x, y, z in source)
        affected = dict.fromkeys(source, air)
        self._check_size(len(set((*source, *targets))))
        self._ensure_block_only((*source, *targets))
        states = tuple(self.world.get_block(*point) for point in source)
        affected.update(zip(targets, states))
        return self._commit(list(affected.items()))

    def transform_selection(self, box: Box, operation: str, air: Any) -> int:
        """Rotate or reflect a block-only selection in its original format.

        The transformed volume is anchored at the selection minimum. A
        90-degree rotation exchanges X/Z extents. Both source and destination
        are preflighted before staging any edits, including original BEs.
        """
        from .transforms import (transform_position, transformed_size,
                                 transform_state)
        self._check_size(box.volume)
        source = tuple(box.points())
        new_size = transformed_size(box.size, operation)
        target_min = box.minimum
        target_box = Box(
            target_min,
            tuple(target_min[i] + new_size[i] - 1 for i in range(3)))
        dest = tuple(target_box.points())
        # Bound total writes before we read or compute rotated block states.
        self._check_size(len(set((*source, *dest))))
        self._ensure_block_only((*source, *dest))
        original_states = tuple(self.world.get_block(*p) for p in source)
        legacy = bool(getattr(self.world, "legacy", False))
        updates = dict.fromkeys(source, air)
        for point, state in zip(source, original_states):
            rel = tuple(point[i] - box.minimum[i] for i in range(3))
            rotated = transform_position(rel, box.size, operation)
            dst = tuple(box.minimum[i] + rotated[i] for i in range(3))
            updates[dst] = transform_state(state, operation, legacy=legacy)
        return self._commit(list(updates.items()))

    def edit_block_entity_structure(self, point: Position, operation: str,
                                    path: tuple[str | int, ...], **kwargs) -> int:
        """Atomic staged expert NBT field/list modification, with full undo."""
        from .nbt_structure import edit_structure
        before = self.world.get_block_entity_raw(*point)
        if before is None:
            raise ValueError("Block Entity does not exist")
        after = edit_structure(before, operation, tuple(path), **kwargs)
        if before == after:
            return 0
        change = BEPayloadEdit(tuple(point), before, after)
        self._apply([change], reverse=False)
        self.undo_stack.append([change])
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return 1

    def edit_block_entity_scalar(self, point: Position,
                                 path: tuple[str | int, ...], value: Any) -> int:
        """Undoable, original-tag-type edit without NBT reconstruction."""
        getter = getattr(self.world, "get_block_entity_scalar", None)
        if not callable(getter):
            raise ValueError("Backend does not support safe BE scalar editing")
        before = getter(tuple(point), tuple(path))
        if before == value:
            return 0
        change = BEScalarEdit(tuple(point), tuple(path), before, value)
        self._apply([change], reverse=False)
        self.undo_stack.append([change])
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return 1

    def move_entity_in_chunk(self, source: str, cx: int, cz: int,
                             index: int, position) -> int:
        """Undoable native Pos-only relocation, restricted to one chunk.

        The backend validates entity type, link tags, coordinate boundaries
        and data layout before any staging or disk changes.
        """
        if not hasattr(self.world, "get_entity_position"):
            raise ValueError("Current backend cannot move entities safely")
        before = self.world.get_entity_position(source, cx, cz, index)
        try:
            after = tuple(float(n) for n in position)
        except (TypeError, ValueError) as exc:
            raise ValueError("Entity position requires three numeric values") from exc
        if len(after) != 3:
            raise ValueError("Entity position requires X, Y and Z")
        if before == after:
            return 0
        change = EntityMove(source, cx, cz, index, before, after)
        self._apply([change], reverse=False)
        self.undo_stack.append([change])
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return 1

    def move_entities_in_chunk(self, cx: int, cz: int,
                               updates: list[tuple[str, int, tuple]]) -> int:
        """Group same-chunk standalone entity moves into one undo record.

        Pure-preflight every entity before mutating any staged NBT. The
        existing EditSession _apply will also reverse staged earlier moves
        if a later entity unexpectedly fails.
        """
        if len(updates) > min(2048, self.max_blocks):
            raise ValueError("Entity move batch exceeds editor safety limit")
        validator = getattr(self.world, "validate_entity_move", None)
        getter = getattr(self.world, "get_entity_position", None)
        if not callable(validator) or not callable(getter):
            raise ValueError("Backend cannot safely preflight entity moves")
        prepared = []
        seen = set()
        for source, index, new_position in updates:
            if (not isinstance(source, str) or type(index) is not int or
                    type(cx) is not int or type(cz) is not int):
                raise ValueError("Invalid entity source, chunk or list index")
            key = (source, index)
            if key in seen:
                raise ValueError("Duplicate entity in grouped move")
            seen.add(key)
            if not isinstance(new_position, (tuple, list)) or len(new_position) != 3:
                raise ValueError("Entity target position requires X, Y, Z")
            if any(type(v) not in (int, float) for v in new_position):
                raise ValueError("Entity target position must be numeric")
            after = tuple(float(v) for v in new_position)
            before = getter(source, cx, cz, index)
            validator(source, cx, cz, index, after)
            if before != after:
                prepared.append(EntityMove(source, cx, cz, index, before, after))
        if not prepared:
            return 0
        self._apply(prepared, reverse=False)
        self.undo_stack.append(prepared)
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return len(prepared)

    def delete_entities_in_chunk(self, cx: int, cz: int,
                                 members: list[tuple[str, int]]) -> int:
        """Staged original-list deletion, grouped into a single Undo entry.

        Only the native backend's strict dropped-item/XP-orb allowlist is
        accepted. Removing list members changes their indices on disk:
        Undo/Redo is intentionally invalidated at SAVE. Use the complete
        region backup to restore committed deletion after saving.
        """
        if (type(cx) is not int or type(cz) is not int or
                not isinstance(members, (list, tuple)) or
                not 1 <= len(members) <= min(2048, self.max_blocks)):
            raise ValueError("Expected a bounded list of native entity selections")
        validator = getattr(self.world, "validate_entity_deletion", None)
        if not callable(validator):
            raise ValueError("Current backend cannot safely delete entities")
        changes = []
        seen = set()
        for member in members:
            if (not isinstance(member, tuple) or len(member) != 2 or
                    member[0] not in ("chunk", "entities") or
                    type(member[1]) is not int):
                raise ValueError("Invalid original entity source or list index")
            source, index = member
            if (source, index) in seen:
                raise ValueError("Duplicate original entity deletion selection")
            seen.add((source, index))
            validator(source, cx, cz, index)
            changes.append(EntityRemoval(source, cx, cz, index))
        # All candidates pass before staging ANY deletion.
        self._apply(changes, reverse=False)
        self.undo_stack.append(changes)
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return len(changes)

    def transfer_entity_to_chunk(self, source: str, cx: int, cz: int,
                                 index: int, destination) -> int:
        """Move one existing vanilla loose item/orb to an existing chunk.

        Reuses original UUID; both original regions join the same save
        transaction. Undo before save removes the staged transfer exactly.
        """
        maker = getattr(self.world, "prepare_entity_transfer", None)
        if not callable(maker):
            raise ValueError("World cannot preserve cross-chunk entity NBT")
        dcx, dcz, payload = maker(source, cx, cz, index, destination)
        change = EntityTransfer(source, cx, cz, index, dcx, dcz, payload)
        self._apply([change], reverse=False)
        self.undo_stack.append([change])
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return 1

    def transfer_entities_to_chunks(self, cx: int, cz: int,
                                    updates: list[tuple[str, int, tuple]]) -> int:
        """Move independent items/orbs to existing chunks in one Undo step.

        Every move is prepared and UUID-checked without staging first. If
        any later staging unexpectedly fails, _apply rolls back earlier
        changes and leaves the operation out of Undo history.
        """
        preparer = getattr(self.world, "prepare_entity_transfer_batch", None)
        if not callable(preparer):
            raise ValueError("World backend cannot preflight grouped entity transfers")
        prepared = preparer(cx, cz, updates)
        changes = [
            EntityTransfer(source, cx, cz, index, dcx, dcz, payload)
            for source, index, dcx, dcz, payload in prepared
        ]
        if not changes:
            return 0
        self._apply(changes, reverse=False)
        self.undo_stack.append(changes)
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return len(changes)

    def clone_entity_in_chunk(self, source: str, cx: int, cz: int,
                              index: int, position) -> int:
        """Duplicate only a safe original dropped item/orb with fresh UUID.

        Undo/Redo uses the *same prepared NBT bytes* and UUID. The native
        region save appends it after any staged deletions; structural
        history is cleared after a successful Save as indexes then change.
        """
        maker = getattr(self.world, "prepare_entity_clone", None)
        if not callable(maker):
            raise ValueError("This backend cannot safely clone original entities")
        if (type(index) is not int or index < 0 or
                not isinstance(position, (tuple, list)) or len(position) != 3 or
                any(type(x) not in (int, float) for x in position)):
            raise ValueError("Original entity clone requires a real source and XYZ")
        token, payload = maker(source, cx, cz, index, tuple(position))
        change = EntityAddition(source, cx, cz, token, payload)
        self._apply([change], reverse=False)
        self.undo_stack.append([change])
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return 1

    def rotate_entity_in_chunk(self, source: str, cx: int, cz: int,
                               index: int, rotation) -> int:
        """Undoable native entity yaw/pitch, without changing Pos or UUID."""
        reader = getattr(self.world, "get_entity_rotation", None)
        if not callable(reader):
            raise ValueError("Current backend cannot rotate entities safely")
        before = reader(source, cx, cz, index)
        try:
            after = tuple(float(n) for n in rotation)
        except (TypeError, ValueError, OverflowError) as exc:
            raise ValueError("Entity rotation requires two numeric values") from exc
        if len(after) != 2:
            raise ValueError("Entity rotation requires yaw and pitch")
        if before == after:
            return 0
        change = EntityRotation(source, cx, cz, index, before, after)
        self._apply([change], reverse=False)
        self.undo_stack.append([change])
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return 1

    def edit_entity_scalar(self, source: str, cx: int, cz: int,
                           index: int, path: tuple[str, ...], value) -> int:
        """Stage one original entity scalar with EditSession Undo/Redo."""
        reader = getattr(self.world, "get_entity_scalar", None)
        if not callable(reader):
            raise ValueError("Backend cannot read original entity scalars")
        path = tuple(path)
        before = reader(source, cx, cz, index, path)
        if before == value:
            return 0
        change = EntityScalarEdit(
            source, cx, cz, index, path, before, value)
        self._apply([change], reverse=False)
        self.undo_stack.append([change])
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return 1

    def copy(self, box: Box) -> Clipboard:
        self._check_size(box.volume)
        checker = getattr(self.world, "has_block_entity", None)
        if callable(checker):
            for point in box.points():
                if checker(*point):
                    raise ValueError(
                        f"Block entity at {point} cannot be copied without NBT. "
                        "The block-only clipboard is not a BE transfer format."
                    )
        a = box.minimum
        blocks = tuple(((p[0]-a[0], p[1]-a[1], p[2]-a[2]),
                        self.world.get_block(*p)) for p in box.points())
        self.clipboard = Clipboard(box.size, blocks)
        return self.clipboard

    def copy_with_nbt(self, box: Box) -> Clipboard:
        """Explicit BE-aware copy; original block-only copy stays unchanged."""
        self._check_size(box.volume)
        reader = getattr(self.world, "get_cell", None)
        if not callable(reader):
            raise ValueError("This world backend cannot copy BE NBT losslessly")
        base = box.minimum
        cells = tuple(
            ((p[0] - base[0], p[1] - base[1], p[2] - base[2]),
             reader(*p)) for p in box.points()
        )
        self.clipboard = Clipboard(box.size, cells, with_nbt=True)
        return self.clipboard

    def paste_with_nbt(self, origin: Position) -> int:
        """Paste snapshots, but never overwrite BE records already at target.

        Only root BE x/y/z tags are relocated. Mod UUIDs, internal links,
        ownership, and custom relative fields are not translated.
        """
        clipboard = self.clipboard
        if clipboard is None or not clipboard.with_nbt:
            raise ValueError("Copy selection with BE NBT first")
        self._check_size(len(clipboard.blocks))
        getter = getattr(self.world, "get_cell", None)
        checker = getattr(self.world, "has_block_entity", None)
        if not callable(getter) or not callable(checker):
            raise ValueError("Backend has no lossless BE snapshot support")
        # Import lazily; this core can still run without a native Anvil backend.
        from .be_nbt import relocate_payload
        changes = []
        for rel, snapshot in clipboard.blocks:
            point = tuple(origin[i] + rel[i] for i in range(3))
            if checker(*point):
                raise ValueError(
                    f"Target {point} contains BE data; refusing overwrite")
            source = tuple(rel[i] for i in range(3))
            # Clipboard BE payload retains source absolute coordinates.
            # Validate them from its NBT, then relocate to new target.
            be = snapshot.block_entity
            if be is not None:
                from . import nbtwire
                record = nbtwire.parse(nbtwire.make_tag(10, "", be))
                source = tuple(record.require(axis, 3).value
                               for axis in ("x", "y", "z"))
                be = relocate_payload(be, source, point)
            destination = CellSnapshot(snapshot.block, be)
            before = getter(*point)
            if before != destination:
                changes.append((point, before, destination))
        self._check_size(len(changes))
        if not changes:
            return 0
        self._apply(changes, reverse=False)
        self.undo_stack.append(changes)
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return len(changes)

    def paste(self, origin: Position) -> int:
        if self.clipboard is None:
            raise ValueError("Nothing copied yet")
        if self.clipboard.with_nbt:
            raise ValueError("Use Paste with BE NBT for this clipboard")
        self._check_size(len(self.clipboard.blocks))
        return self._commit([((origin[0]+p[0], origin[1]+p[1], origin[2]+p[2]), b)
                             for p, b in self.clipboard.blocks])

    def undo(self) -> bool:
        if not self.undo_stack:
            return False
        changes = self.undo_stack[-1]
        self._apply(changes, reverse=True)
        self.undo_stack.pop()
        self.redo_stack.append(changes)
        self.dirty = True
        return True

    def redo(self) -> bool:
        if not self.redo_stack:
            return False
        changes = self.redo_stack[-1]
        self._apply(changes, reverse=False)
        self.redo_stack.pop()
        self.undo_stack.append(changes)
        self.dirty = True
        return True

    def save(self):
        # Entity List contraction shifts native entity indices on disk.
        # Do not carry any pre-save EntityRemoval record into the new
        # original-index baseline; after saving recover via region backup.
        structural_history = bool(
            getattr(self.world, "_entity_deletions", ())
            or getattr(self.world, "_entity_additions", ())
            or getattr(self.world, "_entity_transfers", ())) or any(
                isinstance(change, (EntityRemoval, EntityAddition, EntityTransfer))
                for history in (self.undo_stack, self.redo_stack)
                for group in history for change in group
            )
        self.world.save()
        self.dirty = False
        if structural_history:
            self.undo_stack.clear()
            self.redo_stack.clear()


class EditTransaction:
    """Stage plugin block edits; publish them as one undoable operation.

    Callbacks read staged changes through get_block and set_block. No changes
    are written to the backend until commit. The backend remains responsible
    for rejecting edits to protected BE coordinates and invalid sections.
    """

    def __init__(self, session: EditSession):
        self.session = session
        self.changes: dict[Position, Any] = {}
        self._closed = False

    def get_block(self, point: Position):
        if point in self.changes:
            return self.changes[point]
        return self.session.world.get_block(*point)

    def set_block(self, point: Position, block: Any):
        if self._closed:
            raise RuntimeError("Transaction is already closed")
        if len(point) != 3 or any(type(n) is not int for n in point):
            raise ValueError("Block coordinates must be three integers")
        if point not in self.changes:
            if self.session.world.get_block(*point) == block:
                return
            self.session._check_size(len(self.changes) + 1)
        self.changes[point] = block

    def commit(self):
        if self._closed:
            raise RuntimeError("Transaction is already closed")
        count = self.session._commit(list(self.changes.items()))
        self._closed = True
        self.changes.clear()
        return count

    def discard(self):
        self.changes.clear()
        self._closed = True
