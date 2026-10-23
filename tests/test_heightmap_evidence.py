"""Version-bound explicit-state heightmap evidence: parsing and no-write tests."""
import json
from pathlib import Path
import tempfile
import unittest

from mcedit2_next import palette
from mcedit2_next.heightmap_codec import HeightmapError, read_heightmap
from mcedit2_next.heightmap_evidence import (
    HeightmapEvidence, parse_heightmap_evidence, read_heightmap_evidence,
)
from mcedit2_next.heightmap_predicates import classify_heightmap_state
from mcedit2_next.heightmap_collision import preview_collision_heightmap
from mcedit2_next.native_anvil import NativeAnvilWorld
from test_heightmap_codec import make_heightmap_chunk
from test_heightmap_collision import with_ocean_floor
from test_native_anvil import region
from test_selection_native import level_dat

STAIRS = "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=true]"
LEAVES = "minecraft:oak_leaves[distance=1,persistent=false,waterlogged=false]"


def snapshot(version=3955, overrides=None):
    states = {
        "minecraft:air": {"blocks_motion_in_heightmap": False, "has_fluid": False},
        "minecraft:stone": {"blocks_motion_in_heightmap": True, "has_fluid": False},
        "minecraft:water[level=0]": {"blocks_motion_in_heightmap": False, "has_fluid": True},
        STAIRS: {"blocks_motion_in_heightmap": True, "has_fluid": True},
        LEAVES: {"blocks_motion_in_heightmap": True, "has_fluid": False},
    }
    states.update(overrides or {})
    return json.dumps({
        "schema": 1, "data_version": version,
        "source": "Synthetic registry facts for unit testing ONLY",
        "states": states,
    }, sort_keys=True).encode("utf-8")


