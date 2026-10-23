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
