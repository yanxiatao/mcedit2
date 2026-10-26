# Original-format fixtures for native Rust regression tests

This directory retains Minecraft world, Region, NBT and schematic sample **data**, not the original Python test suite. The old Python `create_testbed.py` generator was removed from this branch. Native Rust tests must treat every checked-in fixture as **read-only**, verify that inspection does not change source bytes, and keep any write tests on independently created temporary copies.

- `AnvilWorld/` and `AnvilWorld_1.8/` exercise original Java `level.dat` and `.mca` Region discovery, including negative source Region coordinates.
- `*.schematic`, `*.nbt`, `indev.mclevel`, `PocketWorld.zip` and other legacy files remain as compatibility corpus inputs, not as evidence that their formats are already supported.

The Rust native world tests live in `../rust/crates/mcedit2-world/tests/`. Never upload users' actual save archives into this public tree.
