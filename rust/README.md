# MCEdit2 — full Rust + iced rewrite

This independent rewrite starts on branch rust-iced-rewrite, based on master.
It is NOT built from, or committed into, the Python 3 migration branch.
The Python 2 original and Python 3/Qt6 migration are preserved for
differential testing; neither is a runtime dependency of this workspace.

## Supported Minecraft formats and CLI

See [COMPATIBILITY.md](COMPATIBILITY.md) for the exact old-version,
release, snapshot and original-MCEdit2 command-line compatibility
contract, including known not-yet-implemented behaviors.
Format readers currently provide READ-ONLY decoding, never writes.

## Architecture

- mcedit2-world: native Rust, read-only Java level.dat and Anvil/MCRegion
  region header parser, bounded decompression for original chunk NBT.
- mcedit2-cli: headless audit of extracted Java worlds, for fixtures and
  performance baselines.
- mcedit2-iced: standalone iced 0.14 desktop frontend, no Python or Qt.

## Original MCEdit2 startup flags

Both iced GUI binary `mcedit2` and headless `mcedit2-cli` now
recognize -debug, -log, -v, -vv, -resetPrefs, -view,
-h/--help, -eval CODE and multiple world paths from the original source.
World directories, level.dat and level.dat_old are accepted by the world
reader. The old Python -eval semantics CANNOT run in an all-Rust program:
the option is recognized but explicitly fails with a useful error.
There is no claim of complete behavior compatibility until saving and
legacy schematic imports are implemented. See COMPATIBILITY.md.

## Run

Install the stable Rust toolchain and operating-system GPU dependencies,
then from this directory:

    cargo test --workspace
    cargo run -p mcedit2-cli -- "/path/to/extracted/world"
    cargo run -p mcedit2-iced -- -view "/path/to/world"
    cargo run -p mcedit2-iced

In the iced window paste a world directory path and choose Open world.
A Minecraft .zip must first be extracted into a temporary directory.

## Current functionality and safety

The editor indexes worlds and reads original block identity at XYZ
coordinates with one common Rust API for Alpha, MCRegion, numeric and
named Anvil, and current release/snapshot Java Section formats.
`level.dat`, original region headers and targeted chunks are never
modified by inspection or staging. Missing, truncated, malformed or
unsupported chunks are refused rather than guessed. DataVersion is
shown when present, not inferred when absent.

The iced desktop now has a **safe in-memory block editing overlay**:
- Enter a rectangle as `X1,Y1,Z1; X2,Y2,Z2` (up to 4096 cells per batch).
- Enter an explicit target state, e.g. `minecraft:oak_log[axis=y]`.
- For an old numeric world, use `numeric:301[data=7]`; unknown old
  modded numeric IDs are never guessed or silently converted.
- Optionally specify the existing effective state to match; blank matches all.
- Preview first, then stage as ONE transaction; Undo, Redo and Discard
  operate only in memory. Original chunks are decoded once per source
  chunk rather than once per block.
- If an original chunk or Section is absent, or the world changes after
  preview, the complete operation is rejected with no partial edits.
- Multiple open worlds and dimensions maintain separate edit overlays.
- Original `-view` prohibits staging even temporary changes.

**No edits are currently saved to disk.** Closing the application discards
the overlay; it does not modify original Minecraft worlds or modded NBT.
Safe on-disk block persistence, 2D/3D rendering, entity editing, Bedrock
LevelDB and full world conversion are still future milestones.

The intended writer will retain original unknown NBT fields using
original-byte spans and strict transactions. Do not roundtrip whole
modded chunks through a lossy generic serializer.

## Migration milestones

1. Native Rust workspace, world/region inspection, iced desktop shell.
2. Original-byte NBT/Sections/Palette decoding with versioned adapters.
3. Block edit transactions and Undo/Redo with lossless region backups.
4. Iced 2D/3D navigation, GPU mesh jobs and selection tools.
5. Native BE/entity safe edits, structure formats, plugins, validation.

