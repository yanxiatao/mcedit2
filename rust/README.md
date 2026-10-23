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

## Persistence round 2/5 — legacy numeric NBT byte edits (not built)

Added an in-memory original-byte editor for MCRegion 8-bit Blocks/Data
and 1.2–1.12.2 Anvil Blocks/Data/optional Add. The 4-bit metadata nibbles
and 12-bit numeric IDs retain their exact original identity. Anvil can
insert a missing Add TAG_Byte_Array when needed; MCRegion refuses >255 IDs.
All Block Entities are guarded, untouched NBT tags are copied verbatim, and
all 4096 original cells in each Section are verified after re-decoding.
The Rust editor preview now dispatches numeric vs named byte patchers.
No Minecraft region file is written; CI remains deferred until round 5.

## Persistence round 3/5 — in-memory append-only Region image planner (unbuilt)

The Rust backend now assembles MCA/MCR candidate images from verified
original-byte chunk edits. It reads the *whole* source region, checks its
location table and overlap-free sector allocation, original compressed NBT
fingerprints, and retains untouched region bytes and original chunk slots
without repacking. Changed chunks are compressed using their original
GZip/Zlib/raw codec and appended in fresh 4 KiB sectors; only the relevant
header offsets and timestamps are changed. No missing chunks are created.
External .mcc streams, overlapping allocations, oversized compressed chunks,
ambiguous files, source drift, invalid source Block Entity metadata and
any change to an untouched original sector are rejected. The assembled
image is decoded again and each changed block counted. An iced dry-run button
shows the number of images, changed chunks and candidate bytes. This step
DOES NOT WRITE Minecraft world files, and has not been built or CI-tested.
Next: round 4 adds disk backups/recovery and fail-closed atomic commit.

## Persistence round 4/5 — opt-in one-Region disk transaction (unbuilt)

The Rust core now offers a **library-only** Session::apply_single_region_save
operation; there is intentionally still NO GUI save button. It rejects edits
spanning multiple MCA/MCR files, because a sequence of per-file renames is
not a global atomic operation. Backups are immutable, created with
create_new, flushed to disk, followed by a same-directory staged candidate
and a recoverable JSON pending marker. The original full Region hash is
checked again immediately before the rename; original bytes are replaced
through a same-filesystem rename, not in-place sector modifications.

A pending transaction must be explicitly confirmed with
region_transaction::finalize_single_region or reverted using
region_transaction::rollback_pending_region. Crash recovery restores EXACT
backup bytes and refuses corrupted backups or region hashes that have
changed independently. Completion and rollback records are preserved,
along with the original backup; neither operation deletes a backup. The
current GUI does not expose actual write, finalize or rollback. Do not
edit live worlds. Cross-region atomicity, process-level locking, complete
filesystem crash durability and GUI recovery remain unfinished. No
compiler/test verification has been performed: this is round 4/5, with
only manual workflow_dispatch available.

Round-4 hardening: the transaction boundary replays every changed voxel
through the allowed original-byte adapter and byte-compares its resulting
NBT to each proposed replacement. This prevents a modified candidate from
rewriting unrelated modded NBT. The pending marker is now fsynced before
the write-stage so interrupted pre-rename work is recoverable. Recovery
archives valid abandoned stage bytes rather than deleting them.

## Persistence round 5/5 — guarded iced save, manual finalization and recovery

The desktop now has a strictly opt-in **single Region** disk-save preview,
requiring the literal confirmation `SAVE`. It recomputes the full in-memory
Region image at confirmation to reject altered edits/source data. The
underlying transaction independently verifies its backup, unchanged original
source SHA-256, all untouched bytes and original Block Entity protections.
The GUI then displays the immutable original backup and pending recovery
record, requiring explicit `FINALIZE` or `ROLLBACK` for the selected
`r.X.Z.mca` / `r.X.Z.mcr`. The latter restores exact original bytes.

Recovery is also accessible after restarting the application when a pending
transaction is present. Every recovery requires naming the specific Region;
`-view` disables all on-disk mutation controls. Any successful write or
recovery invalidates cached edit sessions for the entire world. Other open
sessions with staged edits block potentially conflicting write/recovery.
Alpha chunk files, external sidecars, multiple regions, concurrent Minecraft
processes and cross-file transactional safety remain unsupported. Test only
on disposable copies of worlds until the consolidated CI has passed and
real-world fixtures are checked. The fifth round commit triggers the single
planned cross-platform CI batch; all subsequent errors require remediation.

