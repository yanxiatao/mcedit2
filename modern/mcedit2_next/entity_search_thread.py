"""Isolated Qt worker for read-only generated-world original entity search."""
from __future__ import annotations

from threading import Event

from PySide6.QtCore import QThread, Signal

from .entity_world_search import search_generated_entities
from .native_anvil import NativeAnvilWorld


class EntityWorldSearchWorker(QThread):
    progressChanged = Signal(int, int)
    completed = Signal(object)
    failed = Signal(str)

    def __init__(self, folder, dimension, identifier, snapshot, *,
                 include_staged=True, parent=None):
        super().__init__(parent)
        self.folder = str(folder)
        self.dimension = str(dimension)
        self.identifier = str(identifier)
        self.include_staged = bool(include_staged)
        # Deep-copy container topology on the GUI thread. NBT payloads are
        # immutable bytes and need not be copied/normalized.
        self.snapshot = {
            "edits": dict(snapshot.get("edits", {})),
            "deletions": set(snapshot.get("deletions", ())),
            "additions": dict(snapshot.get("additions", {})),
            "transfers": dict(snapshot.get("transfers", {})),
        }
        self.cancel_event = Event()

    def cancel(self):
        self.cancel_event.set()

    def run(self):
        world = None
        try:
            world = NativeAnvilWorld(self.folder, dimension=self.dimension)
            result = search_generated_entities(
                world, self.identifier,
                include_staged=self.include_staged,
                staged_snapshot=self.snapshot,
                cancel_event=self.cancel_event,
                progress=lambda done, total:
                    self.progressChanged.emit(done, total))
            self.completed.emit(result)
        except Exception as exc:
            self.failed.emit(str(exc))
        finally:
            if world is not None:
                world.close()
