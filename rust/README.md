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

**On-disk changes require explicit native Region preflight, backups and a
separate confirmation.** Merely inspecting or loading texture assets never
writes Minecraft world files. Full vanilla/modded model fidelity, Bedrock
LevelDB and whole-world conversion remain incomplete.

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

## Migration cycle — Major commit 1/5: Sponge Schematic v3

The native Java edition block-only interchange reads and exports the official
v3 root `Schematic` compound with nested `Blocks.Palette` and `Blocks.Data`
VarInts. Version-3 importer enforces exact original DataVersion, bounded
volume/palette size, zero offsets and empty BE/Entity/Biome containers.
It cannot silently drop original modded inventories or entities and never
writes original region data on import preview. iced adds separate v3 export
and guarded paste preview alongside v2 and numeric legacy schematics.

## Major 2/5 — 3D geometry editing tools

Five native original-world-safe brushes (hollow box, ellipsoid, hollow
ellipsoid, Y-cylinder and Y-cylinder shell) now share the strict bounded
block editor. They only read/mutate voxels in the actual brush mask, refuse
original BE collisions and missing source Sections, use exact optional
block filters, and stage as one Undo/Redo group. iced exposes dedicated
preview commands; no world files are written during brush previews.

## Major feature 3/5 — isometric camera controls and world-space picking

The iced 3D surface supports bounded zoom (0.5x–3x), XY pan, four viewing
quarters and camera reset, with selected block highlighting. Drawn top and
side polygon geometry shares the same transforms with original XYZ mouse
picking; the NBT world/editor overlays remain unchanged by camera actions.

## Major 4/5 — versioned multi-operation safe plugin engine

The declarative plugin protocol now accepts v2 `.mcedit2plugin.json`
manifests with up to 24 dependent ReplaceBlocks/Brush actions, optional
exact source DataVersion and dimension constraints. Every action is safely
simulated on a cloned original-world Session; its resulting effective block
states feed the next action, including the new 3D geometry masks. Only after
the entire sequence passes strict original BE/source preflight is the final
state compressed into ONE bounded (<=4096 distinct cells) preview and one
Undo/Redo group. Existing v1 manifests remain supported; neither version
executes untrusted Python, shell or native code. Iced's plugin preview UI
accepts both manifest versions.

## Major 5/5 — original Java entity sidecar catalog and snapshots

Recognizes both legacy embedded Entities lists and separate modern
`entities/r.X.Z.mca` files. The strict read-only catalog validates actual
entity Region chunk Position metadata, reports modded entity IDs, original
Compound SHA-256, tag counts and optional finite Double positions. An
external create-new `.mcedit2entities` file stores exact original entity
chunk NBT and full source provenance; verification refuses changed source
bytes. iced adds entity inspection, export and source verification. No
entity mutation or hidden BE serialization is enabled.

## Display-first cycle, major 1/5 — visual material categories and staged voxels

The original voxel scene classifies transparent, foliage, liquid and solid
materials purely as render hints. Exposed voxel faces account for known
transparent neighbors; unknown source cells are never synthesized as air.
Original Block Entities and staged edits have separate highlight colors.
This does not mutate NBT or any world file.

## Display-first cycle, major 2/5 — orthographic 3D camera and face painter

Replaced fixed diamond projection with a real 3D orthographic camera, full
six-face original voxel geometry, continuous 360-degree yaw and 15–80 degree
pitch, depth-ordered face drawing and exactly matching world-coordinate
face picking. UI exposes 15-degree orbit and pitch adjustment controls,
while preserving zoom/pan and quarter-turn shortcuts. The canvas renderer
is still CPU-side polygons (not yet textured GPU terrain meshes).

## Display-first cycle, major 3/5 — real canvas mouse navigation

3D Canvas owns button-drag interaction state: right mouse drag orbits
continuously with fractional movement accumulation, middle drag pans,
mouse wheel zooms and left click picks exact original XYZ. Hovering reports
the visible original block ID plus BE/staged markers; mouse exit clears it.
No synthetic world edits or background writes occur during navigation.

## Display-first cycle, major 4/5 — navigable source regions and live update