## CI policy (October 2026)

GitHub Actions is **compile checks and automated tests only** on Linux
and Windows: `cargo check` and `cargo test`. It does not execute `cargo
build`, `cargo run`, create application bundles/installers, or upload Windows
executables. End-user builds and packaging happen only when the user runs
them manually. Any temporary push-based test validation is removed after
the migration regressions are resolved, leaving `workflow_dispatch` only.

## Next migration batch 1/5 — Windows native Region transaction portability

Windows paths may include an extended-length prefix (`\\?\\`) after
canonicalization, while an indexed world retains a regular absolute path.
The writer now refuses symlinks and compares canonical source/candidate
identities instead of their string representations. It also uses the Win32
same-directory replace-existing rename operation because Rust's default
Windows rename does not overwrite an open destination path like POSIX.
The stage must still be synced and the original immutable backup retained.
Both apply and rollback use the same platform-correct file replacement.
Do NOT edit an active Minecraft world: Windows file sharing/locking errors
are intentionally not bypassed. Cross-region atomicity is still not offered.

## Next migration batch 2/5 — clickable 2D native iced viewport

The iced frontend now renders a 16x16 (or smaller) clickable grid using
original Java block slices, original Block Entity markers, and explicit
missing-chunk/Section cells. Clicking a cell displays its full numeric or
named/modded BlockState and safely selects exactly that coordinate for
normal 3D preview/stage/Undo workflows. Arrow/height-layer controls pan
by 16 X/Z blocks or one Y level. Larger original 2D slices retain the
read-only 64x64 text-map inspection without creating thousands of widgets.
Stale grids are invalidated when the world/dimension changes, or after
any in-memory mutation/Undo/Redo, so selections cannot silently use a
previously edited viewport. This is an interactive widget grid, not yet
a textured chunk mesh or full 3D GPU renderer. No builds or CI triggered.

## Next migration batch 3/5 — portable block-only clipboard

The Rust Session now exports up to 4096 frozen effective block states in
Y/Z/X order to a versioned `.mcedit2clip` file and previews a later paste
into a selected destination, staging it as one Undo/Redo transaction.
Original Block Entities at the source OR destination refuse the operation
rather than silently removing machine NBT; source chunks and Sections must
exist, and the file is limited to the SAME original world, dimension and
DataVersion, retaining original numeric-vs-named block identity. Clipboard
files are create-new, stored outside the Minecraft world and strictly
validated on import. iced adds selection export and clipboard paste preview
controls. The clipboard deliberately excludes Block Entities, entities and
schematic conversion; standardized `.schematic`/`.schem` formats remain
separate future adapters. No disk world writes occur during clipboard use.

## Next migration batch 4/5 — classic MCEdit schematic importer

Native Rust can now read original GZip-compressed numeric-ID `.schematic`
Blocks, Data and AddBlocks, preserving metadata and old high-ID nibbles,
with up to 4096 blocks per transaction. Nonempty TileEntities/Entities/
TileTicks are refused rather than silently dropped. The iced editor can
preview a classic structure paste at its clipboard destination and apply
it using the existing guarded/undoable operation. Only original Alpha
Materials numeric schematics and numeric destination chunks are supported;
modern Sponge `.schem` and non-lossy BE structure transfer remain pending.

## Next migration batch 5/5 — Sponge v2 structures

Modern Java named-block Sponge schematic v2 `.schem` import/export is
implemented with GZip NBT, exact DataVersion matching, required palette,
PaletteMax, bounded varint BlockData and canonical Y/Z/X ordering. The
exporter emits a new file outside the original world and never regenerates
unrelated original NBT. Nonempty entities/BlockEntities and all biome
information are rejected on import rather than silently discarded; nonzero
Offset, unknown schematic Versions, malformed indices and structures over
4096 cells are likewise refused. iced adds export and paste-preview buttons
and the existing BE guard and Undo system handles destination edits. Sponge
v3, full BE/entity schematic transfers, and version conversion still remain
unimplemented. This completes the five new migration source rounds; perform
combined Linux/Windows **check and test only** next, not application builds.

## Migration batch 3, round 1/5 — read-only opaque Block Entity inspector