class HeightmapEvidenceTests(unittest.TestCase):
    def test_exact_state_facts_enable_previously_refused_complex_states(self):
        evidence = parse_heightmap_evidence(snapshot())
        self.assertEqual(evidence.total_states, 5)
        self.assertEqual(len(evidence.digest), 64)
        for state, motion, ocean in (
            (STAIRS, True, True),
            (LEAVES, True, True),
            ("minecraft:water[level=0]", True, False),
            ("minecraft:air", False, False),
        ):
            with self.subTest(state=state):
                block = palette.make_block(state)
                if state in (STAIRS, LEAVES):
                    with self.assertRaises(HeightmapError):
                        classify_heightmap_state(
                            block, "MOTION_BLOCKING", 3955)
                self.assertEqual(
                    classify_heightmap_state(
                        block, "MOTION_BLOCKING", 3955, evidence=evidence),
                    motion)
                self.assertEqual(
                    classify_heightmap_state(
                        block, "OCEAN_FLOOR", 3955, evidence=evidence),
                    ocean)
        with self.assertRaisesRegex(HeightmapError, "No explicit"):
            classify_heightmap_state(
                palette.make_block("minecraft:oak_slab[type=bottom,waterlogged=false]"),
                "OCEAN_FLOOR", 3955, evidence=evidence)
        with self.assertRaisesRegex(HeightmapError, "DataVersion"):
            classify_heightmap_state(
                palette.make_block(STAIRS), "OCEAN_FLOOR", 3953,
                evidence=evidence)
        with self.assertRaises(TypeError):
            evidence.states["minecraft:air"] = evidence.states["minecraft:stone"]

    def test_complex_waterlogged_stair_source_and_tall_water_edit(self):
        evidence = parse_heightmap_evidence(snapshot())
        raw, (x, y, z), layout = make_heightmap_chunk(3955, state=STAIRS)
        raw = with_ocean_floor(raw, layout)
        before = read_heightmap(raw, "OCEAN_FLOOR", layout)
        edit = {(x, y + 1, z): palette.make_block("minecraft:water[level=0]")}
        with self.assertRaises(HeightmapError):
            preview_collision_heightmap(
                raw, edit, layout, name="MOTION_BLOCKING",
                vanilla_tags_confirmed=True)
        motion = preview_collision_heightmap(
            raw, edit, layout, name="MOTION_BLOCKING",
            vanilla_tags_confirmed=True, evidence=evidence)
        ocean = preview_collision_heightmap(
            raw, edit, layout, name="OCEAN_FLOOR",
            vanilla_tags_confirmed=True, evidence=evidence)
        self.assertEqual(motion.columns[0].computed_after, 4)
        self.assertEqual(ocean.columns[0].computed_after, 3)
        self.assertEqual(read_heightmap(ocean.candidate_nbt,
                                         "OCEAN_FLOOR", layout), before)
        self.assertEqual(ocean.candidate_nbt, raw)
        self.assertIn(evidence.digest[:16], motion.format_report())
        self.assertIn("User-supplied", motion.format_report())

    def test_opaque_modded_and_unlisted_state_do_not_fall_back(self):
        evidence = parse_heightmap_evidence(snapshot())
        for state in ("minecraft:glass", "minecraft:oak_slab[type=top,waterlogged=true]",
                      "modpack:machine"):
            with self.subTest(state=state):
                with self.assertRaisesRegex(HeightmapError, "No explicit"):
                    evidence.classify(
                        palette.make_block(state), "MOTION_BLOCKING", 3955)

    def test_modded_exact_state_is_allowed_only_with_explicit_facts(self):
        mod_state = "modpack:machine[facing=north,waterlogged=true]"
        with self.assertRaises(HeightmapError):
            classify_heightmap_state(
                palette.make_block(mod_state), "MOTION_BLOCKING", 3955)
        evidence = parse_heightmap_evidence(snapshot(overrides={
            mod_state: {"blocks_motion_in_heightmap": False,
                        "has_fluid": True}
        }))
        self.assertTrue(classify_heightmap_state(
            palette.make_block(mod_state), "MOTION_BLOCKING", 3955,
            evidence=evidence))
        self.assertFalse(classify_heightmap_state(
            palette.make_block(mod_state), "OCEAN_FLOOR", 3955,
            evidence=evidence))
        with self.assertRaises(HeightmapError):
            classify_heightmap_state(
                palette.make_block("modpack:machine[facing=south,waterlogged=true]"),
                "MOTION_BLOCKING", 3955, evidence=evidence)

    def test_registry_evidence_can_target_an_explicit_later_java_version(self):
        version = 4080
        evidence = parse_heightmap_evidence(snapshot(version))
        raw, (x, y, z), layout = make_heightmap_chunk(
            version, state="minecraft:stone")
        raw = with_ocean_floor(raw, layout)
        changed = {(x, y + 1, z): palette.make_block("minecraft:water[level=0]")}
        motion = preview_collision_heightmap(
            raw, changed, layout, name="MOTION_BLOCKING",
            vanilla_tags_confirmed=True, evidence=evidence)
        self.assertEqual(motion.columns[0].computed_after, 4)
        with self.assertRaisesRegex(HeightmapError, "1.21/1.21.1"):
            preview_collision_heightmap(
                raw, changed, layout, name="MOTION_BLOCKING",
                vanilla_tags_confirmed=True)

    def test_reject_duplicate_keys_noncanonical_state_invalid_schema(self):
        for document in (
            b'{"schema":1,"schema":1}',
            snapshot().replace(b'"schema": 1', b'"schema": true'),
            snapshot().replace(b'"data_version": 3955', b'"data_version": true'),
            snapshot().replace(b'"blocks_motion_in_heightmap": true',
                               b'"blocks_motion_in_heightmap": 1'),
            snapshot(overrides={"minecraft:Stone": {
                "blocks_motion_in_heightmap": True, "has_fluid": False}}),
            snapshot(overrides={"MODPACK:machine": {
                "blocks_motion_in_heightmap": True, "has_fluid": False}}),
            snapshot(overrides={"minecraft:oak_stairs[waterlogged=true,facing=north]": {
                "blocks_motion_in_heightmap": True, "has_fluid": True}}),
            b"\xff\xfe",
            b"[]",
        ):
            with self.subTest(payload=document[:90]):
                with self.assertRaises(HeightmapError):
                    parse_heightmap_evidence(document)
        with self.assertRaises(HeightmapError):
            parse_heightmap_evidence(b" " * (32 * 1024 * 1024 + 1))

    def test_explicit_file_import_and_backend_never_create_approval(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            evidence_file = folder / "evidence.json"
            evidence_file.write_bytes(snapshot())
            evidence = read_heightmap_evidence(evidence_file)
            self.assertIsInstance(evidence, HeightmapEvidence)
            (folder / "world/region").mkdir(parents=True)
            world_dir = folder / "world"
            raw, (x, y, z), layout = make_heightmap_chunk(3955, state=STAIRS)
            raw = with_ocean_floor(raw, layout)
            region_file = world_dir / "region/r.0.0.mca"
            region_file.write_bytes(region({(0, 0): raw}))
            (world_dir / "level.dat").write_bytes(level_dat(3955))
            original_file = region_file.read_bytes()
            world = NativeAnvilWorld(str(world_dir))
            world.set_block(x, y + 1, z, palette.make_block("minecraft:water[level=0]"))
            preview = world.preview_collision_heightmap(
                0, 0, name="MOTION_BLOCKING", min_y=-64, height=384,
                vanilla_tags_confirmed=True, evidence=evidence)
            self.assertEqual(preview.different_columns, 1)
            self.assertEqual(region_file.read_bytes(), original_file)
            self.assertEqual(world.approved_world_surface_chunks, ())
            self.assertEqual(len(world._edits), 1)
            with self.assertRaisesRegex(HeightmapError, "DataVersion"):
                world.preview_collision_heightmap(
                    0, 0, name="MOTION_BLOCKING", min_y=-64, height=384,
                    vanilla_tags_confirmed=True,
                    evidence=parse_heightmap_evidence(snapshot(4080)))
            world.close()

    def test_large_runtime_registry_import_beyond_old_20000_state_limit(self):
        # A typical modded Minecraft runtime can have far more states
        # than the former 20,000-entry ceiling. A complete snapshot must
        # import as one immutable object, not silently truncate.
        states = {
            f"minecraft:test_block_{i}": {
                "blocks_motion_in_heightmap": i % 2 == 0,
                "has_fluid": False,
            }
            for i in range(25001)
        }
        content = json.dumps({
            "schema": 1, "data_version": 3955,
            "source": "Synthetic large-runtime registry import test",
            "states": states,
        }, separators=(",", ":")).encode("utf-8")
        evidence = parse_heightmap_evidence(content)
        self.assertEqual(evidence.total_states, 25001)
        self.assertEqual(evidence.data_version, 3955)
        self.assertTrue(evidence.classify(
            palette.make_block("minecraft:test_block_100"),
            "OCEAN_FLOOR", 3955))
        self.assertFalse(evidence.classify(
            palette.make_block("minecraft:test_block_101"),
            "OCEAN_FLOOR", 3955))

    def test_file_loader_refuses_directory_symlink_and_large_file(self):
        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            with self.assertRaises(HeightmapError):
                read_heightmap_evidence(folder)
            large = folder / "large.json"
            large.write_bytes(b"x" * (32 * 1024 * 1024 + 1))
            with self.assertRaises(HeightmapError):
                read_heightmap_evidence(large)
            valid = folder / "valid.json"
            valid.write_bytes(snapshot())
            alias = folder / "alias.json"
            try:
                alias.symlink_to(valid)
            except (OSError, NotImplementedError):
                return
            with self.assertRaises(HeightmapError):
                read_heightmap_evidence(alias)


if __name__ == "__main__":
    unittest.main()