3D source-only selections now span 16x16x32 (8192 real voxels), independent
of the stricter 4096 editable block limit. Paging across negative/positive
X/Z Region positions, Y bands, and centering on current XYZ are protected
against overflow. Staging a block edit and Undo/Redo automatically reloads
the visible scene so overlay materials and geometry track changes. World
chunks and missing Sections are never created by navigation.

## Display-first cycle, major 5/5 — 3D scene guides, diagnostics and clipping

The original-source 3D viewport now renders its full 3D clipping box and
color-coded XYZ axes with toggles, original Block Entity/staged/material
statistics, and exposed-face counts. Polygon faces entirely outside the
canvas are culled for both draw and picking. The guides are separately
labeled as viewport boundaries, NEVER generated Minecraft geometry.
Five large display features completed; now one Linux/Windows cargo check/test
validation is appropriate. Application binary building/packaging stays manual.


## Original-world 3D display migration — five compressed commits (2026-10-24)

This is **not** full MCEdit2 renderer parity and not a textured GPU block
modeler. The current iced Canvas 3D inspector is read-only with respect to
display, and saves continue to require explicit native validation. The five
new large functional batches on rust-iced-rewrite add:

1. Approximate named, legacy-numeric and dyed Java materials; transparent
   water/glass and deterministic non-registry modded fallback colors.
2. Material-preserving near/far-rendered hovered/selected face outlines,
   exact selected face normals and original XYZ face information.
3. Recomputed original-state, signed-Y top cutaway meshes (not just an
   invisibility mask), bounded by the inspected source scene.
4. Two-click-corner 3D edit-box preview, with exact negative coordinates
   and the existing 4096-cell guard; copying it to edit bounds never stages.
5. Optional original-world chunk X/Z floor grid and 16-block section Y
   cross-sections, with floor division across negative coordinates.

The viewport remains a CPU-projected bounded 16x16x32 block rendering,
not wgpu Minecraft texture atlases / block models, biome shading, or a
complete 3D entity editor. Mouse picking and guides describe displayed
source voxels only; missing chunks/sections cannot be treated as air.