The native core now reports an original modded Block Entity ID, SHA-256
fingerprint, serialized compound byte length, original NBT tag types and
bounded direct-field names/sizes. The iced editor exposes a read-only
inspection button for a specific XYZ coordinate. This DOES NOT yet allow
Block Entity editing, movement or copying: source and destination BE blocks
remain protected during all existing editing operations. Malformed original
NBT fails closed rather than yielding a misleading empty object. No build,
packaging or GitHub Actions run occurs until five feature rounds complete.

## Migration batch 3, round 2/5 — clipboard spatial transforms

Block-only clipboard paste supports 90/180/270-degree X/Z rotation plus
mirrors about X or Z. Known `facing`, `axis`, and 0..15 `rotation` fields
rotate along with the placement; simple invariant `waterlogged`, `lit`,
`powered` remain unchanged. Unrecognized modded state properties and all
legacy numeric block metadata refuse spatial transforms rather than being
incorrectly converted. The native Y/Z/X ordering, dimensions, original
source provenance and existing BE collision/Undo checks remain intact.
Iced provides immediate transform preview buttons, all still requiring
explicit normal stage confirmation. No CI or application builds this round.

## Migration batch 3, round 3/5 — interactive orthogonal vertical slices

The native editor now supports strict original X/Y and Z/Y vertical planes
up to 64x64 with explicit missing chunks/Sections and original BE markers.
The top row corresponds to maximum Y; negative Y and custom Java heights
are supported. iced shows clickable 16x16 vertical grids whose block cells
select an exact 1x1x1 staging area; larger planes remain read-only text
maps. Existing transaction/BE/Undo safety applies to all selections.
This adds a second graphical projection, not textured 3D GPU rendering.

## Migration batch 3, round 4/5 — overlap-safe original block MOVE

The Rust Session now previews a true block-only Move: freezing source
blocks, setting the vacated source to the correct original-layout air,
and overlaying the frozen blocks at the destination. Source/destination
overlap is deterministic and the full move is one Undo/Redo group. All
original source and destination Sections, BE coordinates and numeric-vs-
named identities must pass strict preflight; moves touching opaque BE NBT
are refused. Combined source/destination changes remain bounded to 4096
cells. iced exposes a Move preview button next to the existing Clone.
Original Minecraft disk content is never changed by a Move preview.

## Migration batch 3, round 5/5 — original classic schematic export

Classic MCEdit GZip `.schematic` export is now symmetrical with the existing
numeric-ID importer: exact 8-bit Blocks, four-bit metadata Data bytes and
optional nibble-packed AddBlocks high IDs are preserved, with empty entity,
Block Entity and scheduled tick lists. Source capture refuses all original
BE coordinates, unknown Sections and modern named-only IDs, rather than
exporting a lossy machine schematic. Files are create-new outside the world,
up to 4096 cells; iced supports export alongside classic import and modern
Sponge v2. The five feature milestones are complete; the planned next step
is ONE consolidated Linux/Windows `cargo check` + `cargo test` run, with no
application binary building or packaging.

## Migration batch 4, round 1/5 — interactive isometric 3D viewport

A native iced 0.14 canvas now paints a real-height, isometric 3D projection
of existing Java block columns (up to 16x16 footprint and 32 Y layers).
It uses per-state untextured shaded cube polygons, original BE highlighting,
negative Y coordinates, exact world XYZ selection and an event-driven mouse
picker; chunk/Section absence is not silently generated as air. The surface
is frozen for UI consistency and invalidated whenever sessions change.
This is the initial interactive 3D editor surface, not yet texture-atlas
rendering, full occlusion-aware terrain meshing, perspective rotation or
GPU-optimized chunk rendering. No application builds or CI before 5 rounds.

## Migration batch 4, round 2/5 — surgical existing BE scalar NBT edits (preview only)

An original Block Entity can now have one existing TAG_Byte/Short/Int/Long/
String scalar edited through a byte-range NBT surgery. The edited field must
already exist; no unknown fields are removed or normalized. `id`, `x`, `y`,
`z`, `keepPacked`, nested compounds, arrays and entity identity are protected.
Full reparse checks every original block state, BE position and unchanged
sibling field. iced computes a candidate digest without staging or writing
ANY disk data. Persistent BE changes, BE relocations and BE Undo are NOT
available yet; the native Region transaction still rejects any opaque NBT
change not caused by normal block-state edits.

## Migration batch 4, round 3/5 — immutable opaque BE snapshots

