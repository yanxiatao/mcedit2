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
class BiomeCellEdit:
    """One original Java 4x4x4 biome quart edit, with undoable names."""
    point: Position
    before: str
    after: str


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
        # All native biome edits in a paint are one atomic staging operation;
        # don't re-pack increasingly large palettes 128 times or leave a
        # partial transaction if a later cell cannot be represented.
        if changes and all(isinstance(c, BiomeCellEdit) for c in changes):
            batch = getattr(self.world, "set_biomes_batch", None)
            if not callable(batch):
                raise ValueError("Native backend lacks atomic biome staging")
            batch([(c.point, c.before if reverse else c.after)
                   for c in changes])
            return

        def apply_one(change, undo):
            if isinstance(change, BiomeCellEdit):
                self.world.set_biome(
                    change.point, change.before if undo else change.after)
                return
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

    def paint_biome_box(self, box: Box, biome: str) -> int:
        """Paint intersecting existing 4x4x4 biome cells in one Undo step.

        Selection is rounded OUTWARD to complete quart cells. The backend
        validates every original target chunk/section and the final name
        before any mutation; this never changes terrain block NBT.
        """
        from .biome_edit import MAX_BIOME_CELLS
        if not isinstance(box, Box):
            raise ValueError("Select a valid XYZ box for biome painting")
        ranges = [range(box.minimum[i] >> 2, (box.maximum[i] >> 2) + 1)
                  for i in range(3)]
        count = len(ranges[0]) * len(ranges[1]) * len(ranges[2])
        if count == 0 or count > MAX_BIOME_CELLS:
            raise ValueError("Biome selection must cover 1..128 quart cells")
        prepared = []
        for qx in ranges[0]:
            for qy in ranges[1]:
                for qz in ranges[2]:
                    point = (qx * 4, qy * 4, qz * 4)
                    before = self.world.get_biome(point)
                    if before != biome:
                        prepared.append(BiomeCellEdit(point, before, biome))
        if not prepared:
            return 0
        preflight = getattr(self.world, "validate_biome_writes", None)
        if not callable(preflight):
            raise ValueError("Biome editing requires a native validating backend")
        preflight([(edit.point, edit.after) for edit in prepared])
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

    def preview_batch_replace(self, box: Box, rules, *,
                              exact_state: bool = False,
                              skip_block_entities: bool = True):
        """Plan simultaneous native replacements without staging any edits."""
        from .batch_replace import plan_batch_replace
        self._check_size(box.volume)
        preview, _ = plan_batch_replace(
            self.world, box, rules, exact_state=exact_state,
            skip_block_entities=skip_block_entities)
        return preview

    def batch_replace(self, box: Box, rules, *,
                      exact_state: bool = False,
                      skip_block_entities: bool = True) -> int:
        """Stage one multi-rule undo; never overwrite original Block Entities.

        All targets derive from the same pre-operation block-state snapshot.
        A change rejected by any native Section/BE/ID preflight aborts the
        entire batch without applying an earlier rule.
        """
        from .batch_replace import plan_batch_replace
        self._check_size(box.volume)
        preview, updates = plan_batch_replace(
            self.world, box, rules, exact_state=exact_state,
            skip_block_entities=skip_block_entities)
        if preview.protected_be and not skip_block_entities:
            raise ValueError(
                f"{preview.protected_be} matching Block Entity locations; "
                "entire replacement refused (enable BE skip to preserve them)")
        if not updates:
            return 0
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

    def _guard_clone_destinations(self, targets, *, only_replace_air: bool):
        """Select native target cells BEFORE writing and protect actual writes.

        Later copy stamps do not make an occupied destination eligible: the
        mask sees the frozen destination state after all stamps are planned
        and before the first original world mutation. Skipped coordinates
        remain entirely untouched, including their opaque Block Entity NBT.
        """
        if type(only_replace_air) is not bool:
            raise ValueError("Clone destination-air mask must be Boolean")
        from .clone_mask import is_vanilla_source_air
        entries = targets.items() if isinstance(targets, dict) else targets
        # A malformed/modded world can contain a real original BE alongside
        # an air BlockState. The target-air mask must NEVER treat that BE as
        # replaceable. Ordinary no-mask clones still refuse BE destinations.
        checker = getattr(self.world, "has_block_entity", None)
        selected = [
            (point, state) for point, state in entries
            if not only_replace_air
            or (is_vanilla_source_air(self.world.get_block(*point))
                and not (callable(checker) and checker(*point)))
        ]
        self._ensure_block_only(point for point, _ in selected)
        return selected

    def clone_box(self, box: Box, offset: Position, *,
                  skip_air: bool = False,
                  only_replace_air: bool = False) -> int:
        """Clone a selection by a fixed offset without changing blockstates.

        The source is fully captured before any modification, including for
        overlapping selections. Entity and BE data are never copied here.
        """
        if type(skip_air) is not bool:
            raise ValueError("Clone source-air mask must be Boolean")
        dx, dy, dz = self._offset3(offset)
        self._check_size(box.volume)
        if not (dx or dy or dz):
            return 0
        source = tuple(box.points())
        # Check EVERY source BE, even if its block state looks like air.
        self._ensure_block_only(source)
        states = tuple(self.world.get_block(*point) for point in source)
        from .clone_mask import is_vanilla_source_air
        targets = [
            ((x + dx, y + dy, z + dz), state)
            for (x, y, z), state in zip(source, states)
            if not (skip_air and is_vanilla_source_air(state))
        ]
        # Only actual writes require destination BE checks. A protected BE
        # underneath a skipped air source position is left unchanged.
        return self._commit(self._guard_clone_destinations(
            targets, only_replace_air=only_replace_air))

    def clone_box_repeated(self, box: Box, offset: Position,
                           repetitions: int, *,
                           placement: Position = (0, 0, 0),
                           skip_air: bool = False,
                           only_replace_air: bool = False) -> int:
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
        if type(skip_air) is not bool:
            raise ValueError("Clone source-air mask must be Boolean")
        dx, dy, dz = self._offset3(offset)
        px, py, pz = self._offset3(placement)
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
        from .clone_mask import is_vanilla_source_air
        # Frozen mask from the ORIGINAL source, never the previous stamp.
        selected_states = [
            (point, state) for point, state in zip(source, original_states)
            if not (skip_air and is_vanilla_source_air(state))
        ]
        destinations: dict[Position, Any] = {}
        for repeat in range(1, repetitions + 1):
            ox, oy, oz = (px + dx * repeat, py + dy * repeat,
                          pz + dz * repeat)
            for (x, y, z), state in selected_states:
                # Keys are unique; later stamps take precedence. A target
                # may overlap the source, but the snapshot cannot cascade.
                destinations[(x + ox, y + oy, z + oz)] = state
        return self._commit(self._guard_clone_destinations(
            destinations, only_replace_air=only_replace_air))

    def clone_box_with_nbt(self, box: Box, offset: Position, *,
                           repetitions: int = 1,
                           skip_air: bool = False,
                           only_replace_air: bool = False) -> int:
        """Clone one frozen native cell/opaque-BE snapshot with XYZ tiling.

        Every possible write is checked before modifying any staging state.
        Only root TAG_Int BE x/y/z may change. No entity cloning, orientation
        mapping, UUID regeneration, machine-link repair or DataFixer conversion.
        A copy must never overwrite an existing BE, even inside its source.
        """
        from .be_nbt import relocate_payload
        from .clone_mask import is_vanilla_source_air
        from .palette import BlockState

        if (not isinstance(box, Box)
                or len(box.minimum) != 3 or len(box.maximum) != 3
                or any(type(v) is not int for v in box.minimum + box.maximum)
                or any(box.minimum[i] > box.maximum[i] for i in range(3))):
            raise ValueError("BE clone requires a valid bounded XYZ selection")
        if type(skip_air) is not bool or type(only_replace_air) is not bool:
            raise ValueError("BE clone air-mask settings must be Boolean")
        step = self._offset3(offset)
        if not any(step):
            raise ValueError("BE clone requires a nonzero XYZ step")
        if type(repetitions) is not int or not 1 <= repetitions <= 32:
            raise ValueError("BE clone repetitions must be an integer in 1..32")
        if (getattr(self.world, "is_mcregion", True) or
                (getattr(self.world, "legacy", True) and
                 not getattr(self.world, "lossless_legacy_be_transfer", False))):
            raise ValueError("BE clone requires named Java Anvil or validated numeric Anvil")
        # Limit planned work BEFORE loading or producing potentially duplicated
        # target cells; a highly overlapping tiling cannot bypass the limit.
        self._check_size(box.volume * repetitions)
        get_cell = getattr(self.world, "get_cell", None)
        checker = getattr(self.world, "has_block_entity", None)
        section = getattr(self.world, "_section", None)
        maker = getattr(self.world, "make_block", None)
        if not all(callable(x) for x in (get_cell, checker, section, maker)):
            raise ValueError("Native BE-safe snapshot/section backend required")

        source = []
        source_be_count = 0
        source_be_bytes = 0
        for point in box.points():
            if (not -30_000_000 <= point[0] <= 30_000_000
                    or not -2048 <= point[1] < 4096
                    or not -30_000_000 <= point[2] <= 30_000_000):
                raise ValueError("Source coordinate exceeds native Java world bounds")
            section(point[0] >> 4, point[2] >> 4, point[1] >> 4)
            snap = get_cell(*point)
            if not isinstance(snap, CellSnapshot) or not isinstance(snap.block, BlockState):
                raise ValueError(f"Original source snapshot incomplete at {point}")
            if snap.block != maker(str(snap.block)):
                raise ValueError("Source native BlockState cannot roundtrip")
            # BE must be validated even if a custom/modded source state
            # resembles air. A real BE is never skipped as source air.
            if snap.block_entity is not None and is_vanilla_source_air(snap.block):
                raise ValueError(f"Original Block Entity exists on air at {point}")
            if snap.block_entity is not None and getattr(self.world, "legacy", False):
                from .legacy_anvil import id_meta
                if id_meta(snap.block)[0] == 0:
                    raise ValueError(f"Original legacy BE uses numeric air ID at {point}")
            if skip_air and snap.block_entity is None and is_vanilla_source_air(snap.block):
                continue
            if snap.block_entity is not None:
                source_be_count += 1
                source_be_bytes += len(snap.block_entity)
                if (source_be_count * repetitions > 512 or
                        source_be_bytes * repetitions > 8 * 1024 * 1024):
                    raise ValueError(
                        "BE clone exceeds 512 BE records or 8 MiB opaque "
                        "NBT payload across the requested copies")
            source.append((point, snap))

        targets = {}
        for repeat in range(1, repetitions + 1):
            displacement = tuple(value * repeat for value in step)
            for source_point, snap in source:
                destination = tuple(source_point[i] + displacement[i]
                                    for i in range(3))
                if (not -30_000_000 <= destination[0] <= 30_000_000
                        or not -2048 <= destination[1] < 4096
                        or not -30_000_000 <= destination[2] <= 30_000_000):
                    raise ValueError("BE clone destination outside native Java bounds")
                # Later stamps win; both states and opaque BEs come from the
                # SAME frozen source cells, never intermediate destination.
                targets[destination] = (source_point, snap)
        self._check_size(len(targets))
        planned = []
        for destination, (source_point, snap) in sorted(targets.items()):
            # Target sections must already exist; no missing-chunk or absent
            # section synthesis. Inspect target from its pre-edit state.
            section(destination[0] >> 4, destination[2] >> 4,
                    destination[1] >> 4)
            before = get_cell(*destination)
            if not isinstance(before, CellSnapshot):
                raise ValueError(f"Invalid native target snapshot at {destination}")
            # Optional air mask checks the frozen original target, never an
            # earlier stamp. Skipped occupied cells are not touched.
            if only_replace_air and not is_vanilla_source_air(before.block):
                continue
            if checker(*destination) or before.block_entity is not None:
                raise ValueError(
                    f"Target {destination} has original/staged BE data; "
                    "native BE cloning cannot overwrite existing Block Entities")
            payload = snap.block_entity
            if payload is not None:
                payload = relocate_payload(payload, source_point, destination)
            after = CellSnapshot(snap.block, payload)
            if before != after:
                planned.append((destination, before, after))
        if not planned:
            return 0
        # A single native edit with normal rollback and one Undo/Redo entry.
        self._apply(planned, reverse=False)
        self.undo_stack.append(planned)
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return len(planned)

    def clone_box_transformed(self, box: Box, offset: Position,
                              operation: str, repetitions: int = 1, *,
                              placement: Position = (0, 0, 0),
                              skip_air: bool = False,
                              only_replace_air: bool = False) -> int:
        """Clone rotated/mirrored native block states without changing source.

        The transform is evaluated against ONE snapshot of the original
        selection; all stamps are preflighted together. Unknown modded
        orientation and legacy numeric-ID semantics fail closed. Existing
        BE/entity NBT are deliberately NOT part of this cloning mode.
        """
        from .transforms import (
            MOVES, transform_position, transform_state, transformed_size,
        )
        if type(skip_air) is not bool:
            raise ValueError("Transformed clone source-air mask must be Boolean")
        dx, dy, dz = self._offset3(offset)
        px, py, pz = self._offset3(placement)
        if operation not in MOVES:
            raise ValueError("Unsupported clone rotation/mirror operation")
        if type(repetitions) is not int or not 1 <= repetitions <= 128:
            raise ValueError("Transformed clone repeat count must be 1..128")
        if not (dx or dy or dz):
            raise ValueError("Transformed clone requires a nonzero XYZ step")
        # Check planned operations BEFORE reading the source or destinations.
        self._check_size(box.volume * repetitions)
        source = tuple(box.points())
        self._ensure_block_only(source)
        legacy = bool(getattr(self.world, "legacy", False))
        from .clone_mask import is_vanilla_source_air
        transformed = []
        for point in source:
            original = self.world.get_block(*point)
            if skip_air and is_vanilla_source_air(original):
                continue
            relative = tuple(point[axis] - box.minimum[axis] for axis in range(3))
            relative_dst = transform_position(relative, box.size, operation)
            state = transform_state(original, operation, legacy=legacy)
            transformed.append((relative_dst, state))
        size = transformed_size(box.size, operation)
        if any(n <= 0 for n in size):
            raise ValueError("Invalid transformed selection dimensions")
        destinations: dict[Position, Any] = {}
        for repeat in range(1, repetitions + 1):
            origin = (
                box.minimum[0] + px + dx * repeat,
                box.minimum[1] + py + dy * repeat,
                box.minimum[2] + pz + dz * repeat,
            )
            for (rx, ry, rz), state in transformed:
                # Later stamps override earlier ones on overlaps.
                position = (origin[0] + rx, origin[1] + ry, origin[2] + rz)
                destinations[position] = state
        # Refuse any original protected BE in source or any target,
        # even when the changed state otherwise equals the original.
        # _commit preflights all changed target coordinates before staging,
        # and keeps one before/after entry for every unique target point.
        return self._commit(self._guard_clone_destinations(
            destinations, only_replace_air=only_replace_air))

    def clone_box_scaled(self, box: Box, offset: Position,
                         scale: tuple[int, int, int],
                         repetitions: int = 1, *,
                         placement: Position = (0, 0, 0),
                         skip_air: bool = False,
                         only_replace_air: bool = False) -> int:
        """Nearest-neighbour integer-scale clone from one original snapshot.

        This is a conservative block-only operation: no BE/entity payloads
        are scaled, relocated, invented or regenerated. Original native
        block states are replicated *unchanged*, so even opaque modded
        states may be copied as long as the native destination accepts
        them. No orientation or registry semantics are inferred.

        All source/destination BE positions and every writable native
        target are preflighted before any mutation; Undo is one grouped
        operation with each destination recorded exactly once.
        """
        if type(skip_air) is not bool:
            raise ValueError("Scaled clone source-air mask must be Boolean")
        from .clone_mask import is_vanilla_source_air
        from .clone_scaling import plan_scaled_clone
        plan = plan_scaled_clone(
            box, offset, scale, repetitions, self.max_blocks,
            placement=placement)
        source = tuple(box.points())
        self._ensure_block_only(source)
        originals = tuple(self.world.get_block(*point) for point in source)
        sx, sy, sz = plan.scale
        dx, dy, dz = plan.step
        px, py, pz = plan.placement
        targets: dict[Position, Any] = {}
        # Every stamp uses the SAME pre-mutation original snapshot.
        for stamp in range(1, plan.repetitions + 1):
            base_x = box.minimum[0] + px + stamp * dx
            base_y = box.minimum[1] + py + stamp * dy
            base_z = box.minimum[2] + pz + stamp * dz
            for (x, y, z), native in zip(source, originals):
                if skip_air and is_vanilla_source_air(native):
                    continue
                tx = base_x + (x - box.minimum[0]) * sx
                ty = base_y + (y - box.minimum[1]) * sy
                tz = base_z + (z - box.minimum[2]) * sz
                for ax in range(sx):
                    for ay in range(sy):
                        for az in range(sz):
                            # Later stamps win on overlap. Duplicate target
                            # coordinates cannot corrupt the Undo baseline.
                            targets[(tx + ax, ty + ay, tz + az)] = native
        # _commit validates every changed native target before staging
        # and stores a single before/after pair for each unique coordinate.
        return self._commit(self._guard_clone_destinations(
            targets, only_replace_air=only_replace_air))

    def clone_box_oriented_scaled(self, box: Box, offset: Position,
                                  operation: str, scale: tuple[int, int, int],
                                  repetitions: int = 1, *,
                                  placement: Position = (0, 0, 0),
                                  skip_air: bool = False,
                                  only_replace_air: bool = False) -> int:
        """Right-angle rotate/mirror FIRST, then scale native block states.

        Each source cell is decoded and transformed once before stamping.
        The scale axes refer to the rotated footprint, not original axes.
        The source remains unchanged except for intentional overlap by
        destinations. No BE or entity payload is copied/transformed.
        """
        if type(skip_air) is not bool:
            raise ValueError("Oriented scaled source-air mask must be Boolean")
        from .clone_mask import is_vanilla_source_air
        from .clone_scaling import plan_oriented_scaled_clone
        from .transforms import transform_position, transform_state
        plan = plan_oriented_scaled_clone(
            box, offset, operation, scale, repetitions, self.max_blocks,
            placement=placement)
        original_points = tuple(box.points())
        self._ensure_block_only(original_points)
        legacy = bool(getattr(self.world, "legacy", False))
        samples = []
        for point in original_points:
            original = self.world.get_block(*point)
            if skip_air and is_vanilla_source_air(original):
                continue
            rel = tuple(point[i] - box.minimum[i] for i in range(3))
            rotated = transform_position(rel, box.size, operation)
            state = transform_state(original, operation, legacy=legacy)
            samples.append((rotated, state))
        sx, sy, sz = plan.scale
        dx, dy, dz = plan.step
        px, py, pz = plan.placement
        targets: dict[Position, Any] = {}
        for stamp in range(1, plan.repetitions + 1):
            origin = (
                box.minimum[0] + px + stamp * dx,
                box.minimum[1] + py + stamp * dy,
                box.minimum[2] + pz + stamp * dz,
            )
            for (rx, ry, rz), state in samples:
                x, y, z = (
                    origin[0] + rx * sx,
                    origin[1] + ry * sy,
                    origin[2] + rz * sz,
                )
                for ix in range(sx):
                    for iy in range(sy):
                        for iz in range(sz):
                            # Last stamp wins; one unique undo baseline.
                            targets[(x + ix, y + iy, z + iz)] = state
        return self._commit(self._guard_clone_destinations(
            targets, only_replace_air=only_replace_air))

    def clone_box_reduced(self, box: Box, offset: Position,
                          divisors: tuple[int, int, int],
                          repetitions: int = 1, *,
                          placement: Position = (0, 0, 0),
                          skip_air: bool = False,
                          only_replace_air: bool = False) -> int:
        """Clone a BE-free original selection with XYZ integer downsampling.

        Each output cell takes the ORIGINAL state at the low corner of
        its sampled source group; trailing partial groups are retained.
        Scan ALL source cells for protected BEs (not only sampled cells).
        Every destination is preflighted before staging one Undo record.
        """
        if type(skip_air) is not bool:
            raise ValueError("Reduced clone source-air mask must be Boolean")
        from .clone_mask import is_vanilla_source_air
        from .clone_scaling import plan_reduced_clone
        plan = plan_reduced_clone(
            box, offset, divisors, repetitions, self.max_blocks,
            placement=placement)
        source = tuple(box.points())
        self._ensure_block_only(source)
        dx, dy, dz = plan.divisors
        sampled = []
        for x, y, z in source:
            rx, ry, rz = (x - box.minimum[0], y - box.minimum[1],
                          z - box.minimum[2])
            if rx % dx == 0 and ry % dy == 0 and rz % dz == 0:
                native = self.world.get_block(x, y, z)
                if skip_air and is_vanilla_source_air(native):
                    continue
                sampled.append(((rx // dx, ry // dy, rz // dz), native))
        if not skip_air and len(sampled) != (
                plan.output_size[0] * plan.output_size[1] *
                plan.output_size[2]):
            raise ValueError("Reduced source snapshot is incomplete")
        targets: dict[Position, Any] = {}
        for stamp in range(1, plan.repetitions + 1):
            origin = tuple(
                box.minimum[i] + plan.placement[i] + stamp * plan.step[i]
                for i in range(3))
            for rel, native in sampled:
                target = tuple(origin[i] + rel[i] for i in range(3))
                targets[target] = native
        return self._commit(self._guard_clone_destinations(
            targets, only_replace_air=only_replace_air))

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

    def move_box_with_nbt(self, box: Box, offset: Position) -> int:
        """Move original BE-bearing blocks in ONE guarded undoable transaction.

        Source cells are snapshotted first, even for overlapping translations.
        Only root BE x/y/z are relocated; every opaque mod field is retained.
        Preexisting BEs outside the source are never overwritten. This does
        not transfer standalone entities, ticks or cross-machine references.
        """
        from .be_nbt import relocate_payload
        from .palette import BlockState

        if (not isinstance(box, Box) or
                any(type(v) is not int for v in box.minimum + box.maximum)
                or any(box.minimum[i] > box.maximum[i] for i in range(3))):
            raise ValueError("BE-aware move requires a valid bounded 3D Box")
        if (getattr(self.world, "is_mcregion", True) or
                (getattr(self.world, "legacy", True) and
                 not getattr(self.world, "lossless_legacy_be_transfer", False))):
            raise ValueError("BE-aware move requires named Java Anvil or validated numeric Anvil")
        offset = self._offset3(offset)
        if not any(offset):
            return 0
        self._check_size(box.volume)
        get_cell = getattr(self.world, "get_cell", None)
        checker = getattr(self.world, "has_block_entity", None)
        section = getattr(self.world, "_section", None)
        make_block = getattr(self.world, "make_block", None)
        if not all(callable(x) for x in (get_cell, checker, section, make_block)):
            raise ValueError("Native BE-safe backend required")
        source = tuple(box.points())
        sources = set(source)
        destinations = tuple(
            tuple(point[i] + offset[i] for i in range(3))
            for point in source)
        affected = set(source)
        affected.update(destinations)
        self._check_size(len(affected))
        for point in affected:
            if not (-30_000_000 <= point[0] <= 30_000_000
                    and -2048 <= point[1] < 4096
                    and -30_000_000 <= point[2] <= 30_000_000):
                raise ValueError("BE-aware move exceeds native world coordinate limits")

        # Preflight every touched source/target before any staged mutations.
        # This also refuses absent original chunks/sections instead of
        # generating world data or using get_block()'s air fallback.
        originals = {}
        for point in sorted(affected):
            section(point[0] >> 4, point[2] >> 4, point[1] >> 4)
            snap = get_cell(*point)
            if not isinstance(snap, CellSnapshot) or not isinstance(snap.block, BlockState):
                raise ValueError(f"Incomplete native BE cell at {point}")
            originals[point] = snap
            if (point in sources and snap.block_entity is not None and
                    getattr(self.world, "legacy", False)):
                from .legacy_anvil import id_meta
                if id_meta(snap.block)[0] == 0:
                    raise ValueError(f"Original legacy BE uses numeric air ID at {point}")
            if point not in sources and checker(*point):
                raise ValueError(
                    f"External destination BE at {point} must not be overwritten")

        air = make_block("minecraft:air")
        if air != make_block(str(air)):
            raise ValueError("Cannot verify native air BlockState")
        targets = {point: CellSnapshot(air, None) for point in source}
        for source_point, destination in zip(source, destinations):
            cell = originals[source_point]
            state = make_block(str(cell.block))
            if state != cell.block:
                raise ValueError("Native block state roundtrip changed identity")
            payload = cell.block_entity
            if payload is not None:
                # validate_payload is called by relocate_payload, with
                # strict original source coordinate and BE identity checks.
                payload = relocate_payload(payload, source_point, destination)
            targets[destination] = CellSnapshot(state, payload)
        # Empty-source and source/destination overlaps are resolved using
        # the frozen originals; no cascading mutations can occur.
        changes = [
            (point, originals[point], targets[point])
            for point in sorted(targets) if originals[point] != targets[point]
        ]
        if not changes:
            return 0
        self._apply(changes, reverse=False)
        self.undo_stack.append(changes)
        self.undo_stack = self.undo_stack[-100:]
        self.redo_stack.clear()
        self.dirty = True
        return len(changes)

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
