"""Bounded asynchronous greedy mesh construction.

Only immutable blockstate snapshots and tile index maps cross worker threads.
Qt images and GPU buffer uploads remain on the UI/OpenGL thread.
"""
from __future__ import annotations
from array import array
from concurrent.futures import ThreadPoolExecutor

from .greedymesh import greedy_section_geometry


class MeshBuildPool:
    def __init__(self, workers=2, max_pending=4):
        self.executor = ThreadPoolExecutor(max_workers=workers,
                                           thread_name_prefix="mcedit-mesh")
        self.max_pending = max_pending
        self.pending = {}
        self.generation = 0
        self.closed = False
        self.errors = {}

    @staticmethod
    def _build(states, neighbors, tiles, model_faces=None,
               translucent_states=(), layered=False, fluids=False):
        data = greedy_section_geometry(
            states, neighbors, tiles, model_faces=model_faces,
            split_layers=layered, translucent_states=translucent_states,
            fluid_geometry=fluids)
        if layered:
            opaque, blended = data
            return array("f", opaque).tobytes(), array("f", blended).tobytes()
        return array("f", data).tobytes()

    def submit(self, key, states, neighbors, tiles, model_faces=None,
               *, translucent_states=(), layered=False, fluids=False):
        if self.closed or key in self.pending or len(self.pending) >= self.max_pending:
            return False
        epoch = self.generation
        future = self.executor.submit(
            self._build, states, neighbors, tiles, model_faces,
            translucent_states, layered, fluids)
        self.pending[key] = (epoch, future)
        return True

    def poll(self, limit=3):
        result = []
        for key, (epoch, future) in tuple(self.pending.items()):
            if len(result) >= limit:
                break
            if not future.done():
                continue
            self.pending.pop(key, None)
            try:
                data = future.result()
            except Exception as exc:
                self.errors[key] = f"{type(exc).__name__}: {exc}"
                result.append((key, None))
                continue
            if epoch == self.generation:
                result.append((key, data))
        return result

    def forget(self):
        self.generation += 1
        for epoch, future in self.pending.values():
            future.cancel()
        self.pending.clear()
        self.errors.clear()

    def close(self):
        self.closed = True
        self.forget()
        self.executor.shutdown(wait=True, cancel_futures=True)