Any original Block Entity's full Compound NBT payload can be exported to
an external `.mcedit2be` provenance-bound snapshot without normalization.
The file records the exact original world/dimension/XYZ and both complete
source-chunk and BE-payload SHA-256 digests. Loading it validates source
content byte-for-byte against the current original world, refusing stale,
modified or cross-world snapshots. Create-new file semantics prohibit silent
overwriting; exporting inside the original world is refused. iced exposes
export and verify commands. This is a lossless evidence/backup primitive,
NOT a Block Entity insertion, paste, mutation or world write feature.

## Migration batch 4, round 4/5 — four-direction 3D camera

The iced isometric surface camera rotates in quarter-turns around the
vertical world axis while keeping exact original XYZ selection. Each view
sorts polygons in its own painter order, and click picking reverses that
same order so the nearest visible surface wins. The camera never changes
source NBT or edit positions. Rotated terrain is still an untextured
highest-block-per-column preview, not a complete interior/hidden-face
3D mesh renderer.

## Migration batch 4, round 5/5 — declarative plugin compatibility entry

A versioned `.mcedit2plugin.json` manifest can declare a strictly bounded
replace-blocks operation with exact optional DataVersion, dimension,
replacement and source-state filter. The operation uses the same original
NBT checks, BE refusal and single Undo transaction as manual edits; iced
shows a preview and the user must still choose Apply. Plugin JSON is
size/type checked and never executes arbitrary Python, shell, DLL, network,
or native plugin code. This is a minimal extension protocol foundation,
NOT compatibility with original MCEdit2 Python plugins. Broader plugin
permissions, signed sandbox runtimes and complete legacy plugin APIs are
still unimplemented. The five source rounds are now complete; run only
Linux/Windows `cargo check` and `cargo test`, no build/package actions.

3D CI follow-up: iced 0.14 requires an Element (e.g. `text`) rather than
a plain `String` as button content. The voxel inspector now reuses the
already decoded source chunk and overlays staged effective block states
without re-decompressing NBT for every cell. The isometric Y projection is
anchored to the top Y level so visible high blocks are on-screen.

## Five-batch migration cycle — Batch 1: multi-layer voxel view

The editor now reads the actual bounded 16x16x32-or-smaller Java volume,
uses original known air/solid cells and renders all visible top/side faces,
including exposed underground cavities. Completely surrounded voxels are
culled. Missing chunks/Sections are UNKNOWN, never synthesized as air.
Boundary faces indicate selected viewport clipping, not world adjacency.
The iced isometric canvas has separate highest-column and exposed-volume
buttons; top and side polygons select the original exact XYZ. This remains
an untextured bounded view, not a GPU mesh renderer.

Scheduling correction: only run one compile/test-only CI after FIVE FULL
migration batches. This is batch ONE of the new cycle, not five subrounds.
No CI/build/package is triggered by this source-only commit.

## Five-batch migration cycle — Batch 2: opaque BE scalar staging

An existing original signed/String scalar can now be preflighted and staged
in a bounded independent in-memory NBT overlay with separate BE Undo/Redo.
The original chunk SHA and exact effective source are verified again before
a byte-identical scalar replay is accepted. Stale previews or changed
original disk bytes are refused. The editor exposes clear BE-specific
Stage, Undo, Redo and inspect controls, distinct from block-state Undo.
All legacy block-only Region disk writes and journal exports are DISABLED
while any BE NBT remains staged, so unsupported serialization cannot
silently drop a modded BE edit. No new GitHub CI run (batch TWO of five).

Batch-2 hardening: click-picking any X/Z, vertical or isometric voxel
invalidates a prepared BE scalar edit; previews display real candidate
SHA-256 instead of a placeholder. Inspecting staged BE bytes also verifies
that the complete original chunk has not changed independently.

## Five-batch migration cycle — Batch 3: BE scalar replay checkpoint

BE scalar edits are now exportable as a provenance-bound, create-new
`.mcedit2bejournal.json` file outside the original Minecraft world.
Each step records original full-chunk SHA-256, exact original XYZ,
existing scalar field/value, and the resulting entire-chunk digest. The
exporter reconstructs EVERY active operation from original source bytes
and refuses truncated/non-replayable histories. Import checks exact
world/dimension/DataVersion and all source hashes, replays every operation
in a cloned temporary Session, and swaps it into place ONLY if all checks
pass. The restored history supports separate BE Undo/Redo. No block-only
Region write is enabled while any BE bytes remain staged. This is a
checkpoint/restore capability, not a Minecraft world writer.