User requirement: organize work as five SUBSTANTIAL feature commits then
one complete CI batch, not five trivial commits. This first milestone
has not been compiler-tested in the execution environment; Rust toolchain
is not installed here. No CI results should be asserted before testing.


## Native block inspector (current milestone)

Open the iced desktop frontend, select an indexed world/dimension, enter
X, Y, Z (including negative Y) and choose Inspect exact block.
The original raw Section is decoded without rewriting the world.
Unknown modded numeric IDs stay numeric:id with their data metadata.
The inspected world can contain Java release or snapshot DataVersions
(for example 26.3 5023 and 26.4 Snapshot 5 5125).
This is read-only inspection, NOT release/snapshot write support.

Rust CI workflow in .github/workflows/rust-iced.yml builds the world
crate, original CLI compatibility parser, iced GUI, and tests the
original checked-in 1.8 Anvil world.


## New Rust batch — substantial feature 1/5

The above world-independent editing transaction layer, iced staging UI,
legacy numeric and modern snapshot tests, and documentation are one
substantial feature commit. The Rust branch's previous five-feature
batch already passed its two-job GitHub Actions build. This NEW batch
will defer CI until five substantial new commits; current staged
editing source must not be marked compiler-tested yet.

## New Rust batch — substantial feature 2/5: original Block Entity guards

The native reader now parses original 1.18+ `block_entities` and legacy
`Level.TileEntities` coordinate lists without rewriting any NBT. Staged
replacements refuse original BE coordinates; edits to unrelated blocks in
the same chunk are allowed. Malformed, duplicate or cross-chunk BE positions
fail closed. A Block Entity added to an original chunk after Preview is
rechecked during Commit, so the entire pending group is rejected before
any overlay state changes. The tests include opaque modded NBT, adjacent
edits, late BE insertion and atomic refusal. This safety guard is necessary
but not sufficient for a production on-disk writer; still **memory only**.

## New Rust batch — substantial feature 3/5: block-only 3D Clone Tool

The iced editor now offers a **block-only clone preview** with an explicit
source XYZ selection and destination minimum coordinate. The Rust world core
freezes the source's effective block states once (including prior unsaved
edits), so overlapping source and target ranges do not cascade. It validates
legacy numeric vs modern named block identities, chunk/Section presence,
source/destination Block Entities, world coordinates and a 4096-cell budget.
Source blocks and staging are rechecked at Commit. Each clone is a single
reversible transaction; failed preflights never stage a partial clone.
No entities or Block Entities are copied, nor are world files written.

## New Rust batch — substantial feature 4/5: 2D horizontal slice inspector

The iced desktop now offers a read-only X/Z slice of up to 64x64 blocks
at one signed Y. Native reads are batched by original chunk and show the
current unsaved edit overlay without writing to disk. A per-slice legend
retains exact numeric and named/modded block identities. Missing original
chunks, absent Sections and original Block Entity positions are distinct.
This is an intermediate inspectable view, not a textured GPU slice or full
MCEdit2 2D/3D editor. Synthetic tests cover missing data, modern worlds,
old numeric modded IDs, overlay display and opaque NBT safety.

## New Rust batch — substantial feature 5/5: bounded connected 3D fill

The new iced **connected 3D fill** preview accepts a selected 3D volume,
seed coordinate and target state. A 6-neighbor graph traverses only the
connected component matching the seed's effective (possibly staged) state;
disconnected matching blocks remain unchanged. It preloads and preflights
original chunks/Sections and rejects Block Entity or unsupported numeric/
named transitions with no partial edits. A successful fill is one normal
Undo/Redo group and never writes disk. The five-substantive-feature Rust
batch can now run the complete GitHub Actions workspace test/build.

## Windows native Rust build validation

