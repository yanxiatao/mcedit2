"""Offline protocol tests for game-produced evidence JSON verification.

The actual NeoForge game runtime test is performed separately in the
optional official-MDK GitHub Actions job.
"""
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

VERIFIER = (
    Path(__file__).resolve().parents[1] /
    "tools/neoforge-1.21.1-exporter/verify_runtime_export.py"
)
_spec = importlib.util.spec_from_file_location("runtime_export_verifier", VERIFIER)
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)

SOURCE = (
    "NeoForge 1.21.1 server runtime Heightmap.Types / fluid-state export"
)
STAIRS = (
    "minecraft:oak_stairs[facing=north,half=bottom,"
    "shape=straight,waterlogged=true]"
)


def export_blob(*, version=3955, source=SOURCE, extra=True):
    states = {
        "minecraft:air": {
            "blocks_motion_in_heightmap": False, "has_fluid": False},
        "minecraft:stone": {
            "blocks_motion_in_heightmap": True, "has_fluid": False},
        "minecraft:water[level=0]": {
            "blocks_motion_in_heightmap": False, "has_fluid": True},
    }
    if extra:
        states[STAIRS] = {
            "blocks_motion_in_heightmap": True, "has_fluid": True}
    return json.dumps(
        {"schema": 1, "data_version": version,
         "source": source, "states": states},
        sort_keys=True, separators=(",", ":")).encode("utf-8")


class RuntimeExportVerifierTests(unittest.TestCase):
    def save(self, folder: Path, data: bytes, *,
             filename_digest: str | None = None):
        digest = filename_digest or sha256(data).hexdigest()[:16]
        file = folder / f"mcedit2-states-3955-{digest}.json"
        file.write_bytes(data)
        return file

    def test_valid_exact_state_snapshot_protocol(self):
        with tempfile.TemporaryDirectory() as temp:
            data = export_blob()
            path = self.save(Path(temp), data)
            self.assertEqual(_mod.verify_export(path, minimum_states=4), 4)

    def test_bad_filename_content_digest_and_source_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            path = self.save(folder, export_blob(), filename_digest="0" * 16)
            with self.assertRaisesRegex(ValueError, "hash"):
                _mod.verify_export(path, minimum_states=4)
            path = self.save(folder, export_blob(source="untrusted"))
            with self.assertRaisesRegex(ValueError, "source"):
                _mod.verify_export(path, minimum_states=4)
            path = self.save(folder, export_blob(version=4080))
            with self.assertRaisesRegex(ValueError, "DataVersion"):
                _mod.verify_export(path, minimum_states=4)

    def test_missing_runtime_coverage_is_not_accepted(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            data = export_blob(extra=False)
            path = self.save(folder, data)
            with self.assertRaisesRegex(ValueError, "Missing representative"):
                _mod.verify_export(path, minimum_states=3)
            with self.assertRaisesRegex(ValueError, "incomplete registry"):
                _mod.verify_export(path, minimum_states=5000)


if __name__ == "__main__":
    unittest.main()