**Cycle: 3 full batches of 5 completed; no CI yet.**

## Five-batch migration cycle — Batch 4: source-replayed BE Region preview

The native Region planner now accepts an explicit ordered proof of original
BE scalar changes, independently verifying a byte-for-byte reapplication
against the original chunk's NBT. Both BE-only and mixed block/BE changes
can produce a complete append-only in-memory Region image. The final
transaction verifier reconstructs all block edits AND all permitted BE
tag surgeries from the original decompressed chunk to ensure unknown
modded NBT, original inventories, positions and neighboring Blocks remain
unchanged. No proof, a wrong field/value, extra opaque byte changes or a
nonexistent chunk is rejected. **The disk transaction explicitly refuses
any candidate containing BE operations**; this stage only validates
complete save previews, not BE writes. No CI (batch FOUR of five).

## Five-batch migration cycle — Batch 5: explicit verified BE single-Region save

BE-only and combined block/BE modifications now have a distinct protected
single-Region transaction. The UI requires a fresh full-byte-identical
Region preview and the exact `SAVE_BE_NBT` typed confirmation, separate
from the existing block-only `SAVE`. The native Region transaction still
verifies the entire original file/append-only chunk sectors and replays
ALL BE scalar changes byte-for-byte. It writes a synced immutable ORIGINAL
backup, a pending marker, and a synced staged replacement, followed by
an atomic original Region path replacement. FINALIZE or ROLLBACK remains
required before re-opening the world; recovery restores ORIGINAL bytes.
BE move/paste, nested-Compound or inventory editing, and multi-Region
atomic writing remain unsupported. Use disposable copied worlds until
all relevant old/new version fixtures are validated.

Five FULL migration batches complete. Now run exactly one consolidated
Linux/Windows **cargo check/test ONLY** validation, never build/packaging.

## Five-round development cycle — Round 1/5

Expanded strictly byte-local original Block Entity scalar surgery to TAG_Float
and TAG_Double, encoding IEEE 754 big-endian original NBT payload bytes and
rejecting NaN/infinities, unchanged scalar values and malformed syntax.
The existing exact-byte siblings, BE positions and block state verification
still apply. Added modded machine Float/Double regression fixtures.

## Round 2/5 — nested modded Compound scalar leaf editing

The original byte-level BE editor now accepts slash-separated paths through
up to eight real existing TAG_Compound ancestors (e.g.
`ModdedCompound/Inner/BurnTime`). The leaf is changed in place, while
siblings at every nesting level are verified with their original exact NBT
bytes. Lists, arrays, unknown leaf types, identity or coordinate fields
remain protected; paths never create or delete tags. Test fixtures include
nested opaque mod data and invalid path refusals.

## Round 3/5 — atomic declarative BE NBT scalar batches

An external strict `.mcedit2beops.json` file (<=64KiB) supports 1–24
existing numeric/String BE scalar replacements with per-world/dimension/
DataVersion scope. The engine clones the original edit Session and replays
every operation with original source SHA and byte-preserving NBT checks;
only on full success does it adopt the new in-memory overlay. A later bad
operation leaves no partial changes; each accepted edit retains its BE Undo.
The iced GUI provides separate preview/stage controls. No disk writes.

## Round 4/5 — source-verified multi-Region dry-run manifests

Added strict external `.mcedit2regionplan.json` preflight reports: the
entire original Region source path/digest, edited chunk count, BE scalar
proof count, full candidate SHA-256, size and timestamp for every affected
Region. The planner rereads each original file and refuses symlinks,
path escapes, duplicates and concurrent mutations. Export is create-new,
outside the Minecraft world, and verification reconstructs the full plan
at its stored timestamp. No multi-Region disk commit has been enabled;
reports make the non-atomic boundary explicit. Regression covers change
detection and no disk writes.

## Round 5/5 — bounded nested original BE field-tree inspector

Original Block Entities now expose a read-only nested NBT Compound tree,
including canonical slash paths, tag types, raw payload byte counts and
immediate child counts. Up to 256 fields and eight Compound ancestor levels
are inspected; ambiguous or unsafe field names are not marked editable.
This makes the new nested scalar surgery discoverable in the GUI without
rewriting original modded arrays, inventories or sibling payloads.
All five rounds are now complete; one check/test-only Linux/Windows CI
may run. Packaging and binary builds remain manual only.