CI is compiled/tested only after all five batches: changing the
\`.github/rust-ci-batch.trigger\` sentinel on the fifth commit starts one
Linux/Windows check+test-only workflow. Ordinary feature commits carry
\`[skip ci]\` and do not modify the sentinel. Neither the workflow nor
these batches invokes \`cargo build\`, release packaging or uploads.


## Five-large-batch 3D texture and model migration (2026-10-24)

1. Bounded read-only ZIP/directory block PNG loader with namespaced mod textures,
   log-axis faces, grass overlays and original blockstate identities.
2. Depth-ordered texture UV tessellation on existing iced Canvas, adaptive
   quality budgets and safe color fallback with original XYZ picking.
3. Vanilla blockstate-aware partial cuboids (slabs, stairs, snow layers,
   carpets, paths, trapdoors and simple connected fence rail shapes).
4. Blockstate variant JSON and model parent/#texture-alias parsing with
   namespace-aware keys and depth/cycle limits.
5. Explicit desktop pack input (folder, .zip or original client .jar),
   load/unload/toggle status, dimension guards and original BE/staged outlines.

LIMITATION: This is still CPU/iced Canvas textured geometry, not a full GPU
mesh renderer, arbitrary multipart JSON element model pipeline, animation,
or multi-chunk streaming. Minecraft world files are never touched by the
texture loader. The last commit of each five-large-feature batch updates
the CI trigger; jobs run cargo check and cargo test only, no packaging.


## Five-substantive-commit GPU rendering phase (2026-10-24)

1. Shared bounded source-face tessellator to GPU-ready RGBA triangle vertices.
2. Native iced 0.14 `shader::Program` and wgpu vertex buffer plus WGSL
   alpha-blending renderer. Rendering is batched into one draw call per view.
3. Switchable wgpu underlay and Canvas fallback with transparent interactive
   Canvas overlay retaining world picking, camera orbit, selection, BE and
   section/grid guides. GPU mode never writes world or editor data.
4. Strict unrotated resource-pack JSON model `elements` converted from 0–16
   Java coordinates to actual partial cuboids, bounded inheritance/element
   count and safe fallback for unsupported rotation/multipart variants.
5. Fast/Balanced/Detailed GPU projected-texture quality profiles, bounded
   draw complexity, shared geometry independent of texture toggle, render
   diagnostics and read-only Canvas overlay hot-path optimizations.

GPU geometry is real via iced/wgpu; face textures are still CPU-sampled
into vertices before upload, NOT yet a GPU-sampled texture atlas. Depth is
currently painter-sorted transparency, not a true GPU depth-buffered 3D
render pass; multi-chunk streaming and arbitrary mod models are not done.
The renderer has a capped 16x16x32 original-world window. The PNG/model
loader cannot modify Java chunks, unknown block entities or other NBT.
CI is gated by five large commits and only checks/tests, never app builds.


## Five-batch native GPU atlas and model-rotation milestone (2026-10-24)

1. Stable, bounded namespaced RGBA texture atlas with one-pixel duplicate
   gutters and a white fallback. CPU downsamples first animation frames
   into atlas tiles without modifying pack archives or world NBT.
2. GPU UV coordinates replace high-cost CPU per-pixel quad tessellation:
   one quad / two triangles per original visible Minecraft face.
3. Native wgpu GPU atlas texture upload, WGSL sampling, alpha discard and
   switchable nearest/linear filtering with atlas reuse between frames.
4. Correct 90-degree X/Y blockstate variant element rotation, inverse
   displayed-to-source face texture selection, negative-coordinate-safe
   original XYZ picking. UV lock, arbitrary rotations and multipart models
   remain explicit unsupported fallbacks.
5. Shared GPU/Canvas face budget and status readouts; first-frame animated
   PNG heights up to 4096px allowed with 512px maximum width/frame size.

Limitations: The viewport still reads at most 16x16x32 original blocks.
GPU texture atlas is RGBA8Unorm nearest/linear sampled but it is not a
full Minecraft material/lightmap engine. The previous painter-only renderer
was replaced by a bounded offscreen opaque Z pass plus translucent pass. Transparent/cutout pixels
preserve alpha, but complex interpenetrating translucent meshes and model
elements rotated 22.5/45 degrees remain unsupported. Do not merge until
representative resource packs and real Minecraft worlds have been checked.


## Five large 3D display migration commits (October 24 2026)

1. Read-only original resource-pack PNG alpha classification into opaque,
   binary cutout and blended (liquid/glass) material modes. The GPU fragment
   shader tests alpha cutout pixels before blending.
2. Per-element model JSON face mappings, texture alias lookup, UV crop and
   explicit 90-degree face UV rotation in the GPU texture atlas.
3. Conditional multipart model elements with bounded AND/OR property rules,
   original block-coordinate ownership and the original model element face.
4. Frame-local exact-state geometry cache and GPU material/UV cache to reduce
   repeated JSON/texture traversal across repeated source blockstates.
5. True projected element-face vertices for vanilla 22.5/45-degree
   element rotations about explicit origins and optional rescale, including
   variant right-angle rotations; safe fallback on unsupported transforms.

The workspace continues to use a capped 16x16x32 source volume. GPU
transparency is painter-ordered, not a complete depth-buffer/OIT solution.
Arbitrary weighted variants, full Java model UV-lock, animated timeline,
large-volume streaming and screenshots still need development. Neither
resource loading nor 3D interaction changes original world NBT.


## Five-batch bounded cross-chunk rendering migration (October 24, 2026)

1. Read-only original Java scene limit raised to 32×32×48 / 49,152
   sampled cells per request, while preserving the independent 4,096-cell
   edit transaction limit, original chunk caching and missing-chunk honesty.
2. Desktop 16×16×32 and 32×32×48 inspection presets, full coordinate
   validation, signed-Y scene navigation and unchanged edit confirmations.
3. Camera-prioritized bounded face selection across large views, replacing
   chunk-order early truncation. GPU display and Canvas picking use exactly
   the same nearest retained faces and <=150k polygon budget.
4. Known vanilla partial occluders (slabs, stairs, fences, panes, snow
   layers, etc.) no longer incorrectly hide neighboring exposed faces.
   Unknown and modded neighbors are not inferred as air or partial blocks.
5. Polygon centroid depth for rotated faces plus stable opaque/transparent
   painter-order ties, shared by the GPU and original-world selection.

No app build or packaging runs in CI; only cargo check and cargo test.
A 32×32×48 bounded inspector is NOT general infinite chunk streaming;
full GPU Z depth/OIT, texture animation and complete models are pending.
World NBT and BE bytes remain read-only until a separate confirmed save.


## Unbuilt, untested source-view rendering batch (October 24, 2026)

Five sizeable feature commits since the preceding successful CI:
1. Per-pixel barycentric picking for actual rotated triangle depths,
   matching the GPU triangle split instead of center-depth ordering.
2. One shared per-view projected-face cache across GPU, Canvas interaction
   overlay and hit testing, with viewport dimensions as the cache key.
3. Geometry-derived implicit Java model UVs when face uv is omitted,
   respecting slab/stair/partial-model face dimensions and inherited textures.
4. Explicit Java model face presence: absent model faces are not created,
   and face textures stay tied to the unrotated local model face.
5. Real source-chunk tile metadata for multi-chunk read-only inspection:
   source presence, known/unknown columns and exposed voxels; transparent
   diagnostic grid marks missing chunks red, partial columns amber and
   fully known tile green, without generating or modifying world blocks.

IMPORTANT: These changes intentionally have not run CI, compilation,
packaging or executable builds. Run cross-platform cargo check/test ONLY
after an explicit user request. All commits use [skip ci], and the batch
CI trigger file is left untouched. UI/graphics behavior remains unverified.


## GPU/Canvas parity and source-picking improvements (2026-10-24; no CI)

- Original-block face picking tests projected barycentric depth and the
  same texture atlas alpha mask as GPU cutout rendering, so fully transparent
  texture holes are not clickable false solids.
- Canvas fallback now samples per-element Java JSON cropped/rotated UVs and
  PNG alpha via the common atlas, rather than a generic full-face texture.
- Native wgpu upload caches a vertex-buffer fingerprint: unchanged geometry
  (for example hover/outline changes) no longer requires an identical large
  GPU queue.write_buffer; buffer growth always forces upload.
- Cutout textures fill zero-alpha RGB within 3 texels of opaque artwork in
  the memory-only atlas to reduce black fringes with linear interpolation,
  without changing PNG pixels, alpha values or saved files.
- GPU and CPU Canvas use rotated world-space face normals for directional
  brightness, so tilted 22.5/45-degree model elements have consistent
  interpolated light without a Minecraft biome/lightmap implementation.

The five commits all use [skip ci]; no GitHub Actions execution, cargo
check, cargo test, cargo build, packaging or release has been requested.
Compilation and graphical rendering still require later verification.


## Full render-feature history consolidation

The native 3D display, texture atlas, JSON model projection, interactive
Canvas/GPU rendering, bounded source-volume inspection, and model-face
culling/UV selection are one feature family and now appear in a single
functional commit rather than repeated artificial five-commit rounds.
JSON faces without `cullface` remain visible near solid neighbors;
explicit `cullface` directions are rotated into original world axes and
only culled when the actual transformed face reaches the voxel boundary.
CPU picking still refers to original source block coordinates; unresolved
or modded blockstate variants never justify destructive source guesses.
Compilation and GPU runtime appearance after consolidation are UNVERIFIED;
do not run CI or package unless explicitly requested by the user.


## Native offscreen GPU depth migration (no CI requested)

GPU source vertices now preserve the original isometric world-view depth
and normalize to WebGPU's 0..1 depth range without changing source NBT.
Each shader frame draws into a bounded offscreen RGBA8Unorm render target
and Depth24Plus texture (max 4096 pixels per edge):

- Opaque and alpha-tested PNG cutout triangles write a real Z buffer,
  resolving intersecting cuboids and angled source model elements.
- Alpha-blended faces depth-test against opaque Z but do not write it;
  transparent triangles remain depth sorted back-to-front. Transparency
  accumulation and final iced-window composite use premultiplied alpha.
- The resulting 2D scene texture is composited into the existing iced
  shader rectangle; Canvas mouse orbit, picked XYZ, guides, and overlays
  remain the existing UI owner, not a second window renderer.
- Viewport dimensions, atlas, sampler quality or mesh byte changes
  invalidate and rerender the offscreen scene. Unchanged hover-only draws
  reuse its GPU output without reuploading the original-world triangles.

Known limitations: sorted transparency is NOT order-independent OIT, and
the Canvas-only fallback still uses the painter's algorithm. This is
not infinite world chunk streaming or an authentic Minecraft lightmap.
The new shader and render-pass setup have NOT been compiled, GPU-tested,
or run in CI; this requires an explicit future user request. Do not
run an application build or package as part of CI.


## Exact zero-thickness Java vegetation planes (no CI requested)

Java resource-pack models may use zero-thickness elements for flower and
sapling crossed planes (for example vanilla block/cross). This renderer
now recognizes exactly one collapsed model axis when and only when its
declared JSON faces actually face that axis. Ordinary full-volume
Box3 validation remains strict: malformed/degenerate source shapes are
not guessed or used in world writes. Plane vertices still pass through
the original model's rotation, rescale and orthographic projection.
Elements with `shade:false` have full intrinsic directional brightness
in both the native GPU mesh and Canvas fallback; all block picking retains
the original Java world XYZ. The resource pack must supply the actual
model inheritance and transparent PNG for faithful visuals.
No CI, runtime rendering or build was executed for this feature.


## World-border-safe 3D camera coordinates (no CI requested)

Java world X/Z coordinates can reach tens of millions of blocks. The
orthographic source projection previously rounded absolute i32 positions
to f32 before subtracting the camera, collapsing neighboring source cells
at large magnitudes. The projection, source selection wireframes, chunk/
section guides and source model vertices now use exact f64 world positions
and subtract the camera origin before converting the bounded, relative
offset to f32 GPU coordinates. Tests cover opposite +/-30M border views.
This is a display-only precision improvement; original BE/NBT, source
coordinates, edit bounds, and disk save safeguards remain unchanged.
CI, compilation and runtime visual verification have not been requested.

## Progressive very long-distance terrain viewer (source-only, no CI)

The near-field exact inspector now supports **64×64×48** cells, independently
from its unchanged 4096-cell edit budget. The separate far-terrain
viewer does **not** allocate all source blocks in a gigantic cube.

- Distant presets are radii of 16, 32, 64, 128 and **256 original Java chunks**.
  Radius 256 spans 8,208 blocks horizontally (4,096 blocks from center
  to the edge). This exceeds typical Minecraft normal render distances.
- A bounded camera-centric ring plan samples near chunks at full horizontal
  block density, then samples only every 4/8/16 columns in progressively
  sparser ORIGINAL source chunks. It includes outermost ring coordinates.
  Maximum planner capacity is 20,000 sampled source chunks; no artificial
  world generation, entity rendering, ticking or spawning.
- Streaming reads 12 source chunk patches per asynchronous UI update,
  builds an original-chunk status record and appends only known terrain
  samples. Missing chunks/Sections are not invented; authenticated block
  editing remains independent and never consumes approximate previews.
- `SurfaceVoxel.span>1` is an explicit approximate LOD preview. The projected
  X/Z tiles have the corresponding visual width, but mouse picking rejects
  approximate spans. An exact `span=1` position still reports the ORIGINAL
  source XYZ, and original BE data is never rewritten.
- Normal modern world height samples -64..319; older original worlds
  default to 0..255. The near-field inspection retains adjustable signed Y.
  The far stream cancels stale generations when users move to another scene.
- Large view projection uses camera-relative f64 then bounded f32 clip
  coordinates. Chunk-grid debug outlines are restricted to 256 tiles per
  frame at these radii. Face and GPU buffer budgets are still enforced.

Important limitations: This is progressively sampled *terrain*, NOT
Minecraft's exact full-detail 256-chunk simulation. Distant water and
modded block geometry may be approximated, with far LOD never selectable.
No user-requested CI run or app build has occurred for this feature;
Rust compilation, GUI threading and actual GPU drawing remain unverified.
Do not initiate CI until all 3D display migration is complete and the
user explicitly requests the final consolidated check/test run.

## Java blockstate UVLock texture-orientation parity (unvalidated)

The JSON blockstate `uvlock:true` flag now computes a complete mapping
from the four rotated source model-face vertices to the corresponding
world-facing canonical texture corners. This remaps the atlas's UVs for all
multiples of 90 degrees on both X and Y, including geometrically mirrored
corner order: it does not use an incorrect rotation-only shortcut.
Original model face UV crop and per-face rotation are applied before this
corner permutation. The same atlas style is used by native wgpu rendering,
Canvas fallback and alpha-tested original-block picking.

This is a read-only rendering change. Do not execute CI, compile or
package until the entire 3D display feature is complete.


## Extreme read-only terrain view distance (no CI)

The distant-terrain planner offers progressive 16 / 32 / 64 / 128 /
256 / 512 / 1024 / 1536 / 2047 chunk radius presets. The largest
radius is 32,752 world blocks from the center to each horizon edge,
excluding unseen or unavailable source chunks. To keep memory finite, the center samples
single source block columns and successive chunk rings use 4/8/16-block
LOD footprints; beyond 256/512 chunks the planner also skips more of
the farthest chunks. Past the 1024-chunk radius, only every eighth
outer ring is sampled, with a coarser angular step; the final source
radius is always represented. A view plans at most 36,000 source chunk
requests, processed in bounded asynchronous batches. Each batch reuses
up to 32 validated read-only Region indexes, avoiding repeated 8 KiB
header parsing for adjacent original chunks while retaining per-chunk
NBT and Block Entity coordinate checks. Nothing is written. It does not inspect or render entities.
Far LOD tiles are never authenticated editable block picks. This is
**sparse distant terrain** rather than pixel-perfect all-block terrain:
unloaded or unplanned areas are not fabricated. The GPU projection uses
camera-relative coordinates and a near-zero minimum scene scale so far
terrain can fit the view. 1024–2047-chunk inspections can take
considerable time on very large saves; a 2047-chunk panorama is highly
sparse and is NOT a promise of continuous terrain coverage.
CI and executable build are still deferred until the entire 3D display
migration has been completed, as explicitly requested.


## Progressive far-terrain rendering budgets and nonblocking reads (unverified)

For 16–2047-chunk radius read-only scenes, the 150,000-face GPU/Canvas
budget is now distributed by **actual source LOD**: detailed original
voxels receive a near-detail allocation, while sampled terrain and the
outermost 25% of the horizon retain guaranteed face capacity. Empty
LOD layers return their unused space to populated layers, and source
picking never treats multi-block LOD samples as authenticated blocks.
Far sampled-height cells render only their observed top faces; the editor
does not invent artificial vertical cliffs where neighboring heights
were not inspected. No entities, lighting simulation or collision data
are loaded for far terrain.

The desktop viewport now shares the original SurfaceView behind Arc for
GPU, Canvas and UI draws. Incoming read-only chunk batches use Arc
copy-on-write only when source samples change, rather than copying all
previously loaded world voxels on every hover or redraw. An immutable
Session snapshot is captured once per far view; small batches of 12
exact near chunks or up to 128 sparse distant chunks execute on bounded
OS worker threads and report through a one-shot iced future, keeping
blocking Region I/O off the iced UI executor. Canceled views discard
outdated responses by nonce and world selection.

This is performance-focused 3D display migration, not a change to the
4,096-block edit transaction cap or to original NBT/BE bytes. The code
has NOT been compiled, tested in CI, or visually validated; check/test
runs remain deferred until the complete 3D display migration is ready
as requested by the user. No application packaging or builds.


## Camera navigation reuse of far source tiles (unverified)

Pan/recenter reuses a bounded cache of real source-chunk patches keyed by
chunk X/Z and LOD sampling span. The cache keeps up to 4,096 patches and
90,000 sampled voxels; old patches are evicted FIFO. New camera requests
hydrate overlapping terrain immediately and schedule only uncached chunks.
Any staged edit, undo, redo, BE import, region finalization, or direct save
invalidates the cached Session snapshot. Cancellation and world switching
still discard late in-flight responses. Cached LOD samples remain read-only.
The new code has not been compiled or CI tested. No packing/building.


## Animated Java resource-pack textures (unverified)

Vertically stacked PNG sprite frames and bounded `.png.mcmeta` animation
metadata now control texture animation. The parser supports `frametime`,
per-frame index/time sequences and implicit sequential frames (1–1200 ticks
per step, 4,096 steps maximum). Invalid/untrusted metadata leaves the static
first frame visible and adds a warning. UV slots, source block models and
world NBT remain untouched. All sprite frames obey the existing 48 MiB
decoded texture-bank limit; metadata is capped at 32 KiB per sprite.
When an original 3D view is visible, iced's Tokio-backed subscription
ticks every 50 ms, building a new bounded GPU/Canvas RGBA atlas only when
a sprite's frame actually changes. The underlying immutable model and
sprite resources share Arc ownership, avoiding reloading packs on ticks.
Interpolation between frames is not implemented; discrete animation frames
are supported. No compilation, CI or application packaging was triggered.


## Persistent projected mesh and responsive overlays (unverified)

The exact projected original-world face buffer is now cached across iced's
reactive UI view rebuilds, rather than only among widgets in one frame.
Camera changes, source chunk arrivals, world/dimension selection and model
asset changes invalidate the cache; sprite animation frame changes only
replace atlas RGBA, and hover/selection outlines are drawn independently
by Canvas without regenerating the original geometry. Importing a block
patch also discards distant original-source cache data. This preserves
original source XYZ picking while reducing CPU projection during orbit/
hover and 20 TPS animated resource-pack updates.
No CI, compilation, executable build or packaging has been performed.


The fully clip-space-normalized GPU mesh now has its own persistent
`Arc<Vec<Vertex>>` cache, sized by the 150,000-face view cap. Atlas frame
changes, hover and selected-block overlays reuse identical vertices and
source face lists; GPU buffer writes are skipped by a matching hash.
Toggling source textures, changing camera projection, loading a different
pack/world or receiving new chunk samples invalidates both projected and
GPU caches together. CPU and GPU frame selection share one atlas tick.
This optimization is not yet CI-tested or runtime verified.


## Stable weighted Java model variants (unverified)

The Java blockstate renderer now evaluates weighted `variants` arrays
and weighted multipart `apply` arrays with a stable selection keyed by
the original integer block X/Y/Z, state and multipart index. No global RNG
state, random shifts while orbiting, or guessed editable source blocks.
Selected geometry and its JSON face texture/UV/cullface derive from the
same variant choice. Both Canvas and GPU texture lookup use position-aware
variants, while caches deduplicate identical selected model combinations
rather than generating one independent mesh for every source voxel.
Weighted counts and array size are bounded, and invalid metadata safely
falls back to the existing conservative display. Entity and disk world
operations remain excluded. This work is pending one final CI gate.

## Read-only oversized Java chunk sidecars (.mcc)

The native original-world reader now supports the vanilla external-stream
flag and neighboring `c.X.Z.mcc` files for GZip, zlib and uncompressed NBT.
An exact owning Region and original absolute chunk position are mandatory;
symlink and non-regular sidecars are refused. Compressed input is limited
to 32 MiB and expanded NBT to 64 MiB. An absent or malformed sidecar is
never treated as air. The existing on-disk Region preflight still refuses
saving external chunks; read support does not imply write support. LZ4 and
custom compression remain unsupported. No CI, compilation or packaging was
requested or run for this change.

## Full-resolution interactive 2D source slices

Horizontal X/Z and vertical X/Y or Z/Y native slices now display **all
64×64 source cells** in an iced Canvas rather than a 16×16 wall of buttons
or unselectable text. The map fits its viewport, colors known original
block identities (including stable synthetic modded colors), highlights
Block Entities, and distinguishes known air, missing chunks and absent
Sections. Clicks map back to exact signed XYZ in the source world. A click
on an unknown cell is ignored, and even a known cell must use the normal
preview/confirm path to edit; the 4096-cell edit cap does not change.
This preview does not fabricate missing source NBT and does not require
entity computation. Tests cover signed coordinates, 64×64 hit-testing,
vertical Y ordering and unknown-source color separation. These new tests
have not been run; CI/build/packaging remain deferred.

## Region index integrity before rendering (read-only)

Every Java .mca/.mcr header now verifies its 1024 sector extents for
cross-chunk overlaps before source blocks are trusted. Corrupt duplicated
or partially overlapping slot claims refuse the entire index instead of
showing potentially unrelated data in the 2D/3D view. Adjacent valid
allocations remain supported. Validation sorts at most 1024 fixed-size
ranges, so a sparse/malicious gigantic file cannot cause a sector-sized
memory allocation. The writer's stricter independent preflight and original
NBT/Block Entity protections are unchanged. CI, tests and packaging have
not been run for this migration batch.

## Adaptive 3D far-horizon source guides

Large 1024–2047 chunk panoramas previously drew thousands of 16-block
chunk-reference lines on every mouse interaction, while showing only the
first 256 source-chunk status borders. The Canvas now coarsens X/Z guides
to at most 128 lines per axis, with power-of-two Minecraft-aligned
spacing and negative-coordinate correctness. It preserves full 16-block
guides in near scenes. Source tile status markers retain near detail
and sample across the entire streamed panorama, including the last
outer-horizon tile; missing original chunks and Sections remain visibly
distinct. No extra chunk reads, entity computation, model rebuilds or
NBT changes are introduced. The draw/selection source remains the same.
This change has not been compiled or CI-tested.

## Resource-pack sampling in 2D slice maps

When original-state resource textures are enabled, both horizontal and
vertical native 2D maps now sample the already-loaded Java resource-pack
atlas using the resolved **top-face texture** of each original blockstate.
The same original XYZ selects deterministic weighted/multipart model
variants, and the same UV-corner ordering honors texture crop/rotation.
The preview is a 2D **top-face color/texture guide**, not an exact vertical
cross-section or a replacement for arbitrary 3D model geometry. Missing,
transparent, air or unsupported texture cells retain their verified source
palette appearance. A fixed per-cell sampling cap ensures a 64x64 map draws
at most 16,384 additional texture squares, without filesystem access,
entity processing or writes to the world. Tests cover atlas-UV interpolation
and bounded rasterization; CI/compilation remain unrun.

## Reused verified Region indexes for near 2D and 3D scenes

Single-shot 64x64 horizontal/vertical slices and 64x64x48 near-field
3D surface/volume inspections now share the existing worker-local
ReadOnlyRegionCache within each view request. This avoids reopening
and validating the same original 8 KiB Region index for adjacent chunks
while preserving exact per-chunk decompression, ownership, Block Entity
validation and absence reporting. Loaded original chunks are still
decoded only once per scene. The bounded cache never writes Region data
or extends the independent 4096-edit budget. Signed, sorted Section Y
lookup uses a binary search instead of a per-block linear traversal
through 1.18+ tall-world Sections; missing Y remains absent rather than
invented air. New negative-Y and missing-Section regression tests are
included but unrun; CI/build/packaging remain disabled.

## Asynchronous original near-field read dispatch (pending compile verification)

Near-field top-surface/exposed-volume 3D and up-to-64x64 horizontal/vertical
2D source inspections now dispatch their synchronous Java chunk decoders
on bounded OS reader workers instead of blocking iced's UI event handler.
A cloned in-memory Session captures original-world identity plus staged
block overlay at dispatch; workers NEVER modify world NBT. The UI queue
tracks a monotonic request nonce, and discards superseded results after
view changes, current-world/dimension switches or any staging/save action.
The background result never silently substitutes air for missing chunks
or missing Sections. Source coordinates, per-chunk BE checks and the
independent write-preview budget are unchanged. Validation remains pending;
CI, cargo check, cargo test and packaging have NOT been run.

### Near-field worker backpressure

Near-field source work is serialized to at most **one active worker**.
Rapid pan/zoom/slice changes replace the queued snapshot with the newest
request; an obsolete completion releases the slot but cannot publish
old world data. This bounds source I/O, decoded chunk memory and native
threads under frequent interactions. Worker errors release the same slot.
No more than one pending near-field view is retained in memory; far terrain
maintains its independent bounded progressive reader. No CI was triggered.
