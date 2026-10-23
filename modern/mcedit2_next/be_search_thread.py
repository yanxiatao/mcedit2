"""Qt6 background adapter for read-only indexed native BE world search.

The worker owns its own read-only NativeAnvilWorld and never touches the
main editor's mutable region caches or widgets. Search snapshot is copied on
GUI thread before startup; only queued Qt signals reach the UI thread.
"""
from __future__ import annotations

from threading import Event

from PySide6.QtCore import QThread, Signal

from .be_world_search import search_generated_be
from .native_anvil import NativeAnvilWorld


class BEWorldSearchWorker(QThread):
    progressChanged = Signal(int, int)
    completed = Signal(object)
    failed = Signal(str)

    def __init__(self, path: str, dimension: str, identifier: str,
                 staged: dict, *, include_staged=True, parent=None):
        super().__init__(parent)
        self.folder = str(path)
        self.dimension = str(dimension)
        self.identifier = str(identifier)
        self.include_staged = bool(include_staged)
        self.staged = dict(staged) if include_staged else {}
        self.cancel_event = Event()

    def cancel(self):
        """Thread-safe cooperative cancellation (checked per chunk)."""
        self.cancel_event.set()

    def run(self):
        reader = None
        try:
            reader = NativeAnvilWorld(self.folder, dimension=self.dimension)
            result = search_generated_be(
                reader, self.identifier,
                include_staged=self.include_staged,
                staged_snapshot=self.staged,
                cancel_event=self.cancel_event,
                progress=lambda done, total:
                    self.progressChanged.emit(done, total))
            self.completed.emit(result)
        except Exception as exc:
            self.failed.emit(str(exc))
        finally:
            if reader is not None:
                reader.close()
