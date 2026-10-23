# Exact state heightmap evidence (experimental)

The editor accepts **optional, user-supplied, read-only** JSON evidence
for MOTION_BLOCKING and OCEAN_FLOOR previews. Neither heightmap can be
written by this feature. The built-in conservative Java 1.21/1.21.1
subset still works when no JSON is provided.

## JSON schema 1

Each entry is an **exact native BlockState** including all applicable
properties. The file is bound to one Java DataVersion. Example only:

~~~json
{
  "schema": 1,
  "data_version": 3955,
  "source": "Example only: replace with a real runtime registry export",
  "states": {
    "minecraft:air": {
      "blocks_motion_in_heightmap": false,
      "has_fluid": false
    },
    "minecraft:stone": {
      "blocks_motion_in_heightmap": true,
      "has_fluid": false
    },
    "minecraft:water[level=0]": {
      "blocks_motion_in_heightmap": false,
      "has_fluid": true
    }
  }
}
~~~

The historical JSON property `blocks_motion_in_heightmap` means the **actual runtime predicate** `Heightmap.Types.OCEAN_FLOOR.isOpaque().test(state)` (NOT membership in a Minecraft block tag). The `has_fluid` property means `!state.getFluidState().isEmpty()`. OCEAN_FLOOR uses the first fact; MOTION_BLOCKING uses their logical OR, validated against the actual Minecraft runtime predicate by the optional NeoForge 1.21.1 exporter. The facts must come
from the **same game version, installed mods and active datapacks**
as the target world. The JSON format itself cannot authenticate their
correctness.

To include stairs, slabs, leaves, waterlogged blocks or modded states,
enumerate each relevant exact property combination. A rule for one stair
facing / shape / waterlogged state does not apply to another facing.
Unknown states fail closed even when they share a block name. Do not guess
collision or fluid behavior from the name or copy registry facts between
game versions without checking.

The importer accepts up to 200,000 exact states in a 32 MiB non-symlink
UTF-8 JSON. It enforces canonical names and sorted state properties,
valid schema/types, and rejects duplicate JSON keys. The parsed snapshot
is immutable and reports its source claim and SHA-256 checksum. It never
executes Python, loads a plugin, inspects installed mod JARs, downloads
rules, authenticates their provenance or auto-resolves tags.

## Qt6 usage

Choose File > Preview collision heightmap using state evidence JSON,
select your file, select MOTION_BLOCKING or OCEAN_FLOOR, and supply the
exact dimension minimum Y and total build height. Explicitly acknowledge
the origin and accuracy of the facts. The editor checks the source
DataVersion, every affected column's states, and the original heightmap
values. Missing facts or inconsistencies cause a refusal. The result
is an in-memory candidate and is never written into the world.

This importer does not affect the separately approved experimental
WORLD_SURFACE writer. It does not solve lighting, heightmap collision
semantics outside supplied exact-state facts, or any runtime mod behavior.

## Status

Exporting canonical collision/tag state facts from the running Minecraft
game is not yet automated. A real runtime-derived registry snapshot and
actual game-produced world fixtures are still needed for compatibility
certification. Use disposable offline worlds only.


For an optional game-side runtime exporter source overlay, see
[tools/neoforge-1.21.1-exporter](../tools/neoforge-1.21.1-exporter/README.md).
Its Java source is not yet compiled/verified against NeoForge in CI.
