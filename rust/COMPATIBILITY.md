# Minecraft Java compatibility contract — Rust + iced rewrite

## Scope

Do **not** regress any version which the original MCEdit2 actually opened.
The original mceditlib Anvil adapter (src/mceditlib/anvil/adapter.py)
explicitly rejects pre-Anvil worlds on metadata.version != 19133.
Its additional PocketWorldAdapter is a stub which raises NotImplementedError.
The older *MCEdit 1* product is a separate compatibility target, and had
broader historical storage than the MCEdit2 repository.

Minimum version families that must be handled before release:

| Java release family | Original storage | Rust reader milestone | Safe editing |
| --- | --- | --- | --- |
| Alpha and early Beta individual chunks | 2-level base36 folders, c.x.z.dat gzip + Level.Blocks | Original Alpha path/index + 128-height reader | NOT YET |
| Beta-era Region to Java 1.1 | .mcr, 16x128x16 Blocks/Data | MCRegion array decoder | NOT YET |
| 1.2.1–1.12.2 (original MCEdit2's baseline) | .mca, Level.Sections numeric Blocks/Data/optional Add nibble | Native numeric parser | NOT YET |
| 1.13–1.17.1 | .mca, Level.Sections with Palette/BlockStates | Named Section parser | NOT YET |
| 1.18–1.21.x | .mca, root sections[].block_states with signed Y | Modern Section parser | NOT YET |
| 26.3 release (DataVersion 5023) | Modern Java Sections | Schema-based original-NBT parser | NOT YET |
| 26.4 Snapshot 5 (DataVersion 5125, 2026-10-20) | Modern Java Sections, pending all schema checks | Snapshot fixture reader | NOT YET |
| Future snapshots | May contain unknown layouts/compression | Inspect only if known structure; otherwise error | BLOCKED |

**A parser or unit fixture is NOT end-to-end validated game-version support.**
No save capability exists yet in the Rust workspace. Future writes must
explicitly pass whole-world validation, original opaque NBT preservation,
Block Entity identity/coordinates, entities, biomes, subchunks, region
compression/sidecars, and atomic backup/save transactions.

Unknown modded numeric block IDs are preserved as identity pairs
(e.g. numeric:301 + data:7); mapping them to a made-up modern registry ID
would silently corrupt modded worlds. Original unknown BE NBT bytes must be
retained in the world backend, not regenerated from generic objects.

The old MCEdit 1 supported formats older than Region. Alpha and early
Beta individual .dat chunk indexing and reading are implemented in
the Rust source (with strict canonical coordinate checks and bounded
decompression), but compiler and representative world fixture validation
remain pending. Classic/Indev finite worlds and other schematic-era
assets remain future separate milestones; DO NOT claim full support yet. Bedrock/Pocket
LevelDB and legacy PocketWorldAdapter are not functional MCEdit2 backends.

## Original CLI compatibility — source checked

Original scripts: src/mcedit2/main.py and src/mcedit2/editorapp.py.

- -debug, -log, -v, -vv: recognized, with adapted Rust/wgpu diagnostics.
- -h / --help: recognized.
- -resetPrefs: resets **separate Rust preferences**, never overwrites legacy Qt config.
- -view: recognized as read-only launch; Rust writer is not implemented.
- One or multiple positional files/directories: recognized. Original
  world paths, level.dat and level.dat_old are accepted by the reader.
- -eval CODE: recognized but must FAIL WITH A VISIBLE ERROR. Original
  executes arbitrary Python inside a Qt session; equivalent semantics
  cannot be provided by an all-Rust process. A future interpreter/script
  compatibility bridge needs a separate explicit design/permission.
- Original .schematic and other files can be positional arguments:
  command parsing does not mean those formats have an importer yet.

Therefore "fully compatible CLI" is currently a contract and a
regression target, NOT a completed claim. No silent arg ignoring.

## Rust development verification

The local ChatGPT execution container does not currently include
rustc/cargo, or have usable Internet package registries, so local
Rust builds have **NOT** succeeded. Treat new Rust source as
not-compiler-verified until an appropriate runner completes:

    cd rust
    cargo fmt --all -- --check
    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo build --release -p mcedit2-iced

The GitHub Actions Rust workflow is intended to verify the five-major-
feature batch. Prior MCEdit2 Python tests and user-owned worlds should
be used for differential native read/write validation once integrated.
Never commit private worlds.

## Original Alpha / Beta chunk-file safety

Original Java Alpha paths use lowercase base36 chunk coordinates and two
hashed base36 bucket directories. For example chunk (-13,44) is stored as
1f/18/c.-d.18.dat. The Rust reader verifies canonical filenames, refuses
directory and file symlinks, bounds input gzip size and expanded NBT, and
matches the stored original Level.xPos and Level.zPos against the filename.
Original unknown NBT is never written or converted.

This reader is not yet a full Alpha world editor: biome, tile-entity and
old entity semantics must be tested before any save capability.


## Unified 3D coordinate block query

The Rust world API read_world_block(world, dimension, x, y, z) provides
one strict nonmutating lookup for the historical and modern save families.
It uses floor division to index negative world coordinates, refuses
ambiguous .mca/.mcr locations, verifies stored chunk coordinates,
distinguishes absent chunks from absent Sections, and never infers
unknown modded IDs or silently generates empty blocks.
The iced frontend exposes this through a dimension chooser and
original XYZ coordinate entry. Chunk decompression and palette decoding
are performed only on explicit lookup.

The integrated CI is triggered after the fifth substantive Rust feature
commit. The current ChatGPT container has no Rust compiler and cannot
reach rustup or Debian DNS: only successful CI can verify actual builds.

## Safe nonpersistent world editing (new batch 1/5)

The Rust world core now stages explicit 3D block-state replacements for
original loaded chunks across every supported Java read-only format:
Alpha/early Beta numeric, MCRegion numeric, old numeric Anvil, named
1.13–1.17 Sections, 1.18+ modern Sections, and matching 26.x snapshot
structures. The user can filter by exact original numeric ID+metadata
or modern named properties. No version's unknown modded IDs or unknown
Block Entity NBT are coerced into another version's registry.

Every operation checks original world/Section presence, state layout,
source NBT changes between preview and commit, batch size, and
previously staged state. It then adds one reversible Undo/Redo group to
a per-world, per-dimension **memory overlay**. Empty or unsafe original
Sections are refused, not treated as freshly generated air.

This DOES NOT satisfy persistent edit compatibility. No original file
has been written by this feature. Until a separate original-byte
region writer with backups and rollback passes full differential
testing on historical worlds and recent release/snapshot worlds,
the iced editor must label all these operations UNSAVED.

`-view` remains a strict read-only option. `-eval` still cannot execute
legacy Python scripts in a pure-Rust application.

## Experimental original-byte replacement preparation

The source now includes a read-only/byte-buffer-only first step toward a
lossless native writer for EXISTING named palettes. It is NOT a working
world-save path, cannot grow palettes, and does not yet cover old numeric
format writing, lighting/heightmaps, or region backup/atomic transactions.

### Original-byte named palette growth (source-only checkpoint)

The previous in-memory writer refused palette growth. The new candidate
encoder inserts missing named states into the source palette, supports
single-state palettes without packed data, and re-packs existing block IDs
without resynthesizing unrelated chunk NBT. Full 4096-cell/Section equality
checks are performed after encoding. Region sector/sidecar replacement and
old numeric original-byte writing remain outside this checkpoint.

### Legacy numeric original-byte candidate (round 2/5)

MCRegion and old numeric Anvil now have an in-memory Blocks/Data/Add
byte-level editing candidate, including optional Add growth for Anvil.
This does not provide actual region persistence, lighting/heightmap fixes,
or guarantee compatibility with untested modded save layouts.

### Original Region image construction — round 3/5

The in-memory writer can now propose append-only MCA/MCR region files while
preserving original sector bytes, compression types and untouched header
entries. It only plans edits to preexisting internal chunks. External .mcc
sidecar chunks, unsupported codecs, overlapping sector maps and oversized
internal chunk allocations deliberately fail closed. No on-disk save is
exposed until the next separate transactional writer stage.

### Round 4: experimental atomic one-file writer (not yet tested)

MCA/MCR single-file backing-up, same-directory staged rename, strict
whole-source checksum checking, pending JSON transaction marker,
explicit commit finalization and recovery to exact immutable bytes are
present in the Rust core API. The GUI deliberately cannot save worlds.
No claims are made for multi-region atomicity, external .mcc, Alpha
chunk writing, fully recalculated lighting/heightmaps, or live Minecraft
worlds. Run round-5 CI and tests before any operational use.

### Round 5: guarded experimental desktop Region persistence

Iced supports explicitly confirmed one-Region native disk save and manual
pending-marker finalization/rollback. Both operations preserve immutable
backups, and the editor refuses `-view` writes. This is experimental,
NOT full MCEdit2 editing compatibility: it cannot write Alpha chunk files,
external `.mcc` references, more than one Region atomically, or arbitrary
Block Entities/entities/biomes. Testing is required before live use.

### Native clickable 2D plane (migration batch 2/5)

Signed X/Z and negative-Y slice planes display one clickable cell per
original block (up to 16x16). Clicking selects a corresponding single-block
editing region, while normal Block Entity guards still govern preview/commit.
The full graphical 3D renderer, textured voxel meshes, visual selection
brushes and all-version GPU lighting are still outstanding.