The Linux CI batch for commit `74100c1` passed 40/40 world/CLI tests
and a full iced workspace check. The separate Windows MSVC job now checks
the full workspace, runs native world and CLI regression tests, and builds
an **experimental** `mcedit2.exe` artifact. This is not a completed
MCEdit2 binary release: the GUI remains memory-only, and Windows desktop
runtime and actual Minecraft save writes must be validated separately.

## Native original-NBT snapshot API (unbuilt source milestone)

The backend now exposes read_world_chunk_bytes alongside strict
read_world_chunk. It returns the original decompressed chunk NBT without
reserialization, covering Alpha chunk files and MCA/MCR regions. This
provides exact original-byte fingerprints for future portable edit journals
and lossless transactional writers. Block edit sessions still NEVER write
original region or Alpha files. This source update is intentionally not
built while automatic Rust CI is disabled.

## Portable edit journals (unbuilt source)

Rust Session now exports/imports external `.mcedit2patch` files using
whole-original-chunk SHA-256 NBT fingerprints, strict ID and Block Entity
preflight, and one-step Undo of the recovered overlay. The journal writes
outside the source world using create-new semantics, never writing regions,
level.dat or any unknown/modded NBT. Only matching world paths, dimensions,
DataVersions and untouched original chunk bytes can be imported. This is
not a native-world save function, and source-only changes are not compiler-
verified while GitHub Actions remains manual-only.

### Rust iced patch journal controls (unbuilt)

The desktop now exposes an explicit external file path plus Export edit
journal / Import verified journal buttons. Import is disabled by -view,
rejects nonempty sessions, checks the original entire NBT SHA-256 and
restores the block overlay with one Undo. New regression sources cover
modern and numeric worlds, unchanged original bytes, foreign dimensions,
duplicate entries, altered opaque NBT, and overwrite/world-directory
refusal. No build was requested or initiated after disabling push CI.

## First lossless named-palette NBT byte patch candidate (unbuilt)

New patch_named_existing_palette edits ONLY the existing packed LongArray
bytes for original modern/1.13+ named Anvil chunks, leaving all other NBT
bytes and unknown modded tags in place. It strictly requires each target
state to already exist in the exact Section palette. It validates source
states, original Block Entity coordinates, duplicate targets, packing,
unchanged NBT length and a successful decode of the resulting byte buffer.
This is an in-memory algorithm, not an MCA sector/backup/atomic writer; it
refuses palette expansion, old numeric arrays and unsupported cases. No
build or Actions run was started for this source-only milestone.

### Rust editor native-byte save eligibility preview (unbuilt)

The iced editor can now inspect whether all staged named-state edits can
be represented by original-NBT palette/packed-data payload replacement.
Palette growth, NBT size changes, and missing single-state packed arrays
are supported as an in-memory candidate, without regenerating other tags. The Rust API returns in-memory per-chunk candidate
buffers plus original SHA-256 checksums, intended for a later backup-aware
writer. Unknown modded tag bytes are not regenerated. Palette growth and
all legacy numeric writes remain unsupported, and no Save-to-world action
exists. CI remains manual-only, no compilation performed for this commit.

## Persistence round 1/5 — named palette growth (not built)

Original-byte NBT surgery now expands legacy 1.13+ and modern Java Section
palettes, including existing one-state sections without a packed data array.
Only the targeted palette List payload and packed LongArray payload are
replaced, or a missing data tag inserted before the original Compound end.
Other Block Entity/entity/modded NBT tag bytes are copied untouched; the
candidate is decoded and *all 4096 blocks in every original Section* are
compared to original values plus intentional replacements. Multi-section
operations are applied in descending byte offset. This is not a region
writer and remains unbuilt by request. Rust branch history was compacted
from 19 commits into this one checkpoint; no automatic CI until five rounds.

Palette bit-width transitions now explicitly distinguish pre-1.16
contiguous packing from 1.16+ padded packing; ambiguous legacy transition
DataVersions refuse a growth plan rather than choosing a layout silently.
